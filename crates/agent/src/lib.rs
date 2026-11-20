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

use autopilot_core::{Step, Ticket, Understanding};

pub type AgentError = Box<dyn std::error::Error + Send + Sync>;

pub struct StepRequest<'a> {
    pub step: Step,
    pub ticket: &'a Ticket,
}

#[derive(Debug)]
pub enum AgentEvent {
    Started,
    Message { summary: String },
    Result(Understanding),
    Failed(AgentError),
}

pub trait Agent {
    /// # Errors
    /// Returns an error if the request cannot be started. A running step
    /// reports failure through its event stream.
    fn start(&self, request: StepRequest<'_>) -> Result<AgentRun, AgentError>;
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
                Ok(Ok(result)) => AgentEvent::Result(result),
                Ok(Err(error)) => AgentEvent::Failed(error),
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
            Some(AgentEvent::Started | AgentEvent::Message { .. })
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
}
