use autopilot_agent::{Agent, AgentError, UnderstandRun};
use autopilot_core::{Event, Ticket, TicketId, Understanding};
use autopilot_engine::{run_understand, watch};

struct Synthetic;

impl Agent for Synthetic {
    fn understand(&self, ticket: &Ticket) -> Result<UnderstandRun, AgentError> {
        if ticket.description == "fail" {
            return Err(std::io::Error::other("fixture exited unexpectedly").into());
        }
        Ok(UnderstandRun {
            activity: vec!["Read the sample ticket.".into()],
            result: Understanding {
                summary: "Implement the sample change.".into(),
                assumptions: vec![],
                needs_design: false,
            },
        })
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

    let result = run_understand(data.path(), &ticket, &Synthetic).unwrap();
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
fn a_failed_understand_run_is_durable_and_cannot_be_reported_as_done() {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("fixture-2"),
        title: "Failed example".into(),
        description: "fail".into(),
    };

    assert!(run_understand(data.path(), &ticket, &Synthetic).is_err());
    let events = watch(data.path(), &ticket.id, 0).unwrap();
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
