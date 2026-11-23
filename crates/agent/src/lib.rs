//! Provider-independent agent execution and events.

use std::{
    io,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use std::path::Path;

use autopilot_core::{Design, Money, Plan, SessionId, Step, Ticket, Understanding};

pub type AgentError = Box<dyn std::error::Error + Send + Sync>;

/// A run that ended at a configured limit. The agent worked as asked, so this
/// is not a failure and must not be retried as one.
#[derive(Debug, thiserror::Error)]
#[error("agent stopped: {0}")]
pub struct Stopped(pub String);

pub struct StepRequest<'a> {
    pub step: Step,
    pub ticket: &'a Ticket,
    /// What earlier steps established, as text for the prompt. Empty for Understand.
    pub context: &'a str,
}

/// A request for a pipeline draft. The description is the owner's words, which
/// are data for the agent, never instructions to the engine.
pub struct DraftRequest<'a> {
    pub description: &'a str,
    /// The pipelines that already exist, so the draft can start from one.
    pub known: &'a [String],
    /// What the owner is changing, in plain text: the pipeline being edited
    /// with its steps, and the custom steps already defined. Empty for a new
    /// pipeline. It is data for the agent, like the description.
    pub current: &'a str,
}

/// One custom step: the owner's own prompt, run on what the ticket has so far.
pub struct CustomRequest<'a> {
    pub step: Step,
    pub ticket: &'a Ticket,
    /// The name the owner gave the step.
    pub name: &'a str,
    /// What the owner wants done. It is the owner's instruction, unlike the
    /// ticket and the context, which are data.
    pub prompt: &'a str,
    /// What the earlier steps established, as text.
    pub context: &'a str,
}

/// One build turn in a ticket's worktree.
pub struct BuildRequest<'a> {
    pub ticket: &'a Ticket,
    /// Directory the agent works in. Absolute.
    pub worktree: &'a Path,
    /// The prompt: the approved plan on a first turn, a failure note on a
    /// refine turn, or a summary of what failed for a fresh session.
    pub prompt: &'a str,
    pub session: SessionUse,
    /// What is left of the step's caps. The adapter passes them to the
    /// provider as that call's own limits.
    pub turns_left: u32,
    pub budget_left: Option<Money>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionUse {
    /// Start a new provider session. The adapter reports its opaque reference.
    Fresh,
    /// Continue the session behind this reference.
    Resume(SessionId),
}

#[derive(Debug)]
pub enum AgentEvent {
    Started,
    /// The opaque local reference of the session this run uses. The provider's
    /// own identifier never leaves the adapter.
    Session(SessionId),
    Message {
        summary: String,
    },
    /// What the run used, when the provider says. Adapters that report
    /// nothing never send this, so a missing figure stays missing.
    Usage(autopilot_core::UsageReport),
    /// Cost reported for the run. `estimated` is true when the provider's own
    /// figure is not a bill, as with a subscription.
    Cost {
        amount: Money,
        estimated: bool,
    },
    Result(Understanding),
    PlanResult(Plan),
    /// The raw reply to a pipeline draft request. It is untrusted text: the
    /// engine parses and checks it, so nothing is dropped before it is seen.
    DraftResult(String),
    DesignResult(Design),
    CustomResult(autopilot_core::CustomOutcome),
    /// A build turn ended normally. Its effect is in the worktree.
    BuildDone,
    Stopped(String),
    Failed(AgentError),
}

pub trait Agent {
    /// # Errors
    /// Returns an error if the request cannot be started. A running step
    /// reports failure through its event stream.
    fn start(&self, request: StepRequest<'_>) -> Result<AgentRun, AgentError>;

    /// Run one build turn. Adapters that cannot build keep this default.
    ///
    /// # Errors
    /// Returns an error if the request cannot be started or is unsupported.
    fn start_build(&self, _request: BuildRequest<'_>) -> Result<AgentRun, AgentError> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this agent cannot run the Build step",
        )
        .into())
    }

    /// Start the Design step. Adapters that cannot design keep this default.
    ///
    /// # Errors
    /// Returns an error if the request cannot be started or is unsupported.
    fn start_design(&self, _request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this agent cannot run the Design step",
        )
        .into())
    }

    /// Ask for a pipeline draft from the owner's description. The reply is raw
    /// text for the engine to validate. Adapters that cannot draft keep this
    /// default, which refuses.
    ///
    /// # Errors
    /// Returns an error if the request cannot be started or is unsupported.
    fn start_draft(&self, _request: DraftRequest<'_>) -> Result<AgentRun, AgentError> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this agent cannot draft a pipeline",
        )
        .into())
    }

    /// Start a custom step. Adapters that cannot keep this default.
    ///
    /// # Errors
    /// Returns an error if the request cannot be started or is unsupported.
    fn start_custom(&self, _request: CustomRequest<'_>) -> Result<AgentRun, AgentError> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this agent cannot run a custom step",
        )
        .into())
    }

    /// Start the Plan step. Adapters that cannot plan keep this default.
    ///
    /// # Errors
    /// Returns an error if the request cannot be started or is unsupported.
    fn start_plan(&self, _request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this agent cannot run the Plan step",
        )
        .into())
    }
}

#[derive(Clone)]
pub struct AgentContext {
    events: Sender<AgentEvent>,
    cancel: CancelHandle,
}

impl AgentContext {
    #[must_use]
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel.0
    }

    /// Report a bounded, safe-to-display activity summary.
    pub fn message(&self, summary: String) {
        let _ = self.events.send(AgentEvent::Message { summary });
    }

    /// Report the opaque reference of the session this run uses.
    pub fn session(&self, reference: SessionId) {
        let _ = self.events.send(AgentEvent::Session(reference));
    }

    pub fn usage(&self, usage: autopilot_core::UsageReport) {
        let _ = self.events.send(AgentEvent::Usage(usage));
    }

    pub fn cost(&self, amount: Money, estimated: bool) {
        let _ = self.events.send(AgentEvent::Cost { amount, estimated });
    }
}

/// A cloneable cancellation handle usable while another thread reads events.
#[derive(Clone)]
pub struct CancelHandle(Arc<AtomicBool>);

impl CancelHandle {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// A blocking iterator over agent events. Dropping it cancels and waits for
/// the worker so the caller can release other resources after cleanup.
pub struct AgentRun {
    events: Option<Receiver<AgentEvent>>,
    cancel: CancelHandle,
    worker: Option<JoinHandle<()>>,
}

impl AgentRun {
    /// Start a worker and publish its terminal result after it finishes.
    /// The worker must observe cancellation promptly and bound activity output.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn spawn(
        work: impl FnOnce(AgentContext) -> Result<Understanding, AgentError> + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(work, AgentEvent::Result)
    }

    /// Like [`AgentRun::spawn`] for a Plan result.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn spawn_plan(
        work: impl FnOnce(AgentContext) -> Result<Plan, AgentError> + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(work, AgentEvent::PlanResult)
    }

    /// Like [`AgentRun::spawn`] for a Design result.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn spawn_design(
        work: impl FnOnce(AgentContext) -> Result<Design, AgentError> + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(work, AgentEvent::DesignResult)
    }

    /// Like [`AgentRun::spawn`] for a custom step's outcome.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn spawn_custom(
        work: impl FnOnce(AgentContext) -> Result<autopilot_core::CustomOutcome, AgentError>
        + Send
        + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(work, AgentEvent::CustomResult)
    }

    /// Like [`AgentRun::spawn`] for a pipeline draft, whose result is raw text.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn spawn_draft(
        work: impl FnOnce(AgentContext) -> Result<String, AgentError> + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(work, AgentEvent::DraftResult)
    }

    /// Like [`AgentRun::spawn`] for a build turn, which has no structured result.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn spawn_build(
        work: impl FnOnce(AgentContext) -> Result<(), AgentError> + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(work, |()| AgentEvent::BuildDone)
    }

    fn spawn_with<T: Send + 'static>(
        work: impl FnOnce(AgentContext) -> Result<T, AgentError> + Send + 'static,
        wrap: fn(T) -> AgentEvent,
    ) -> io::Result<Self> {
        let (sender, events) = mpsc::channel();
        let cancel = CancelHandle(Arc::new(AtomicBool::new(false)));
        let context = AgentContext {
            events: sender.clone(),
            cancel: cancel.clone(),
        };
        let worker = thread::Builder::new().spawn(move || {
            if sender.send(AgentEvent::Started).is_err() {
                return;
            }
            // The stream must terminate even if another thread retains a context.
            let terminal = match panic::catch_unwind(AssertUnwindSafe(|| work(context))) {
                Ok(Ok(result)) => wrap(result),
                Ok(Err(error)) => match error.downcast::<Stopped>() {
                    Ok(stopped) => AgentEvent::Stopped(stopped.0),
                    Err(error) => AgentEvent::Failed(error),
                },
                Err(_) => AgentEvent::Failed(io::Error::other("agent worker panicked").into()),
            };
            let _ = sender.send(terminal);
        })?;
        Ok(Self {
            events: Some(events),
            cancel,
            worker: Some(worker),
        })
    }

    #[must_use]
    pub fn cancellation(&self) -> CancelHandle {
        self.cancel.clone()
    }
}

impl Iterator for AgentRun {
    type Item = AgentEvent;

    fn next(&mut self) -> Option<Self::Item> {
        let event = self.events.as_ref()?.recv().ok();
        if !matches!(
            event,
            Some(
                AgentEvent::Started
                    | AgentEvent::Message { .. }
                    | AgentEvent::Session(_)
                    | AgentEvent::Cost { .. }
                    | AgentEvent::Usage(_)
            )
        ) {
            self.events = None;
        }
        event
    }
}

impl Drop for AgentRun {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.events = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_ends_at_the_terminal_event_even_if_a_context_remains() {
        let (retain, retained) = mpsc::channel();
        let mut run = AgentRun::spawn(move |context| {
            retain.send(context).unwrap();
            Err(io::Error::other("synthetic failure").into())
        })
        .unwrap();

        let context = retained.recv().unwrap();
        assert!(matches!(run.next(), Some(AgentEvent::Started)));
        assert!(matches!(run.next(), Some(AgentEvent::Failed(_))));
        context.message("late report".into());
        assert!(run.next().is_none());
    }

    #[test]
    fn dropping_the_run_waits_for_the_worker_to_observe_cancellation() {
        use std::time::{Duration, Instant};

        let (cleaned_up, cleaned) = mpsc::channel();
        let mut run = AgentRun::spawn(move |context| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !context.cancel_flag().load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    return Err(io::Error::other("cancellation never arrived").into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            cleaned_up.send(()).unwrap();
            Err(io::Error::other("cancelled").into())
        })
        .unwrap();
        assert!(matches!(run.next(), Some(AgentEvent::Started)));
        drop(run);
        assert_eq!(cleaned.try_recv(), Ok(()));
    }

    // Invariant: a usage report is an in-run event like cost, so the result
    // that follows it is still delivered. Without it the iterator stops after
    // the report and the step ends with "stream ended without a result".
    #[test]
    fn a_usage_report_does_not_end_the_stream() {
        let mut run = AgentRun::spawn(|context| {
            context.usage(autopilot_core::UsageReport::default());
            context.cost(Money::from_microusd(5), false);
            Ok(autopilot_core::Understanding {
                summary: "s".into(),
                ambiguity: autopilot_core::Ambiguity::all(autopilot_core::Clarity::Clear),
                open_questions: vec![],
                assumptions: vec![],
                needs_design: false,
                ui_files: vec![],
                files_touched: vec![],
            })
        })
        .unwrap();
        assert!(matches!(run.next(), Some(AgentEvent::Started)));
        assert!(matches!(run.next(), Some(AgentEvent::Usage(_))));
        assert!(matches!(run.next(), Some(AgentEvent::Cost { .. })));
        assert!(matches!(run.next(), Some(AgentEvent::Result(_))));
    }
}
