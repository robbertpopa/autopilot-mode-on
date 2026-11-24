//! Sample data for trying the window without an agent. It writes real events
//! into a data directory, so every screen is drawn by the same code as a live
//! run.

use std::{io, path::Path};

use autopilot_core::{
    Ambiguity, AttemptId, Clarity, Criterion, Event, Money, NewEvent, Plan, RequestId, Step,
    TicketId, Understanding, Verify,
};
use autopilot_pipeline::inbox::{Inbox, Request, RequestBody};
use autopilot_store::{SqliteStore, Store};

const MINUTE: u64 = 60_000;

struct Writer<'a> {
    store: &'a mut SqliteStore,
    ticket: TicketId,
    last: u64,
    at: u64,
}

fn failure(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

impl Writer<'_> {
    fn put(&mut self, step: Option<Step>, kind: Event, after: u64) -> io::Result<()> {
        self.at += after;
        let new = NewEvent {
            at_ms: self.at,
            step,
            kind,
        };
        let written = self
            .store
            .append(&self.ticket, self.last, &[new])
            .map_err(failure)?;
        self.last = written.last().map_or(self.last, |record| record.seq);
        Ok(())
    }

    fn start(&mut self, step: Step, after: u64) -> io::Result<()> {
        let attempt = AttemptId::from(format!("{}-1", step.as_str()));
        let kind = Event::StepStarted {
            attempt,
            session: None,
        };
        self.put(Some(step), kind, after)
    }

    fn finish(&mut self, step: Step, after: u64) -> io::Result<()> {
        self.put(Some(step), Event::StepFinished, after)
    }

    fn check(&mut self, name: &str, passed: bool, note: &str, after: u64) -> io::Result<()> {
        let kind = Event::CheckResult {
            check: name.into(),
            passed,
            note: note.into(),
            artifacts: vec![],
            unknowns: vec![],
            reused: false,
        };
        self.put(Some(Step::BuildCheck), kind, after)
    }

    fn cost(&mut self, step: Step, micro: u64, estimated: bool) -> io::Result<()> {
        let kind = Event::CostTick {
            amount: Money::from_microusd(micro),
            estimated,
        };
        self.put(Some(step), kind, 100)
    }
}

fn understanding(summary: &str, files: &[&str]) -> Understanding {
    Understanding {
        summary: summary.into(),
        ambiguity: Ambiguity::all(Clarity::Clear),
        open_questions: vec![],
        assumptions: vec!["Times are shown in the organiser's timezone".into()],
        needs_design: false,
        ui_files: vec![],
        files_touched: files.iter().map(|file| (*file).into()).collect(),
    }
}

fn plan() -> Plan {
    let criterion = |id: &str, text: &str, verify, test: &str| Criterion {
        id: id.into(),
        text: text.into(),
        verify,
        verification_id: test.into(),
    };
    Plan {
        criteria: vec![
            criterion(
                "c1",
                "Times show in the organiser's timezone",
                Verify::Test,
                "tz_shown",
            ),
            criterion(
                "c2",
                "A recording started near midnight shows the right day",
                Verify::Test,
                "midnight",
            ),
            criterion(
                "c3",
                "A test covers a timezone change",
                Verify::Test,
                "tz_change",
            ),
            criterion(
                "c4",
                "No other list changes",
                Verify::Command,
                "git diff --stat",
            ),
        ],
        test_plan: vec!["cargo test".into()],
        files: vec!["recordings.rs".into(), "page.html".into()],
    }
}

fn building(store: &mut SqliteStore, at: u64) -> io::Result<()> {
    let build = Some(Step::BuildCheck);
    let mut run = Writer {
        store,
        ticket: TicketId::from("demo-timezone"),
        last: 0,
        at,
    };
    let title = "Timezone fix".to_owned();
    run.put(None, Event::TicketCreated { title }, 0)?;
    run.start(Step::Understand, 1000)?;
    let result = understanding(
        "Show recording times in the organiser's timezone",
        &["recordings.rs", "page.html"],
    );
    run.put(
        Some(Step::Understand),
        Event::UnderstandResult { result },
        20_000,
    )?;
    run.cost(Step::Understand, 40_000, false)?;
    run.finish(Step::Understand, 100)?;
    for (step, reason) in [
        (Step::Clarify, "no open questions"),
        (Step::Design, "no UI change"),
    ] {
        let reason = reason.into();
        run.put(Some(step), Event::StepSkipped { reason }, 500)?;
    }
    run.start(Step::Plan, 500)?;
    run.put(
        Some(Step::Plan),
        Event::PlanResult { plan: plan() },
        200_000,
    )?;
    let sha256 = "demo".into();
    run.put(Some(Step::Plan), Event::PlanApproved { sha256 }, 30_000)?;
    run.finish(Step::Plan, 100)?;
    run.start(Step::BuildCheck, 1000)?;
    run.put(build, Event::RoundStarted { round: 1 }, 100)?;
    let summary = "editing recordings.rs".into();
    run.put(build, Event::AgentActivity { summary }, 60_000)?;
    run.check("build, fmt, clippy", true, "", 8 * MINUTE)?;
    run.check("tests", true, "", 20_000)?;
    let note = "a protected path was touched (routes.rs)";
    run.check("protected paths and diff size", false, note, 2_000)?;
    run.put(build, Event::RoundStarted { round: 2 }, 8_000)?;
    run.check("build, fmt, clippy", true, "", 4 * MINUTE)?;
    run.cost(Step::BuildCheck, 620_000, true)
}

fn waiting(store: &mut SqliteStore, directory: &Path, at: u64) -> io::Result<()> {
    let ticket = TicketId::from("demo-heartbeat");
    let request = RequestId::from("demo-approve-understand");
    let mut run = Writer {
        store,
        ticket: ticket.clone(),
        last: 0,
        at,
    };
    let title = "Heartbeat time".to_owned();
    run.put(None, Event::TicketCreated { title }, 0)?;
    run.start(Step::Understand, 1000)?;
    let result = understanding("Show the heartbeat time on the hub page", &["hub.rs"]);
    run.put(
        Some(Step::Understand),
        Event::UnderstandResult { result },
        15_000,
    )?;
    let asked = Event::Asked {
        request: request.clone(),
    };
    run.put(Some(Step::Understand), asked, 100)?;
    Inbox::in_data_dir(directory)
        .write_request(&Request {
            id: request,
            ticket,
            body: RequestBody::Approval {
                action: "Accept what Understand found and go on to Clarify".into(),
                forced: false,
            },
        })
        .map_err(failure)
}

fn failed(store: &mut SqliteStore, at: u64) -> io::Result<()> {
    let mut run = Writer {
        store,
        ticket: TicketId::from("demo-csrf"),
        last: 0,
        at,
    };
    let title = "CSRF test".to_owned();
    run.put(None, Event::TicketCreated { title }, 0)?;
    run.start(Step::Understand, 1000)?;
    let reason = "the agent stopped after 20 turns".into();
    run.put(Some(Step::Understand), Event::Failed { reason }, 45_000)
}

/// A ticket whose plan is approved: the history every build state starts from.
fn approved<'a>(
    store: &'a mut SqliteStore,
    id: &str,
    title: &str,
    at: u64,
) -> io::Result<(Writer<'a>, Option<Step>)> {
    let mut run = Writer {
        store,
        ticket: TicketId::from(id),
        last: 0,
        at,
    };
    run.put(
        None,
        Event::TicketCreated {
            title: title.into(),
        },
        0,
    )?;
    run.start(Step::Understand, 1000)?;
    // Each ticket predicts its own file, so the samples do not warn about one
    // another: a warning here would look like a fault in the page.
    let file = format!("{id}.rs");
    let result = understanding(
        "Show recording times in the organiser's timezone",
        &[file.as_str()],
    );
    run.put(
        Some(Step::Understand),
        Event::UnderstandResult { result },
        20_000,
    )?;
    run.finish(Step::Understand, 100)?;
    for (step, reason) in [
        (Step::Clarify, "no open questions"),
        (Step::Design, "no UI change"),
    ] {
        let reason = reason.into();
        run.put(Some(step), Event::StepSkipped { reason }, 500)?;
    }
    run.start(Step::Plan, 500)?;
    run.put(
        Some(Step::Plan),
        Event::PlanResult { plan: plan() },
        200_000,
    )?;
    let sha256 = "demo".into();
    run.put(Some(Step::Plan), Event::PlanApproved { sha256 }, 30_000)?;
    run.finish(Step::Plan, 100)?;
    Ok((run, Some(Step::BuildCheck)))
}

fn ask(
    run: &mut Writer<'_>,
    directory: &Path,
    step: Step,
    name: &str,
    action: &str,
) -> io::Result<RequestId> {
    let request = RequestId::from(name);
    let asked = Event::Asked {
        request: request.clone(),
    };
    run.put(Some(step), asked, 100)?;
    Inbox::in_data_dir(directory)
        .write_request(&Request {
            id: request.clone(),
            ticket: run.ticket.clone(),
            body: RequestBody::Approval {
                action: action.into(),
                forced: true,
            },
        })
        .map_err(failure)?;
    Ok(request)
}

fn passed_build(run: &mut Writer<'_>) -> io::Result<()> {
    run.start(Step::BuildCheck, 1000)?;
    run.put(
        Some(Step::BuildCheck),
        Event::RoundStarted { round: 1 },
        100,
    )?;
    run.check("build, fmt, clippy", true, "", 4 * MINUTE)?;
    run.check("tests", true, "", 20_000)?;
    run.cost(Step::BuildCheck, 180_000, true)?;
    run.finish(Step::BuildCheck, 100)
}

/// Plan approved, Build not started.
fn plan_approved(store: &mut SqliteStore, at: u64) -> io::Result<()> {
    approved(store, "demo-approved", "Plan approved", at).map(|_| ())
}

/// Build ran out of rounds and asks the owner.
fn build_stopped(store: &mut SqliteStore, directory: &Path, at: u64) -> io::Result<()> {
    let (mut run, build) = approved(store, "demo-stopped", "Build stopped", at)?;
    run.start(Step::BuildCheck, 1000)?;
    run.put(build, Event::RoundStarted { round: 1 }, 100)?;
    run.check("build, fmt, clippy", true, "", 4 * MINUTE)?;
    let note = "tests: midnight failed";
    run.check("tests", false, note, 20_000)?;
    run.cost(Step::BuildCheck, 240_000, true)?;
    let action = "the build needs you. Last failure: tests: midnight failed. Approve (or edit to add a note) to give it more rounds, or reject to stop";
    ask(&mut run, directory, Step::BuildCheck, "demo-stuck", action).map(|_| ())
}

/// Build passed, nothing asked about a pull request yet.
fn build_passed(store: &mut SqliteStore, at: u64) -> io::Result<()> {
    let (mut run, _) = approved(store, "demo-passed", "Build passed", at)?;
    passed_build(&mut run)
}

const PUBLISH: &str =
    "publish a draft pull request: push ticket/demo at 1a2b3c4d to example/demo, base main";

/// The owner is asked to approve one commit.
fn pull_request_asked(store: &mut SqliteStore, directory: &Path, at: u64) -> io::Result<()> {
    let (mut run, _) = approved(store, "demo-asked", "Pull request asked", at)?;
    passed_build(&mut run)?;
    run.start(Step::PullRequest, 500)?;
    ask(
        &mut run,
        directory,
        Step::PullRequest,
        "demo-publish",
        PUBLISH,
    )
    .map(|_| ())
}

/// The owner approved; the push has not happened.
fn approved_not_opened(store: &mut SqliteStore, directory: &Path, at: u64) -> io::Result<()> {
    let (mut run, _) = approved(store, "demo-approved-pr", "Approved, not opened", at)?;
    passed_build(&mut run)?;
    run.start(Step::PullRequest, 500)?;
    let request = ask(
        &mut run,
        directory,
        Step::PullRequest,
        "demo-publish-2",
        PUBLISH,
    )?;
    let answer = "approve".into();
    run.put(
        Some(Step::PullRequest),
        Event::Answered { request, answer },
        5_000,
    )
}

/// The draft pull request exists.
fn opened(store: &mut SqliteStore, directory: &Path, at: u64) -> io::Result<()> {
    let (mut run, _) = approved(store, "demo-opened", "Pull request opened", at)?;
    passed_build(&mut run)?;
    run.start(Step::PullRequest, 500)?;
    let request = ask(
        &mut run,
        directory,
        Step::PullRequest,
        "demo-publish-3",
        PUBLISH,
    )?;
    let answer = "approve".into();
    let pull = Some(Step::PullRequest);
    run.put(pull, Event::Answered { request, answer }, 5_000)?;
    let summary = "Draft pull request #12: https://github.com/example/demo/pull/12".into();
    run.put(pull, Event::AgentActivity { summary }, 3_000)?;
    run.finish(Step::PullRequest, 100)
}

/// A file that marks a data folder as holding sample tickets.
pub const MARKER: &str = "sample-data.txt";

/// Write three sample tickets: one mid-build, one waiting for approval and one
/// that failed. `now_ms` is the current time, so the timeline looks recent.
/// A data directory that already has tickets is left alone.
///
/// # Errors
/// Returns an error if the data directory cannot be written.
pub fn seed(directory: &Path, now_ms: u64) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let mut store = SqliteStore::open(directory.join("events.sqlite3")).map_err(failure)?;
    if !store.tickets().map_err(failure)?.is_empty() {
        return Ok(());
    }
    std::fs::write(
        directory.join(MARKER),
        "sample tickets, not real work
",
    )?;
    let ago = |minutes: u64| now_ms.saturating_sub(minutes * MINUTE);
    building(&mut store, ago(17))?;
    waiting(&mut store, directory, ago(25))?;
    failed(&mut store, ago(90))
}

/// Add one sample ticket for each state of Build and the pull request: plan
/// approved, build stopped for the owner, build passed, pull request asked,
/// approved but not opened, and opened. A data folder that already has them is
/// left alone.
///
/// # Errors
/// Returns an error if the data directory cannot be written.
pub fn seed_build_states(directory: &Path, now_ms: u64) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let mut store = SqliteStore::open(directory.join("events.sqlite3")).map_err(failure)?;
    let known = store.tickets().map_err(failure)?;
    if known.contains(&TicketId::from("demo-approved")) {
        return Ok(());
    }
    let ago = |minutes: u64| now_ms.saturating_sub(minutes * MINUTE);
    plan_approved(&mut store, ago(40))?;
    build_stopped(&mut store, directory, ago(55))?;
    build_passed(&mut store, ago(65))?;
    pull_request_asked(&mut store, directory, ago(75))?;
    approved_not_opened(&mut store, directory, ago(85))?;
    opened(&mut store, directory, ago(120))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopilot_engine::{Badge, load_ticket, tickets};

    // Invariant: the sample data is a valid history, and the views fold it into
    // the three states the design shows.
    #[test]
    fn the_sample_tickets_fold_into_the_states_the_design_shows() {
        let directory = tempfile::tempdir().unwrap();
        seed(directory.path(), 10_000_000_000).unwrap();
        seed(directory.path(), 10_000_000_000).unwrap();
        let ids = tickets(directory.path()).unwrap();
        assert_eq!(ids.len(), 3);
        let badge = |name: &str| {
            load_ticket(directory.path(), &TicketId::from(name))
                .unwrap()
                .badge
        };
        assert_eq!(badge("demo-timezone"), Badge::Running);
        assert_eq!(badge("demo-heartbeat"), Badge::NeedsYou);
        assert_eq!(badge("demo-csrf"), Badge::Failed);
        let build = load_ticket(directory.path(), &TicketId::from("demo-timezone")).unwrap();
        assert_eq!(build.rounds.len(), 2);
        assert!(build.rounds[0].failure().is_some());
    }

    // Invariant: each added ticket reads as the state its name says, so a card
    // drawn for it is not describing some other state.
    #[test]
    fn the_build_states_fold_into_the_states_they_are_named_for() {
        let directory = tempfile::tempdir().unwrap();
        seed_build_states(directory.path(), 10_000_000_000).unwrap();
        seed_build_states(directory.path(), 10_000_000_000).unwrap();
        assert_eq!(tickets(directory.path()).unwrap().len(), 6);
        let view = |name: &str| load_ticket(directory.path(), &TicketId::from(name)).unwrap();
        assert_eq!(view("demo-stopped").badge, Badge::NeedsYou);
        assert_eq!(view("demo-asked").badge, Badge::NeedsYou);
        assert_eq!(view("demo-opened").badge, Badge::Done);
        assert!(view("demo-approved-pr").asking.is_none());
        assert!(view("demo-passed").asking.is_none());
        assert_eq!(view("demo-approved").badge, Badge::Idle);
    }
}
