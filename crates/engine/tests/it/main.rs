use std::{
    io,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use autopilot_agent::{Agent, AgentError, AgentRun, StepRequest, Stopped};
use autopilot_core::{
    Ambiguity, Clarity, Event, Money, OpenQuestion, Ticket, TicketId, Understanding,
};
use autopilot_engine::{
    ClarifyStart, Error, answer_clarify, run_understand, start_clarify, sync_inbox, watch,
};
use autopilot_pipeline::{
    clarify::Choice,
    inbox::{Decision, Inbox, Reply},
};

struct Synthetic {
    data_dir: PathBuf,
}

impl Agent for Synthetic {
    fn start(&self, request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        let fails = request.ticket.description == "fail";
        let asks = request.ticket.description == "ask";
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
                ambiguity: Ambiguity::all(Clarity::Clear),
                open_questions: if asks {
                    vec![OpenQuestion {
                        question: "Which delimiter?".into(),
                        options: vec!["Comma".into(), "Semicolon".into()],
                        recommended: 1,
                    }]
                } else {
                    vec![]
                },
                assumptions: vec![],
                needs_design: false,
                ui_files: vec![],
                files_touched: vec![],
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
    assert!(matches!(
        tail.last().unwrap().event.kind,
        Event::StepFinished
    ));
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

fn understood(
    description: &str,
) -> Result<(tempfile::TempDir, TicketId), Box<dyn std::error::Error>> {
    let data = tempfile::tempdir()?;
    let ticket = Ticket {
        id: TicketId::from("clarify-1"),
        title: "Export".into(),
        description: description.into(),
    };
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    run_understand(data.path(), &ticket, &agent)?;
    Ok((data, ticket.id))
}

// Invariant: a ticket with nothing to ask skips Clarify with the recorded
// reason and does not stall waiting for an answer nobody can give.
#[test]
fn clarify_is_skipped_when_there_are_no_open_questions() {
    let (data, ticket) = understood("plain").unwrap();
    assert_eq!(
        start_clarify(data.path(), &ticket).unwrap(),
        ClarifyStart::Skipped("no open questions")
    );
    let last = watch(data.path(), &ticket, 0).unwrap().pop().unwrap();
    assert!(matches!(
        last.event.kind,
        Event::StepSkipped { ref reason } if reason == "no open questions"
    ));
    assert!(matches!(
        start_clarify(data.path(), &ticket),
        Err(Error::NotReady(_))
    ));
}

// Invariant: the question survives a restart (nothing is held in memory), and
// accepting the recommendation records exactly one Q -> A pair, once.
#[test]
fn clarify_asks_then_records_the_recommended_answer_once() {
    let (data, ticket) = understood("ask").unwrap();
    let ClarifyStart::Asking { request, questions } = start_clarify(data.path(), &ticket).unwrap()
    else {
        panic!("expected a question");
    };
    assert_eq!(questions.len(), 1);
    assert!(matches!(
        answer_clarify(data.path(), &ticket, &request, &[]),
        Err(Error::Answer(_))
    ));
    let pairs = answer_clarify(data.path(), &ticket, &request, &[Choice::Recommended]).unwrap();
    assert_eq!(pairs[0].answer, "Semicolon");
    let history = watch(data.path(), &ticket, 0).unwrap();
    assert!(history.iter().any(|record| matches!(
        &record.event.kind,
        Event::Answered { answer, .. } if answer.contains("Semicolon") && answer.contains("Which delimiter?")
    )));
    assert!(matches!(
        answer_clarify(data.path(), &ticket, &request, &[Choice::Recommended]),
        Err(Error::NotReady(_))
    ));
}

fn answered_count(data: &std::path::Path, ticket: &TicketId) -> usize {
    let history = watch(data, ticket, 0).unwrap_or_default();
    history
        .iter()
        .filter(|record| matches!(record.event.kind, Event::Answered { .. }))
        .count()
}

// Invariant: request written, process lost, answer file dropped in later:
// the answer is applied exactly once, even if the files outlive the commit.
// A double apply would record two answers or fail the step machine.
#[test]
fn an_answer_file_is_applied_exactly_once_across_restarts() {
    let (data, ticket) = understood("ask").unwrap();
    let ClarifyStart::Asking { request, .. } = start_clarify(data.path(), &ticket).unwrap() else {
        panic!("expected a question");
    };
    let inbox = Inbox::in_data_dir(data.path());
    assert_eq!(inbox.requests().unwrap().len(), 1);

    // Nothing answered yet: syncing changes nothing.
    assert_eq!(sync_inbox(data.path(), &ticket).unwrap(), vec![]);
    // The request file was lost in a crash: syncing mirrors it again.
    inbox.clean_up(&request).unwrap();
    assert_eq!(sync_inbox(data.path(), &ticket).unwrap(), vec![]);
    assert_eq!(inbox.requests().unwrap().len(), 1);

    let reply = Reply::Question {
        choices: vec![Choice::Option(0)],
    };
    inbox.write_answer(&request, &reply).unwrap();
    assert_eq!(
        sync_inbox(data.path(), &ticket).unwrap(),
        vec![request.clone()]
    );
    assert_eq!(answered_count(data.path(), &ticket), 1);
    assert_eq!(inbox.requests().unwrap(), vec![]);

    // Crash after the commit but before cleanup: the files come back stale.
    inbox.write_answer(&request, &reply).unwrap();
    assert_eq!(sync_inbox(data.path(), &ticket).unwrap(), vec![]);
    assert_eq!(answered_count(data.path(), &ticket), 1);
    assert!(inbox.read_answer(&request).unwrap().is_none());
}

// Invariant: a wrong-kind or malformed answer is refused and leaves the
// question waiting, so the owner can replace the file.
#[test]
fn a_bad_answer_file_leaves_the_question_waiting() {
    let (data, ticket) = understood("ask").unwrap();
    let ClarifyStart::Asking { request, .. } = start_clarify(data.path(), &ticket).unwrap() else {
        panic!("expected a question");
    };
    let inbox = Inbox::in_data_dir(data.path());
    let approve = Reply::Approval {
        decision: Decision::Approve,
        allow_for_project: false,
    };
    inbox.write_answer(&request, &approve).unwrap();
    assert!(sync_inbox(data.path(), &ticket).is_err());
    let out_of_range = Reply::Question {
        choices: vec![Choice::Option(9)],
    };
    inbox.write_answer(&request, &out_of_range).unwrap();
    assert!(sync_inbox(data.path(), &ticket).is_err());
    assert_eq!(answered_count(data.path(), &ticket), 0);
    inbox
        .write_answer(
            &request,
            &Reply::Question {
                choices: vec![Choice::Recommended],
            },
        )
        .unwrap();
    assert_eq!(sync_inbox(data.path(), &ticket).unwrap().len(), 1);
}
