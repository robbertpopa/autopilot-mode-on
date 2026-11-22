#![allow(clippy::unwrap_used)] // helpers outside #[test] functions
use std::{
    io,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

mod attachments;
#[cfg(windows)]
mod build;

use autopilot_agent::{Agent, AgentError, AgentRun, StepRequest, Stopped};
use autopilot_core::{
    Ambiguity, Clarity, Event, Money, OpenQuestion, Ticket, TicketId, Understanding,
};
use autopilot_engine::{
    ClarifyStart, Error, answer_clarify, approved_plan, effective_overrides, override_settings,
    recover, rerun_understand, reset_step, run_plan, run_understand, run_understand_with,
    start_clarify, sync_inbox, watch,
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

fn understood_and_asking(description: &str) -> (tempfile::TempDir, TicketId) {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("approve-1"),
        title: "Export".into(),
        description: description.into(),
    };
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    run_understand_with(data.path(), &ticket, &agent, Some(true)).unwrap();
    (data, ticket.id)
}

fn last_kind(data: &std::path::Path, ticket: &TicketId) -> Event {
    watch(data, ticket, 0).unwrap().pop().unwrap().event.kind
}

// Invariant: with "Ask me after this step" the step does not pass until the
// owner answers, and the request survives in the inbox meanwhile.
#[test]
fn ask_after_holds_the_step_until_the_owner_approves() {
    let (data, ticket) = understood_and_asking("plain");
    assert!(matches!(
        last_kind(data.path(), &ticket),
        Event::Asked { .. }
    ));
    let inbox = Inbox::in_data_dir(data.path());
    let waiting = inbox.requests().unwrap();
    assert_eq!(waiting.len(), 1);
    // Clarify cannot start while Understand waits for approval.
    assert!(matches!(
        start_clarify(data.path(), &ticket),
        Err(Error::NotReady(_))
    ));
    let approve = Reply::Approval {
        decision: Decision::Approve,
        allow_for_project: false,
    };
    inbox.write_answer(&waiting[0].id, &approve).unwrap();
    assert_eq!(sync_inbox(data.path(), &ticket).unwrap().len(), 1);
    assert!(matches!(
        last_kind(data.path(), &ticket),
        Event::StepFinished
    ));
    assert!(matches!(
        start_clarify(data.path(), &ticket),
        Ok(ClarifyStart::Skipped(_))
    ));
}

// Invariant: reject stops the step as Interrupted (so it can run again), and
// an edit passes the step while keeping the owner's text in the log.
#[test]
fn reject_interrupts_and_edit_keeps_the_text() {
    let (data, ticket) = understood_and_asking("plain");
    let inbox = Inbox::in_data_dir(data.path());
    let id = inbox.requests().unwrap().remove(0).id;
    let reject = Reply::Approval {
        decision: Decision::Reject,
        allow_for_project: false,
    };
    inbox.write_answer(&id, &reject).unwrap();
    sync_inbox(data.path(), &ticket).unwrap();
    assert!(matches!(
        last_kind(data.path(), &ticket),
        Event::Interrupted { ref reason } if reason == "rejected by the owner"
    ));

    let (data, ticket) = understood_and_asking("plain");
    let inbox = Inbox::in_data_dir(data.path());
    let id = inbox.requests().unwrap().remove(0).id;
    let edit = Reply::Approval {
        decision: Decision::Edit("use semicolons".into()),
        allow_for_project: false,
    };
    inbox.write_answer(&id, &edit).unwrap();
    sync_inbox(data.path(), &ticket).unwrap();
    let history = watch(data.path(), &ticket, 0).unwrap();
    assert!(history.iter().any(|record| matches!(
        &record.event.kind,
        Event::Answered { answer, .. } if answer.contains("use semicolons")
    )));
    assert!(matches!(
        history.last().unwrap().event.kind,
        Event::StepFinished
    ));
}

// Invariant: if the request file is lost, it comes back as a forced request,
// so a standing allowance can never be granted from a rebuilt request.
#[test]
fn a_lost_approval_request_returns_as_forced() {
    let (data, ticket) = understood_and_asking("plain");
    let inbox = Inbox::in_data_dir(data.path());
    let id = inbox.requests().unwrap().remove(0).id;
    inbox.clean_up(&id).unwrap();
    sync_inbox(data.path(), &ticket).unwrap();
    let standing = Reply::Approval {
        decision: Decision::Approve,
        allow_for_project: true,
    };
    inbox.write_answer(&id, &standing).unwrap();
    assert!(sync_inbox(data.path(), &ticket).is_err());
    assert_eq!(answered_count(data.path(), &ticket), 0);
}

// Invariant: an attempt whose process died is shown as Interrupted and can be
// started again; a finished or waiting step is never touched. Without this a
// killed run would read as "running" forever.
#[test]
fn recover_interrupts_only_attempts_that_were_cut_off() {
    use autopilot_core::{AttemptId, NewEvent, Step};
    use autopilot_store::{SqliteStore, Store};
    let data = tempfile::tempdir().unwrap();
    let ticket = TicketId::from("killed-1");
    let at = |step, kind| NewEvent {
        at_ms: 1,
        step,
        kind,
    };
    {
        let mut store = SqliteStore::open(data.path().join("events.sqlite3")).unwrap();
        store
            .append(
                &ticket,
                0,
                &[
                    at(None, Event::TicketCreated { title: "t".into() }),
                    at(
                        Some(Step::Understand),
                        Event::StepStarted {
                            attempt: AttemptId::from("understand-1"),
                            session: None,
                        },
                    ),
                ],
            )
            .unwrap();
    }
    assert_eq!(
        recover(data.path(), &ticket).unwrap(),
        vec![Step::Understand]
    );
    assert!(matches!(
        last_kind(data.path(), &ticket),
        Event::Interrupted { .. }
    ));
    assert_eq!(recover(data.path(), &ticket).unwrap(), vec![]);

    let (data, ticket) = understood_and_asking("plain");
    assert_eq!(recover(data.path(), &ticket).unwrap(), vec![]);
}

fn ask_after_change(value: bool) -> Vec<autopilot_core::StepChange> {
    vec![autopilot_core::StepChange {
        step: autopilot_core::Step::Understand,
        change: autopilot_core::StepOverride {
            ask_after: Some(value),
            ..Default::default()
        },
    }]
}

// Invariant: a project-scope override reaches a ticket that starts later, an
// explicit choice beats it, and a ticket-scope change wins over the project
// one for that ticket only. Otherwise the owner's "always ask" would silently
// not apply to new tickets.
#[test]
fn overrides_layer_project_then_ticket_and_apply_at_the_next_start() {
    use autopilot_core::SettingsScope;
    let data = tempfile::tempdir().unwrap();
    let first = understood_with(data.path(), "first", None);
    override_settings(
        data.path(),
        &first,
        SettingsScope::Project,
        &ask_after_change(true),
    )
    .unwrap();
    // The finished ticket was not changed retroactively.
    assert!(matches!(
        last_kind(data.path(), &first),
        Event::SettingsOverridden { .. }
    ));
    let second = understood_with(data.path(), "second", None);
    assert!(matches!(
        last_kind(data.path(), &second),
        Event::Asked { .. }
    ));
    let third = understood_with(data.path(), "third", Some(false));
    assert!(matches!(
        last_kind(data.path(), &third),
        Event::StepFinished
    ));

    override_settings(
        data.path(),
        &third,
        SettingsScope::Ticket,
        &ask_after_change(false),
    )
    .unwrap();
    let step = autopilot_core::Step::Understand;
    assert_eq!(
        effective_overrides(data.path(), &second, step)
            .unwrap()
            .ask_after,
        Some(true)
    );
    assert_eq!(
        effective_overrides(data.path(), &third, step)
            .unwrap()
            .ask_after,
        Some(false)
    );
}

fn understood_with(data: &std::path::Path, id: &str, ask: Option<bool>) -> TicketId {
    let ticket = Ticket {
        id: TicketId::from(id),
        title: "t".into(),
        description: "plain".into(),
    };
    let agent = Synthetic {
        data_dir: data.into(),
    };
    run_understand_with(data, &ticket, &agent, ask).unwrap();
    ticket.id
}

struct Planner(autopilot_core::Plan);

impl Agent for Planner {
    fn start(&self, _request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        Err(io::Error::other("planner only plans").into())
    }

    fn start_plan(&self, request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        assert!(request.context.contains("Summary:"));
        let plan = self.0.clone();
        Ok(AgentRun::spawn_plan(move |context| {
            context.message("Drafting the plan.".into());
            Ok(plan)
        })?)
    }
}

fn criterion(
    id: &str,
    verify: autopilot_core::Verify,
    verification: &str,
) -> autopilot_core::Criterion {
    autopilot_core::Criterion {
        id: id.into(),
        text: "Row count equals ticket count".into(),
        verify,
        verification_id: verification.into(),
    }
}

fn good_plan() -> autopilot_core::Plan {
    autopilot_core::Plan {
        criteria: vec![criterion("c1", autopilot_core::Verify::Test, "row_count")],
        test_plan: vec!["run row_count".into()],
        files: vec!["src/export.rs".into()],
    }
}

fn ticket_for(id: &str) -> Ticket {
    Ticket {
        id: TicketId::from(id),
        title: "Export".into(),
        description: "plain".into(),
    }
}

/// A ticket with Understand passed and Clarify skipped, ready to plan.
fn ready_to_plan(id: &str) -> (tempfile::TempDir, Ticket) {
    let data = tempfile::tempdir().unwrap();
    let ticket = ticket_for(id);
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    run_understand(data.path(), &ticket, &agent).unwrap();
    start_clarify(data.path(), &ticket.id).unwrap();
    (data, ticket)
}

fn approve(
    data: &std::path::Path,
    ticket: &TicketId,
    allow_for_project: bool,
) -> Result<usize, Error> {
    let inbox = Inbox::in_data_dir(data);
    let id = inbox.requests().unwrap().remove(0).id;
    let reply = Reply::Approval {
        decision: Decision::Approve,
        allow_for_project,
    };
    inbox.write_answer(&id, &reply).unwrap();
    sync_inbox(data, ticket).map(|applied| applied.len())
}

// Invariant: the plan waits for approval; after approval the stored copy is
// the one the log vouches for. Without the wait Build could start on a plan
// nobody read.
#[test]
fn a_plan_is_approved_stored_and_read_back_with_its_hash() {
    let (data, ticket) = ready_to_plan("plan-1");
    assert!(matches!(
        approved_plan(data.path(), &ticket.id),
        Err(Error::NotReady(_))
    ));
    run_plan(data.path(), &ticket, &Planner(good_plan())).unwrap();
    assert!(matches!(
        last_kind(data.path(), &ticket.id),
        Event::Asked { .. }
    ));
    assert!(matches!(
        approved_plan(data.path(), &ticket.id),
        Err(Error::NotReady(_))
    ));
    assert_eq!(approve(data.path(), &ticket.id, false).unwrap(), 1);
    assert!(matches!(
        last_kind(data.path(), &ticket.id),
        Event::StepFinished
    ));
    assert_eq!(approved_plan(data.path(), &ticket.id).unwrap(), good_plan());
}

// Invariant: an agent that rewrites the stored plan after approval is caught,
// so it cannot loosen its own acceptance criteria.
#[test]
fn an_altered_approved_plan_is_refused() {
    let (data, ticket) = ready_to_plan("plan-2");
    run_plan(data.path(), &ticket, &Planner(good_plan())).unwrap();
    approve(data.path(), &ticket.id, false).unwrap();
    let path = data.path().join("approved-plans").join("plan-2.json");
    let mut edited = good_plan();
    edited.criteria[0].text = "Anything goes".into();
    std::fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();
    assert!(matches!(
        approved_plan(data.path(), &ticket.id),
        Err(Error::History(_))
    ));
    std::fs::remove_file(&path).unwrap();
    assert!(approved_plan(data.path(), &ticket.id).is_err());
}

// Invariant: a plan that cannot be checked as written asks as a forced
// confirmation, so approving it cannot become a standing allowance.
#[test]
fn a_plan_with_an_unverifiable_criterion_is_a_forced_confirmation() {
    let (data, ticket) = ready_to_plan("plan-3");
    let mut plan = good_plan();
    plan.criteria
        .push(criterion("c2", autopilot_core::Verify::Unknown, ""));
    run_plan(data.path(), &ticket, &Planner(plan)).unwrap();
    let request = Inbox::in_data_dir(data.path())
        .requests()
        .unwrap()
        .remove(0);
    assert!(matches!(
        request.body,
        autopilot_pipeline::inbox::RequestBody::Approval { forced: true, .. }
    ));
    assert!(matches!(
        approve(data.path(), &ticket.id, true),
        Err(Error::Inbox(_))
    ));
    assert_eq!(approve(data.path(), &ticket.id, false).unwrap(), 1);
}

// Invariant: an invalid plan fails the step, and the step can be tried again.
#[test]
fn an_invalid_plan_fails_the_step_and_it_can_run_again() {
    let (data, ticket) = ready_to_plan("plan-4");
    let mut bad = good_plan();
    bad.criteria.clear();
    assert!(run_plan(data.path(), &ticket, &Planner(bad)).is_err());
    assert!(matches!(
        last_kind(data.path(), &ticket.id),
        Event::Failed { .. }
    ));
    run_plan(data.path(), &ticket, &Planner(good_plan())).unwrap();
    assert!(matches!(
        last_kind(data.path(), &ticket.id),
        Event::Asked { .. }
    ));
}

#[test]
fn planning_needs_understand_and_clarify_to_be_settled() {
    let data = tempfile::tempdir().unwrap();
    let ticket = ticket_for("plan-5");
    assert!(run_plan(data.path(), &ticket, &Planner(good_plan())).is_err());
    let (data, ticket) = understood_and_asking("ask");
    let ticket = ticket_for(ticket.as_str());
    assert!(matches!(
        run_plan(data.path(), &ticket, &Planner(good_plan())),
        Err(Error::NotReady(_))
    ));
}

struct Remembers(std::sync::Mutex<Vec<String>>);

impl Agent for Remembers {
    fn start(&self, request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        self.0
            .lock()
            .unwrap()
            .push(request.ticket.description.clone());
        Ok(AgentRun::spawn(move |_| {
            Ok(Understanding {
                summary: "Again.".into(),
                ambiguity: Ambiguity::all(Clarity::Clear),
                open_questions: vec![],
                assumptions: vec![],
                needs_design: false,
                ui_files: vec![],
                files_touched: vec![],
            })
        })?)
    }
}

// Invariant: running Understand again reuses the words the owner first wrote,
// keeps the old attempt in the history, and the step ends passed again. A
// change that dropped the saved words, or that rewrote the first attempt, would
// make a re-run answer a different question than the one that was asked.
#[test]
fn understand_runs_again_with_the_original_words_and_keeps_the_old_attempt() {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("again-1"),
        title: "Export".into(),
        description: "Add an export option.".into(),
    };
    let agent = Remembers(std::sync::Mutex::new(Vec::new()));
    run_understand(data.path(), &ticket, &agent).unwrap();
    rerun_understand(data.path(), &ticket.id, &agent, None).unwrap();

    assert_eq!(
        *agent.0.lock().unwrap(),
        ["Add an export option.", "Add an export option."]
    );
    let history = watch(data.path(), &ticket.id, 0).unwrap();
    let attempts: Vec<String> = history
        .iter()
        .filter_map(|record| match &record.event.kind {
            Event::StepStarted { attempt, .. } => Some(attempt.as_str().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, ["understand-1", "understand-2"]);
    assert!(
        history
            .iter()
            .any(|record| matches!(record.event.kind, Event::StepReset { and_after: true }))
    );
    assert!(matches!(
        history.last().unwrap().event.kind,
        Event::StepFinished
    ));
    let state = autopilot_pipeline::fold(&history).unwrap();
    assert_eq!(
        state.step(autopilot_core::Step::Understand),
        &autopilot_pipeline::StepState::Passed
    );
}

// Invariant: a reset is validated by the same machine as every other event,
// so a step that has not run, or an unknown ticket, is refused with a reason
// and nothing is written.
#[test]
fn a_reset_that_the_machine_refuses_writes_nothing() {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("again-2"),
        title: "Export".into(),
        description: "x".into(),
    };
    let agent = Remembers(std::sync::Mutex::new(Vec::new()));
    run_understand(data.path(), &ticket, &agent).unwrap();
    let before = watch(data.path(), &ticket.id, 0).unwrap().len();

    let error = reset_step(data.path(), &ticket.id, autopilot_core::Step::Plan, false).unwrap_err();
    assert!(error.to_string().contains("nothing to reset"));
    assert_eq!(watch(data.path(), &ticket.id, 0).unwrap().len(), before);

    reset_step(
        data.path(),
        &ticket.id,
        autopilot_core::Step::Understand,
        true,
    )
    .unwrap();
    assert_eq!(watch(data.path(), &ticket.id, 0).unwrap().len(), before + 1);
    assert!(matches!(
        reset_step(
            data.path(),
            &TicketId::from("nobody"),
            autopilot_core::Step::Understand,
            false
        ),
        Err(Error::UnknownTicket)
    ));
}

fn fold_of(data: &std::path::Path, ticket: &TicketId) -> autopilot_pipeline::TicketState {
    autopilot_pipeline::fold(&watch(data, ticket, 0).unwrap()).unwrap()
}

// Invariant: choosing a pipeline skips the steps it turns off with a reason
// the window can show, and the engine cannot start them. Choosing another
// pipeline brings back only the steps the first one turned off.
#[test]
fn a_pipeline_skips_its_off_steps_and_a_later_choice_restores_them() {
    use autopilot_core::Step;
    use autopilot_pipeline::StepState;
    use autopilot_profile::Profile;
    let data = tempfile::tempdir().unwrap();
    let ticket = understood_with(data.path(), "pipe", None);
    let profile = Profile::parse("p.toml", "").unwrap();

    autopilot_engine::choose_pipeline(data.path(), &ticket, &profile, "bug-fix").unwrap();
    let state = fold_of(data.path(), &ticket);
    assert!(matches!(
        state.step(Step::Design),
        StepState::Skipped(reason) if reason == "off in pipeline bug-fix"
    ));
    assert_eq!(state.step(Step::Plan), &StepState::Idle);
    // Clarify is not off in bug-fix, so it still decides on its own.
    assert!(matches!(
        autopilot_engine::start_clarify(data.path(), &ticket),
        Ok(ClarifyStart::Skipped(_))
    ));

    autopilot_engine::choose_pipeline(data.path(), &ticket, &profile, "ui-change").unwrap();
    assert_eq!(
        fold_of(data.path(), &ticket).step(Step::Design),
        &StepState::Idle
    );
    let history = watch(data.path(), &ticket, 0).unwrap();
    assert_eq!(
        autopilot_engine::chosen_pipeline(&history).unwrap().0,
        "ui-change"
    );
    let view = autopilot_engine::ticket_view(&history).unwrap();
    assert_eq!(view.pipeline.unwrap().0, "ui-change");

    assert!(autopilot_engine::choose_pipeline(data.path(), &ticket, &profile, "nope").is_err());
}

// Invariant: a step that already ran cannot be switched off by a later choice,
// and ask-only closes a ticket after Understand because nothing else runs.
// Without the first, a finished plan could be silently dropped from history.
#[test]
fn a_started_step_cannot_be_switched_off_and_ask_only_finishes() {
    use autopilot_core::Step;
    use autopilot_engine::Badge;
    use autopilot_profile::Profile;
    let data = tempfile::tempdir().unwrap();
    let ticket = understood_with(data.path(), "pipe2", None);
    let profile = Profile::parse("p.toml", "").unwrap();
    // Understand already passed: it is not a step any pipeline can drop.
    let _ = Step::Understand;
    autopilot_engine::choose_pipeline(data.path(), &ticket, &profile, "ask-only").unwrap();
    let history = watch(data.path(), &ticket, 0).unwrap();
    let view = autopilot_engine::ticket_view(&history).unwrap();
    assert_eq!(view.badge, Badge::Idle, "Clarify has not decided yet");
    autopilot_engine::start_clarify(data.path(), &ticket).unwrap();
    let view = autopilot_engine::ticket_view(&watch(data.path(), &ticket, 0).unwrap()).unwrap();
    assert_eq!(view.badge, Badge::Done);

    // A ticket that started the plan cannot move to a pipeline without one.
    let other = understood_with(data.path(), "pipe3", None);
    autopilot_engine::start_clarify(data.path(), &other).unwrap();
    let planner = Planner(autopilot_core::Plan {
        criteria: vec![criterion("c1", autopilot_core::Verify::Test, "t1")],
        test_plan: vec!["t1".into()],
        files: vec![],
    });
    let ticket_row = Ticket {
        id: other.clone(),
        title: "t".into(),
        description: "plain".into(),
    };
    run_plan(data.path(), &ticket_row, &planner).unwrap();
    assert!(
        autopilot_engine::choose_pipeline(data.path(), &other, &profile, "ask-only").is_err(),
        "the plan already started"
    );
}

// Invariant: a retry does not undo the pipeline. Running Understand again, or
// resetting a step with everything after it, leaves the steps the pipeline
// turned off skipped, so an ask-only ticket still cannot be planned. Without
// the re-skip, the reset sends them back to idle and the ticket quietly turns
// into one that plans and builds while its pipeline still says ask-only.
#[test]
fn a_retry_keeps_the_steps_the_pipeline_turned_off_skipped() {
    use autopilot_core::Step;
    use autopilot_pipeline::StepState;
    use autopilot_profile::Profile;
    let data = tempfile::tempdir().unwrap();
    let ticket = understood_with(data.path(), "retry", None);
    let profile = Profile::parse("p.toml", "").unwrap();
    autopilot_engine::choose_pipeline(data.path(), &ticket, &profile, "ask-only").unwrap();
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    rerun_understand(data.path(), &ticket, &agent, Some(false)).unwrap();
    let state = fold_of(data.path(), &ticket);
    for step in [
        Step::Plan,
        Step::BuildCheck,
        Step::PullRequest,
        Step::Design,
    ] {
        assert!(
            matches!(state.step(step), StepState::Skipped(_)),
            "{step:?} after a rerun"
        );
    }
    let row = Ticket {
        id: ticket.clone(),
        title: "t".into(),
        description: "plain".into(),
    };
    let planner = Planner(good_plan());
    assert!(run_plan(data.path(), &row, &planner).is_err());

    reset_step(data.path(), &ticket, Step::Understand, true).unwrap();
    let state = fold_of(data.path(), &ticket);
    assert_eq!(state.step(Step::Understand), &StepState::Idle);
    assert!(matches!(state.step(Step::Plan), StepState::Skipped(_)));
    // The history still folds, so the window does not show a problem.
    let history = watch(data.path(), &ticket, 0).unwrap();
    assert!(autopilot_engine::ticket_view(&history).is_ok());
}
