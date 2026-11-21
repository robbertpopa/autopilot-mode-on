//! The Build step against a throwaway Git repository, with the fake `claude`
//! standing in for the model. Windows only, like the checks' own tests: the
//! Linux runner needs the process guardian binary.
#![allow(clippy::unwrap_used)] // helpers outside #[test] functions

use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use autopilot_agent::{Agent, AgentError, AgentRun, StepRequest};
use autopilot_agent_claude::{BuildConfig, ClaudeAgent, ClaudeConfig};
use autopilot_core::{
    Ambiguity, Clarity, Criterion, Event, Plan, Ticket, TicketId, Understanding, Verify,
};
use autopilot_engine::{
    BuildOutcome, BuildSetup, Error, Isolation, run_build, run_plan, run_understand, start_clarify,
    sync_inbox, watch,
};
use autopilot_pipeline::inbox::{Decision, Inbox, Reply};
use autopilot_profile::{Auth, Profile};

/// The build check passes only once `fixed.txt` is committed, which the fake
/// writes on a turn whose prompt carries a failure.
const NEEDS_FIX: &str = r#"
protected_paths = ["secrets/**"]

[steps.build_check.limits]
rounds = 3

[checks.build]
executable = "git"
args = ["cat-file", "-e", "HEAD:fixed.txt"]
"#;

/// Everything passes except the protected-path check.
const ALWAYS_PASS: &str = r#"
protected_paths = ["secrets/**"]

[checks.build]
executable = "git"
args = ["rev-parse", "--git-dir"]
"#;

const ONE_ROUND: &str = r#"
[steps.build_check.limits]
rounds = 1

[checks.build]
executable = "git"
args = ["cat-file", "-e", "HEAD:fixed.txt"]
"#;

struct Understander {
    needs_design: bool,
}

impl Agent for Understander {
    fn start(&self, _request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        let needs_design = self.needs_design;
        Ok(AgentRun::spawn(move |_| {
            Ok(Understanding {
                summary: "Implement the change.".into(),
                ambiguity: Ambiguity::all(Clarity::Clear),
                open_questions: vec![],
                assumptions: vec![],
                needs_design,
                ui_files: vec![],
                files_touched: vec![],
            })
        })?)
    }
}

struct Planner;

impl Agent for Planner {
    fn start(&self, _request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        Err(io::Error::other("planner only plans").into())
    }

    fn start_plan(&self, _request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        Ok(AgentRun::spawn_plan(move |_| {
            Ok(Plan {
                criteria: vec![Criterion {
                    id: "c1".into(),
                    text: "The build check passes".into(),
                    verify: Verify::Command,
                    verification_id: "build".into(),
                }],
                test_plan: vec![],
                files: vec![],
            })
        })?)
    }
}

fn git(directory: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(arguments)
        .current_dir(directory)
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

struct World {
    data: tempfile::TempDir,
    repository: tempfile::TempDir,
    worktrees: tempfile::TempDir,
    ticket: Ticket,
}

/// A ticket with its plan approved, in a repository with one commit on `main`.
fn world(id: &str, description: &str, needs_design: bool) -> World {
    let data = tempfile::tempdir().unwrap();
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(repository.path().join("readme.txt"), "hello").unwrap();
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "-q", "-m", "first"]);
    let ticket = Ticket {
        id: TicketId::from(id),
        title: "Build".into(),
        description: description.into(),
    };
    run_understand(data.path(), &ticket, &Understander { needs_design }).unwrap();
    start_clarify(data.path(), &ticket.id).unwrap();
    run_plan(data.path(), &ticket, &Planner).unwrap();
    answer(data.path(), &ticket.id, Decision::Approve);
    World {
        data,
        repository,
        worktrees: tempfile::tempdir().unwrap(),
        ticket,
    }
}

fn answer(data: &Path, ticket: &TicketId, decision: Decision) {
    let inbox = Inbox::in_data_dir(data);
    let id = inbox.requests().unwrap().remove(0).id;
    let reply = Reply::Approval {
        decision,
        allow_for_project: false,
    };
    inbox.write_answer(&id, &reply).unwrap();
    sync_inbox(data, ticket).unwrap();
}

fn fake() -> PathBuf {
    // Test binaries sit in target/<profile>/deps, the fake one level up.
    let exe = std::env::current_exe().unwrap();
    let fake = exe.parent().unwrap().parent().unwrap().join(format!(
        "autopilot-fake-agent{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        fake.is_file(),
        "build the workspace first: cargo build --workspace"
    );
    fake
}

fn agent(world: &World) -> ClaudeAgent {
    let config = ClaudeConfig {
        auth: Auth::Subscription,
        model: "fake".into(),
        effort: None,
        max_turns: 8,
        max_budget_usd: None,
        executable: Some(fake()),
        timeout: Duration::from_secs(60),
    };
    ClaudeAgent::new(config, PathBuf::new())
        .unwrap()
        .with_build(BuildConfig {
            hook_command: "echo hook".into(),
            data_dir: world.data.path().to_path_buf(),
        })
}

fn build(world: &World, profile: &str) -> Result<BuildOutcome, Error> {
    let profile = Profile::parse("p.toml", profile).unwrap();
    let setup = BuildSetup {
        repository: world.repository.path(),
        base_branch: "main",
        worktrees: world.worktrees.path(),
        git: PathBuf::from("git"),
        guardian: PathBuf::new(),
        profile: &profile,
    };
    run_build(
        world.data.path(),
        &world.ticket,
        &agent(world),
        &setup,
        Isolation::Fixture,
    )
}

fn kinds(world: &World) -> Vec<Event> {
    watch(world.data.path(), &world.ticket.id, 0)
        .unwrap()
        .into_iter()
        .map(|record| record.event.kind)
        .collect()
}

fn failures(world: &World) -> Vec<String> {
    kinds(world)
        .into_iter()
        .filter_map(|kind| match kind {
            Event::CheckResult {
                passed: false,
                note,
                ..
            } => Some(note),
            _ => None,
        })
        .collect()
}

// Invariant: a failure goes back to the agent and the next turn can fix it;
// the step finishes only after the checks pass on the committed tree.
#[test]
fn a_failed_check_is_fixed_on_the_next_round() {
    let world = world("b-fix", "plain", false);
    assert_eq!(build(&world, NEEDS_FIX).unwrap(), BuildOutcome::Ready);
    let kinds = kinds(&world);
    assert!(matches!(kinds.last(), Some(Event::StepFinished)));
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, Event::SessionStarted { .. }))
    );
    assert_eq!(failures(&world).len(), 1, "round one failed once");
}

// Invariant: an agent that never gets it right is stopped by the round cap
// and handed to the owner, not retried forever.
#[test]
fn a_build_that_never_passes_stops_at_the_round_cap_and_asks_the_owner() {
    let world = world("b-cap", "scenario:build-never-fixes", false);
    let BuildOutcome::NeedsYou { request, .. } = build(&world, NEEDS_FIX).unwrap() else {
        panic!("expected the owner to be asked");
    };
    assert_eq!(request.as_str(), "build_check-stuck");
    let waiting = Inbox::in_data_dir(world.data.path()).requests().unwrap();
    assert_eq!(waiting.len(), 1);
    assert!(matches!(
        kinds(&world).last(),
        Some(Event::Asked { request }) if request.as_str() == "build_check-stuck"
    ));
}

// Invariant: a turn that changes nothing is not worth paying for again.
#[test]
fn a_build_that_changes_nothing_goes_to_the_owner() {
    let world = world("b-idle", "scenario:build-nothing", false);
    assert!(matches!(
        build(&world, NEEDS_FIX).unwrap(),
        BuildOutcome::NeedsYou { .. }
    ));
}

// Invariant: a protected path touched by the agent is caught from the
// committed change list, even though the file is new and untracked until the
// host commits it.
#[test]
fn a_protected_path_is_caught_and_not_refined() {
    let world = world("b-protected", "scenario:build-protected", false);
    assert!(matches!(
        build(&world, ALWAYS_PASS).unwrap(),
        BuildOutcome::NeedsYou { .. }
    ));
    let failed = failures(&world);
    assert!(
        failed.iter().any(|note| note.contains("secrets")),
        "{failed:?}"
    );
}

// Invariant: approving a stuck build gives it more rounds and carries the
// owner's note into the next turn; rejecting it ends the attempt.
#[test]
fn the_owner_can_continue_or_reject_a_stuck_build() {
    let world = world("b-stuck", "plain", false);
    assert!(matches!(
        build(&world, ONE_ROUND).unwrap(),
        BuildOutcome::NeedsYou { .. }
    ));
    // The fake fixes the tree on a prompt that contains FAILED:, so the note
    // is how this test shows the hint reached the agent.
    answer(
        world.data.path(),
        &world.ticket.id,
        Decision::Edit("FAILED: write fixed.txt".into()),
    );
    assert_eq!(build(&world, ONE_ROUND).unwrap(), BuildOutcome::Ready);

    let world = world_stuck("b-reject");
    answer(world.data.path(), &world.ticket.id, Decision::Reject);
    assert!(matches!(
        kinds(&world).last(),
        Some(Event::Interrupted { .. })
    ));
}

fn world_stuck(id: &str) -> World {
    let world = world(id, "plain", false);
    assert!(matches!(
        build(&world, ONE_ROUND).unwrap(),
        BuildOutcome::NeedsYou { .. }
    ));
    world
}

// Invariant: Build waits for a settled design when the ticket needs one.
#[test]
fn build_waits_for_the_design_when_the_ticket_needs_one() {
    let world = world("b-design", "plain", true);
    assert!(matches!(build(&world, NEEDS_FIX), Err(Error::NotReady(_))));
}
