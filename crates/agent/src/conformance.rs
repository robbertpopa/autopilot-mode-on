//! Behaviour every adapter must show, written once.
//!
//! An adapter's own tests call [`run`] with a [`Subject`] that says how to
//! start the adapter in each [`Scenario`]. Each rule is also callable alone
//! through [`check`], so a failing adapter is told which rule it broke.

use std::{
    fmt, io,
    panic::{self, AssertUnwindSafe},
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use autopilot_core::Ticket;

use crate::{Agent, AgentError, AgentEvent, AgentRun, BuildRequest, SessionUse};

/// How long a run may take before the suite calls it hung. Real adapters
/// finish their canned scenarios in well under a second.
const RUN_LIMIT: Duration = Duration::from_secs(60);

/// What the adapter is made to do for one check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scenario {
    /// A good Understand run that ends in a result.
    Normal,
    /// A good run whose stream also carries event types and fields this code
    /// has never heard of.
    UnknownEvents,
    /// The stream ends before any result arrives.
    NoResult,
    /// The agent works until it is cancelled.
    Hang,
}

/// The rules, one per behaviour the engine relies on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rule {
    /// A good run ends in a result. `AgentRun` stops reading at the first
    /// terminal event, so a second end or anything after one cannot be seen
    /// here and needs no rule.
    OneResult,
    /// The run opens with `Started`, once.
    Order,
    /// A stream that ends without a result is a failure, never a result and
    /// never a silent end.
    MissingResultFails,
    /// A method the adapter does not support refuses with `Unsupported`.
    UnsupportedRefuses,
    /// Event types and fields nobody has heard of change nothing.
    UnknownIgnored,
    /// A cancelled run ends promptly with a terminal event.
    CancelPrompt,
    /// Cost and usage are each reported at most once in a run.
    CostOnce,
}

impl Rule {
    pub const ALL: [Rule; 7] = [
        Rule::OneResult,
        Rule::Order,
        Rule::MissingResultFails,
        Rule::UnsupportedRefuses,
        Rule::UnknownIgnored,
        Rule::CancelPrompt,
        Rule::CostOnce,
    ];
}

/// How to drive one adapter. The suite never reads the adapter's own output
/// format: the adapter's tests decide what each scenario looks like.
pub struct Subject<'a> {
    /// The adapter, for the rule about unsupported methods. It must be an
    /// adapter that cannot run a Build turn.
    pub agent: &'a dyn Agent,
    pub ticket: &'a Ticket,
    /// Start an Understand run in the given scenario.
    pub start: &'a dyn Fn(Scenario) -> Result<AgentRun, AgentError>,
    /// How long a cancelled run may take to end.
    pub cancel_within: Duration,
}

#[derive(Debug)]
pub struct Violation {
    pub rule: Rule,
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.rule, self.detail)
    }
}

/// Run every rule.
///
/// # Errors
/// Returns every rule the adapter broke.
pub fn run(subject: &Subject<'_>) -> Result<(), Vec<Violation>> {
    let broken: Vec<Violation> = Rule::ALL
        .into_iter()
        .filter_map(|rule| {
            check(rule, subject)
                .err()
                .map(|detail| Violation { rule, detail })
        })
        .collect();
    if broken.is_empty() {
        Ok(())
    } else {
        Err(broken)
    }
}

/// Run one rule.
///
/// # Errors
/// Returns what the adapter did wrong.
pub fn check(rule: Rule, subject: &Subject<'_>) -> Result<(), String> {
    match rule {
        Rule::OneResult => one_result(subject),
        Rule::Order => order(subject),
        Rule::MissingResultFails => missing_result_fails(subject),
        Rule::UnsupportedRefuses => unsupported_refuses(subject),
        Rule::UnknownIgnored => unknown_ignored(subject),
        Rule::CancelPrompt => cancel_prompt(subject),
        Rule::CostOnce => cost_once(subject),
    }
}

fn is_terminal(event: &AgentEvent) -> bool {
    !matches!(
        event,
        AgentEvent::Started
            | AgentEvent::Session(_)
            | AgentEvent::Message { .. }
            | AgentEvent::Usage(_)
            | AgentEvent::Cost { .. }
    )
}

fn is_result(event: &AgentEvent) -> bool {
    is_terminal(event) && !matches!(event, AgentEvent::Stopped(_) | AgentEvent::Failed(_))
}

fn name(event: &AgentEvent) -> String {
    match event {
        AgentEvent::Failed(error) => format!("Failed({error})"),
        AgentEvent::Stopped(why) => format!("Stopped({why})"),
        other => {
            let text = format!("{other:?}");
            text.split(['(', ' ', '{']).next().unwrap_or("?").to_owned()
        }
    }
}

fn start(subject: &Subject<'_>, scenario: Scenario) -> Result<AgentRun, String> {
    panic::catch_unwind(AssertUnwindSafe(|| (subject.start)(scenario)))
        .map_err(|_| format!("starting the {scenario:?} run panicked"))?
        .map_err(|error| format!("the {scenario:?} run could not start: {error}"))
}

/// Read a run to its end on another thread, so a run that never ends is
/// reported instead of hanging the suite.
fn events_of(run: AgentRun) -> Result<Vec<AgentEvent>, String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(run.collect::<Vec<_>>());
    });
    receiver
        .recv_timeout(RUN_LIMIT)
        .map_err(|_| format!("the run did not end within {RUN_LIMIT:?}"))
}

fn one_result(subject: &Subject<'_>) -> Result<(), String> {
    let events = events_of(start(subject, Scenario::Normal)?)?;
    match events.last() {
        Some(last) if is_result(last) => Ok(()),
        Some(last) => Err(format!(
            "a good run should end in a result, it ended in {}",
            name(last)
        )),
        None => Err("the run produced no events".into()),
    }
}

fn order(subject: &Subject<'_>) -> Result<(), String> {
    let events = events_of(start(subject, Scenario::Normal)?)?;
    if !matches!(events.first(), Some(AgentEvent::Started)) {
        return Err("the first event is not Started".into());
    }
    if events
        .iter()
        .skip(1)
        .any(|event| matches!(event, AgentEvent::Started))
    {
        return Err("Started was sent more than once".into());
    }
    Ok(())
}

fn missing_result_fails(subject: &Subject<'_>) -> Result<(), String> {
    let events = events_of(start(subject, Scenario::NoResult)?)?;
    match events.last() {
        Some(AgentEvent::Failed(_)) => Ok(()),
        Some(last) => Err(format!(
            "a stream with no result should end in Failed, it ended in {}",
            name(last)
        )),
        None => Err("the run produced no events".into()),
    }
}

fn unsupported_refuses(subject: &Subject<'_>) -> Result<(), String> {
    let request = BuildRequest {
        ticket: subject.ticket,
        worktree: Path::new("/nonexistent-conformance-worktree"),
        prompt: "conformance",
        session: SessionUse::Fresh,
        turns_left: 1,
        budget_left: None,
    };
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| subject.agent.start_build(request)))
        .map_err(|_| "start_build panicked".to_owned())?;
    match outcome {
        Err(error) => match error.downcast_ref::<io::Error>() {
            Some(error) if error.kind() == io::ErrorKind::Unsupported => Ok(()),
            _ => Err(format!(
                "start_build refused with something other than Unsupported: {error}"
            )),
        },
        Ok(_) => Err("start_build started although this agent cannot build".into()),
    }
}

fn unknown_ignored(subject: &Subject<'_>) -> Result<(), String> {
    let events = events_of(start(subject, Scenario::UnknownEvents)?)?;
    match events.last() {
        Some(last) if is_result(last) => Ok(()),
        Some(last) => Err(format!(
            "unknown events and fields changed the outcome: the run ended in {}",
            name(last)
        )),
        None => Err("the run produced no events".into()),
    }
}

fn cancel_prompt(subject: &Subject<'_>) -> Result<(), String> {
    let mut run = start(subject, Scenario::Hang)?;
    let cancel = run.cancellation();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for event in run.by_ref() {
            if sender.send(event).is_err() {
                break;
            }
        }
    });
    match receiver.recv_timeout(RUN_LIMIT) {
        Ok(AgentEvent::Started) => {}
        _ => return Err("the run did not start".into()),
    }
    cancel.cancel();
    let deadline = Instant::now() + subject.cancel_within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(event) if is_terminal(&event) => return Ok(()),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(format!(
                    "the run was still going {:?} after it was cancelled",
                    subject.cancel_within
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("the run ended with no terminal event".into());
            }
        }
    }
}

fn cost_once(subject: &Subject<'_>) -> Result<(), String> {
    let events = events_of(start(subject, Scenario::Normal)?)?;
    let costs = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::Cost { .. }))
        .count();
    let usages = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::Usage(_)))
        .count();
    // One report per run means a figure can never go backwards: the ledger
    // adds a report to what it has, so a second one would be counted twice.
    if costs > 1 || usages > 1 {
        return Err(format!(
            "cost reported {costs} times and usage {usages} times in one run"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicBool};

    use autopilot_core::{Ambiguity, Clarity, Money, UsageReport};

    use super::*;
    use crate::CancelHandle;

    fn ticket() -> Ticket {
        Ticket {
            id: "c-1".into(),
            title: "t".into(),
            description: "d".into(),
        }
    }

    fn understanding() -> autopilot_core::Understanding {
        autopilot_core::Understanding {
            summary: "s".into(),
            ambiguity: Ambiguity::all(Clarity::Clear),
            open_questions: vec![],
            assumptions: vec![],
            needs_design: false,
            ui_files: vec![],
            files_touched: vec![],
        }
    }

    /// A run that replays the given events and then ends, whatever they are.
    /// `AgentRun::spawn` cannot send a bad sequence, so the broken adapters
    /// below build the run by hand.
    fn scripted(events: Vec<AgentEvent>) -> AgentRun {
        let (sender, receiver) = mpsc::channel();
        for event in events {
            let _ = sender.send(event);
        }
        AgentRun {
            events: Some(receiver),
            cancel: CancelHandle(Arc::new(AtomicBool::new(false))),
            worker: None,
        }
    }

    fn good(scenario: Scenario) -> Result<AgentRun, AgentError> {
        match scenario {
            Scenario::Normal | Scenario::UnknownEvents => Ok(AgentRun::spawn(|context| {
                context.message("working".into());
                context.usage(UsageReport::default());
                context.cost(Money::from_microusd(5), false);
                Ok(understanding())
            })?),
            Scenario::NoResult => Ok(AgentRun::spawn(|_| {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no result").into())
            })?),
            Scenario::Hang => Ok(AgentRun::spawn(|context| {
                let give_up = Instant::now() + Duration::from_secs(10);
                while !context
                    .cancel_flag()
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    if Instant::now() >= give_up {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(io::Error::other("cancelled").into())
            })?),
        }
    }

    struct Cannot;
    impl Agent for Cannot {
        fn start(&self, _: crate::StepRequest<'_>) -> Result<AgentRun, AgentError> {
            Err(io::Error::other("unused").into())
        }
    }

    struct Panics;
    impl Agent for Panics {
        fn start(&self, _: crate::StepRequest<'_>) -> Result<AgentRun, AgentError> {
            Err(io::Error::other("unused").into())
        }
        fn start_build(&self, _: BuildRequest<'_>) -> Result<AgentRun, AgentError> {
            panic!("an unsupported method must refuse, not panic");
        }
    }

    struct WrongError;
    impl Agent for WrongError {
        fn start(&self, _: crate::StepRequest<'_>) -> Result<AgentRun, AgentError> {
            Err(io::Error::other("unused").into())
        }
        fn start_build(&self, _: BuildRequest<'_>) -> Result<AgentRun, AgentError> {
            Err(io::Error::other("nope").into())
        }
    }

    fn subject<'a>(
        agent: &'a dyn Agent,
        ticket: &'a Ticket,
        start: &'a dyn Fn(Scenario) -> Result<AgentRun, AgentError>,
    ) -> Subject<'a> {
        Subject {
            agent,
            ticket,
            start,
            cancel_within: Duration::from_millis(500),
        }
    }

    // Invariant: a well-behaved adapter passes every rule. Without it a rule
    // that rejects everything would pass the mutation tests below.
    #[test]
    fn a_well_behaved_adapter_passes_every_rule() {
        let ticket = ticket();
        let start = |s| good(s);
        let result = run(&subject(&Cannot, &ticket, &start));
        assert!(result.is_ok(), "{result:?}");
    }

    // Each test below breaks exactly one behaviour and expects that rule, and
    // only that rule, to fail. A rule that passed a broken adapter would let
    // the real adapters drift without any test noticing.
    fn only_breaks(
        rule: Rule,
        agent: &dyn Agent,
        start: &dyn Fn(Scenario) -> Result<AgentRun, AgentError>,
    ) {
        let ticket = ticket();
        let failed: Vec<Rule> = run(&subject(agent, &ticket, start))
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.rule)
            .collect();
        assert!(failed.contains(&rule), "{rule:?} did not fail: {failed:?}");
    }

    #[test]
    fn a_run_that_never_produces_a_result_breaks_one_result() {
        let start = |s| match s {
            Scenario::Normal => Ok(scripted(vec![
                AgentEvent::Started,
                AgentEvent::Failed(io::Error::other("always fails").into()),
            ])),
            other => good(other),
        };
        only_breaks(Rule::OneResult, &Cannot, &start);
    }

    #[test]
    fn activity_before_started_breaks_order() {
        let start = |s| match s {
            Scenario::Normal => Ok(scripted(vec![
                AgentEvent::Message {
                    summary: "early".into(),
                },
                AgentEvent::Started,
                AgentEvent::Result(understanding()),
            ])),
            other => good(other),
        };
        only_breaks(Rule::Order, &Cannot, &start);
    }

    #[test]
    fn an_empty_stream_with_a_result_breaks_missing_result_fails() {
        let start = |s| match s {
            Scenario::NoResult => Ok(AgentRun::spawn(|_| Ok(understanding()))?),
            other => good(other),
        };
        only_breaks(Rule::MissingResultFails, &Cannot, &start);
    }

    #[test]
    fn a_stream_that_just_ends_breaks_missing_result_fails() {
        let start = |s| match s {
            Scenario::NoResult => Ok(scripted(vec![AgentEvent::Started])),
            other => good(other),
        };
        only_breaks(Rule::MissingResultFails, &Cannot, &start);
    }

    #[test]
    fn panicking_on_an_unsupported_method_breaks_unsupported_refuses() {
        let start = |s| good(s);
        only_breaks(Rule::UnsupportedRefuses, &Panics, &start);
    }

    #[test]
    fn refusing_with_another_error_breaks_unsupported_refuses() {
        let start = |s| good(s);
        only_breaks(Rule::UnsupportedRefuses, &WrongError, &start);
    }

    #[test]
    fn failing_on_unknown_events_breaks_unknown_ignored() {
        let start = |s| match s {
            Scenario::UnknownEvents => Ok(AgentRun::spawn(|_| {
                Err(io::Error::other("unknown event type").into())
            })?),
            other => good(other),
        };
        only_breaks(Rule::UnknownIgnored, &Cannot, &start);
    }

    #[test]
    fn ignoring_cancellation_breaks_cancel_prompt() {
        let start = |s| match s {
            Scenario::Hang => Ok(AgentRun::spawn(|_| {
                thread::sleep(Duration::from_millis(1500));
                Err(io::Error::other("finished anyway").into())
            })?),
            other => good(other),
        };
        only_breaks(Rule::CancelPrompt, &Cannot, &start);
    }

    #[test]
    fn reporting_cost_twice_breaks_cost_once() {
        let start = |s| match s {
            Scenario::Normal => Ok(AgentRun::spawn(|context| {
                context.cost(Money::from_microusd(5), false);
                context.cost(Money::from_microusd(3), false);
                Ok(understanding())
            })?),
            other => good(other),
        };
        only_breaks(Rule::CostOnce, &Cannot, &start);
    }

    #[test]
    fn reporting_usage_twice_breaks_cost_once() {
        let start = |s| match s {
            Scenario::Normal => Ok(AgentRun::spawn(|context| {
                context.usage(UsageReport::default());
                context.usage(UsageReport::default());
                Ok(understanding())
            })?),
            other => good(other),
        };
        only_breaks(Rule::CostOnce, &Cannot, &start);
    }
}
