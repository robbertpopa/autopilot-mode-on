//! Checks against throwaway Git repositories. Like the Git crate's integration
//! tests these run on Windows only: the Linux runner needs the process guardian
//! binary, which Cargo builds for the process crate's own tests alone.
#![cfg(windows)]
// Helpers outside #[test] functions panic on setup errors, as the tests do.
#![allow(clippy::unwrap_used)]

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

use autopilot_checks::{
    Check, Context, CriteriaCheck, Environment, Report, Toolbox, standard_checks,
};
use autopilot_core::{Criterion, Plan, Verify};
use autopilot_policy::Policy;
use autopilot_process::Runner;
use autopilot_profile::Profile;

const PROFILE: &str = r#"
[checks.build]
executable = "git"
args = ["rev-parse", "--git-dir"]

[checks.test]
executable = "git"
args = ["log", "--format=%s", "--grep"]

[checks.broken]
executable = "git"
args = ["rev-parse", "--verify", "no-such-ref"]
"#;

fn git(directory: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(arguments)
        .current_dir(directory)
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "-q"]);
    git(
        directory.path(),
        &["commit", "-q", "--allow-empty", "-m", "alpha_test"],
    );
    directory
}

fn toolbox() -> Arc<Toolbox> {
    let profile = Profile::parse("p.toml", PROFILE).unwrap();
    let path: OsString = env::var_os("PATH").unwrap();
    Toolbox::from_profile(
        &profile,
        Arc::new(Runner::new(PathBuf::new())),
        Environment::from_vars(env::vars_os()),
        &path,
    )
}

fn plan(criteria: Vec<Criterion>) -> Plan {
    Plan {
        criteria,
        test_plan: vec![],
        files: vec![],
    }
}

fn criterion(verify: Verify, verification_id: &str) -> Criterion {
    Criterion {
        id: "c1".into(),
        text: "t".into(),
        verify,
        verification_id: verification_id.into(),
    }
}

fn run(check: &dyn Check, worktree: &Path, plan: &Plan) -> Report {
    let cancel = AtomicBool::new(false);
    check
        .run(&Context {
            worktree,
            plan,
            changes: &[],
            cancel: &cancel,
        })
        .unwrap()
}

// Invariant: a command check runs in the worktree it is given and nowhere else.
// The test process itself sits inside a Git repository, so a check that
// ignored the directory would pass in the empty folder and fail this test.
#[test]
fn a_command_check_passes_and_fails_in_the_given_worktree() {
    let toolbox = toolbox();
    let build = toolbox.check("build").unwrap();
    let repo = repository();
    let empty = tempfile::tempdir().unwrap();
    let plan = plan(vec![]);

    let passed = run(&build, repo.path(), &plan);
    assert!(passed.passed(), "{}", passed.note);
    let failed = run(&build, empty.path(), &plan);
    assert!(!failed.passed());
    assert!(failed.note.contains("exit code"), "{}", failed.note);
    assert!(
        failed.note.contains("not a git repository"),
        "the output tail is in the note: {}",
        failed.note
    );
}

#[test]
fn a_missing_executable_fails_with_a_reason_and_does_not_stop_the_others() {
    let profile = Profile::parse(
        "p.toml",
        "[checks.build]\nexecutable = \"no-such-tool-for-autopilot\"\n[checks.fmt]\nexecutable = \"git\"\nargs = [\"--version\"]",
    )
    .unwrap();
    let path = env::var_os("PATH").unwrap();
    let toolbox = Toolbox::from_profile(
        &profile,
        Arc::new(Runner::new(PathBuf::new())),
        Environment::from_vars(env::vars_os()),
        &path,
    );
    let repo = repository();
    let plan = plan(vec![]);
    let build = run(&toolbox.check("build").unwrap(), repo.path(), &plan);
    assert!(!build.passed());
    assert!(build.note.contains("was not found"), "{}", build.note);
    assert!(run(&toolbox.check("fmt").unwrap(), repo.path(), &plan).passed());
}

// Invariant: a `test` criterion passes only when a test with that name
// actually ran. `git log --grep` exits 0 whether or not anything matched,
// like a test filter that selects nothing, so the name must be in the output.
#[test]
fn a_criterion_needs_its_test_to_have_run() {
    let repo = repository();
    let check = CriteriaCheck::new(toolbox());

    let found = run(
        &check,
        repo.path(),
        &plan(vec![criterion(Verify::Test, "alpha_test")]),
    );
    assert!(found.passed(), "{}", found.note);

    let nothing = run(
        &check,
        repo.path(),
        &plan(vec![criterion(Verify::Test, "no_such_test")]),
    );
    assert!(!nothing.passed());
    assert!(
        nothing.unknowns.is_empty(),
        "a missing test is a failure the agent can fix"
    );
    assert!(
        nothing.note.contains("no test named no_such_test ran"),
        "{}",
        nothing.note
    );
}

#[test]
fn a_command_criterion_runs_the_profile_command_it_names() {
    let repo = repository();
    let check = CriteriaCheck::new(toolbox());
    let good = run(
        &check,
        repo.path(),
        &plan(vec![criterion(Verify::Command, "build")]),
    );
    assert!(good.passed(), "{}", good.note);
    let bad = run(
        &check,
        repo.path(),
        &plan(vec![criterion(Verify::Command, "broken")]),
    );
    assert!(!bad.passed());
    assert_eq!(bad.unknowns.len(), 0);
    assert!(
        bad.note.starts_with("0 of 1 criteria verified"),
        "{}",
        bad.note
    );
}

#[test]
fn the_standard_order_skips_commands_the_profile_lacks() {
    let toolbox = toolbox();
    let policy = Arc::new(Policy::new(&Profile::parse("p.toml", "").unwrap()).unwrap());
    let ids: Vec<String> = standard_checks(&toolbox, policy, None)
        .iter()
        .map(|check| check.id().to_owned())
        .collect();
    assert_eq!(ids, ["build", "test", "diff", "criteria"]);
}
