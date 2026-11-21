use std::{path::Path, sync::Arc, sync::atomic::AtomicBool};

use autopilot_core::Plan;
use autopilot_policy::Policy;
use autopilot_profile::Profile;

use crate::{Change, Check, Context, DiffCheck, Report};

fn run(paths: &[&str]) -> Report {
    let profile = Profile::parse("p.toml", "protected_paths = [\"secrets/**\"]").unwrap();
    let check = DiffCheck::new(Arc::new(Policy::new(&profile).unwrap()));
    let changes: Vec<Change> = paths
        .iter()
        .map(|path| Change {
            path: (*path).into(),
            lines: Some((3, 1)),
        })
        .collect();
    let plan = Plan {
        criteria: vec![],
        test_plan: vec![],
        files: vec![],
    };
    let cancel = AtomicBool::new(false);
    check
        .run(&Context {
            worktree: Path::new("."),
            plan: &plan,
            changes: &changes,
            cancel: &cancel,
        })
        .unwrap()
}

// Invariant: a protected or out-of-repo path fails the round and the note
// names it, so the next build turn can revert it. Lockfiles only force a
// question later; failing on them would block every dependency change.
#[test]
fn protected_paths_fail_and_lockfiles_do_not() {
    let ok = run(&["src/lib.rs", "Cargo.lock"]);
    assert!(ok.passed());
    assert_eq!(ok.note, "2 files changed, +6 -2");

    let bad = run(&["src/lib.rs", "secrets/key.txt"]);
    assert!(!bad.passed());
    assert!(bad.note.contains("secrets/key.txt"), "{}", bad.note);

    assert!(!run(&["../outside.txt"]).passed());
}
