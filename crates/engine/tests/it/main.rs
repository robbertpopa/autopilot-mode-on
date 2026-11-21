use std::{
    io,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use autopilot_agent::{Agent, AgentError, AgentRun, StepRequest, Stopped};
use autopilot_core::{Event, Money, Ticket, TicketId, Understanding};
use autopilot_engine::{Error, run_understand, watch};

struct Synthetic {
    data_dir: PathBuf,
}

impl Agent for Synthetic {
    fn start(&self, request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        let fails = request.ticket.description == "fail";
        let ticket = request.ticket.id.clone();
        let data_dir = self.data_dir.clone();
        Ok(AgentRun::spawn(move |context| {
            context.message("Read the sample ticket.".into());
            if fails {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    if watch(&data_dir, &ticket, 0)?
                        .iter()
                        .any(|record| matches!(record.event.kind, Event::AgentActivity { .. }))
                    {
                        return Err(io::Error::other("fixture exited unexpectedly").into());
                    }
                    if Instant::now() >= deadline {
                        return Err(
                            io::Error::other("activity was held until the worker exited").into(),
                        );
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
            Ok(Understanding {
                summary: "Implement the sample change.".into(),
                assumptions: vec![],
                needs_design: false,
            })
        })?)
    }
}

#[test]
fn understand_saves_the_result_and_watch_can_resume_from_a_sequence() {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("fixture-1"),
        title: "Sample".into(),
        description: "Add an export option.".into(),
    };

    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    let result = run_understand(data.path(), &ticket, &agent).unwrap();
    let initial = watch(data.path(), &ticket.id, 0).unwrap();
    let through_start = initial
        .iter()
        .find(|event| matches!(event.event.kind, Event::StepStarted { .. }))
        .unwrap()
        .seq;
    let tail = watch(data.path(), &ticket.id, through_start).unwrap();
    assert!(tail.iter().all(|event| event.seq > through_start));
    assert!(tail.iter().any(|event| matches!(&event.event.kind, Event::UnderstandResult { result: stored } if stored == &result)));
    assert!(matches!(tail.last().unwrap().event.kind, Event::Done));
}

#[test]
fn a_failed_understand_run_retains_activity_and_cannot_be_reported_as_done() {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("fixture-2"),
        title: "Failed example".into(),
        description: "fail".into(),
    };

    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    assert!(matches!(
        run_understand(data.path(), &ticket, &agent),
        Err(Error::Agent(error)) if error.to_string() == "fixture exited unexpectedly"
    ));
    let events = watch(data.path(), &ticket.id, 0).unwrap();
    assert!(events.iter().any(|record| matches!(
        &record.event.kind,
        Event::AgentActivity { summary } if summary == "Read the sample ticket."
    )));
    assert!(matches!(
        events.last().unwrap().event.kind,
        Event::Failed { .. }
    ));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event.kind, Event::Done))
    );
}

/// Reports a cost and then stops at a limit, as a capped Claude run does.
struct Capped;

impl Agent for Capped {
    fn start(&self, _: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        Ok(AgentRun::spawn(|context| {
            context.cost(Money::from_microusd(1_500), true);
            Err(Stopped("error_max_turns".into()).into())
        })?)
    }
}

// Invariant: a limit stop is recorded as Interrupted, not Failed or Done, and
// the cost already spent stays in history. Folding it into Failed would make a
// capped step look retryable, and nothing else reads the stored events.
#[test]
fn a_limit_stop_is_recorded_as_interrupted_with_its_cost() {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("fixture-3"),
        title: "Capped".into(),
        description: "any".into(),
    };
    assert!(matches!(
        run_understand(data.path(), &ticket, &Capped),
        Err(Error::Stopped(reason)) if reason == "error_max_turns"
    ));
    let events = watch(data.path(), &ticket.id, 0).unwrap();
    assert!(events.iter().any(|record| matches!(
        record.event.kind,
        Event::CostTick { amount, estimated: true } if amount.as_microusd() == 1_500
    )));
    let kinds: Vec<_> = events.iter().map(|record| &record.event.kind).collect();
    assert!(matches!(kinds.last().unwrap(), Event::Interrupted { .. }));
    assert!(
        !kinds
            .iter()
            .any(|kind| matches!(kind, Event::Failed { .. } | Event::Done))
    );
}
