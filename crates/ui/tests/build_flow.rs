//! Build from the window, through the same actions its buttons use. Windows
//! only, like the other fake-agent tests: the fake agent is a program built
//! beside the test.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use autopilot_core::{Event, Step};
use autopilot_pipeline::StepState;
use autopilot_ui::backend::{Action, AgentChoice, Options, perform, read};

fn git(directory: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(arguments)
        .current_dir(directory)
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

/// A Git repository with one commit on `trunk` and a GitHub remote that only
/// pushes to a local bare repository, so nothing can leave the machine.
fn project_with_remote(root: &Path) -> PathBuf {
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-q", "-b", "trunk"]);
    std::fs::write(project.join("readme.txt"), "hello").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "first"]);
    let bare = root.join("remote.git");
    git(root, &["init", "-q", "--bare", "remote.git"]);
    git(
        &project,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/someone/sample.git",
        ],
    );
    git(
        &project,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            &bare.to_string_lossy(),
        ],
    );
    project
}

// Invariant: from the window, an approved plan is built in a copy of the
// project that Git's own branch describes (the project's branch is `trunk`, not
// `main`), and a pull request is refused until the build has passed. The
// practice agent stands in for Claude (the sandbox tests and the live runs
// cover Claude); a change that let the pull request step start on an unbuilt
// ticket, read the wrong branch or lost the copy fails here. The approval and
// the push themselves are covered by the engine's publish tests.
#[test]
fn the_window_builds_an_approved_plan_and_refuses_a_pull_request_until_it_passes() {
    let directory = tempfile::tempdir().unwrap();
    let project = project_with_remote(directory.path());
    let options = Options {
        data_dir: directory.path().join("data"),
        profile: directory.path().join("profile.toml"),
        view: None,
        ticket: None,
    };
    std::fs::write(
        &options.profile,
        "[checks.build]\nexecutable = \"git\"\nargs = [\"rev-parse\", \"--git-dir\"]\n",
    )
    .unwrap();
    AgentChoice::Practice.save(&options.data_dir);
    perform(
        &options,
        &Action::Start {
            title: "Add export".into(),
            description: "Add an export option".into(),
            attachments: Vec::new(),
            workspace: Some(project),
            pipeline: None,
        },
    )
    .unwrap();
    let id = read(&options).tickets[0].id.clone();
    if read(&options).tickets[0].asking.is_some() {
        perform(&options, &Action::AcceptDefaults { ticket: id.clone() }).unwrap();
    }
    perform(&options, &Action::MakePlan(id.clone())).unwrap();
    perform(
        &options,
        &Action::Decide {
            ticket: id.clone(),
            approve: true,
            always: false,
        },
    )
    .unwrap();

    let built = perform(&options, &Action::MakeBuild(id.clone()));
    assert!(built.is_ok(), "{built:?}");
    assert!(
        autopilot_engine::worktree_of(&options.data_dir, &id).is_some(),
        "the build made a copy of the project"
    );
    // The practice plan has one criterion to check by hand, and an unknown
    // criterion can never pass, so a practice build always stops for the owner
    // with that reason. That is the pipeline working, not a failure.
    let view = read(&options).tickets[0].clone();
    let step_state = view.step(Step::BuildCheck).unwrap().state.clone();
    assert!(matches!(step_state, StepState::Asking(_)), "{step_state:?}");
    let failed: Vec<String> = autopilot_engine::watch(&options.data_dir, &id, 0)
        .unwrap()
        .into_iter()
        .filter_map(|record| match record.event.kind {
            Event::CheckResult {
                passed: false,
                note,
                ..
            } => Some(note),
            _ => None,
        })
        .collect();
    assert!(
        failed.iter().any(|note| note.contains("criteria")),
        "{failed:?}"
    );

    // No pull request can be asked for until the build has passed.
    let refused = perform(&options, &Action::PreparePullRequest(id));
    assert!(refused.is_err(), "{refused:?}");
}
