use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use autopilot_core::{Criterion, Plan, Verify};
use autopilot_process::Runner;
use autopilot_profile::Profile;

use crate::{Check, Context, CriteriaCheck, Environment, Report, Toolbox};

fn run(profile: &str, criteria: Vec<Criterion>) -> Report {
    let profile = Profile::parse("p.toml", profile).unwrap();
    let toolbox = Toolbox::from_profile(
        &profile,
        Arc::new(Runner::new(PathBuf::new())),
        Environment::default(),
        OsStr::new(""),
    );
    let plan = Plan {
        criteria,
        test_plan: vec![],
        files: vec![],
    };
    let cancel = AtomicBool::new(false);
    CriteriaCheck::new(toolbox)
        .run(&Context {
            worktree: Path::new("."),
            plan: &plan,
            changes: &[],
            cancel: &cancel,
        })
        .unwrap()
}

fn criterion(verify: Verify, verification_id: &str) -> Criterion {
    Criterion {
        id: "c1".into(),
        text: "t".into(),
        verify,
        verification_id: verification_id.into(),
    }
}

// Invariant: a criterion that cannot be verified never lets the check pass,
// whatever the agent claims. Every way to be unverifiable is its own branch
// that could fall through to "holds", so each is covered.
#[test]
fn unknown_cannot_pass() {
    for criterion in [
        criterion(Verify::Unknown, ""),
        // The plan may not name a command of its own, only a profile entry.
        criterion(Verify::Command, "rm -rf /"),
        // A flag in place of a test name is refused before anything runs.
        criterion(Verify::Test, "--ignored"),
        // No test command in this profile.
        criterion(Verify::Test, "tests::a"),
    ] {
        let report = run("", vec![criterion.clone()]);
        assert!(!report.passed(), "{criterion:?}");
        assert_eq!(report.unknowns.len(), 1, "{criterion:?}");
    }
}

#[test]
fn environment_keeps_only_the_tools_it_needs() {
    let env = Environment::from_vars([
        ("PATH".into(), "/bin".into()),
        ("GITHUB_TOKEN".into(), "secret".into()),
        ("ANTHROPIC_API_KEY".into(), "secret".into()),
    ]);
    let text = format!("{env:?}");
    assert!(text.contains("PATH"));
    assert!(
        !text.contains("secret") && !text.contains("TOKEN"),
        "{text}"
    );
}
