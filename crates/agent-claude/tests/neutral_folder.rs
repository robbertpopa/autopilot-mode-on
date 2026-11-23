//! Where a host-run call starts when the ticket has no project folder.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::{path::PathBuf, time::Duration};

use autopilot_agent::{Agent, StepRequest};
use autopilot_agent_claude::{ClaudeAgent, ClaudeConfig};
use autopilot_core::{Step, Ticket, TicketId};
use autopilot_profile::Auth;

fn fake() -> PathBuf {
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

// Invariant: a call with no project does not start in the folder this process
// runs in. Claude Code reads the CLAUDE.md of its working folder, so starting
// in the owner's own repository steers a ticket about something else with that
// repository's rules (a live run offered to commit to this repository's
// branch). A change that breaks it: leaving the working folder unset.
#[test]
fn a_call_without_a_project_starts_in_an_empty_folder_of_its_own() {
    let config = ClaudeConfig {
        auth: Auth::Subscription,
        model: "fake".into(),
        effort: None,
        max_turns: 4,
        max_budget_usd: None,
        executable: Some(fake()),
        timeout: Duration::from_secs(30),
    };
    let agent = ClaudeAgent::new(config, PathBuf::new()).unwrap();
    let ticket = Ticket {
        id: TicketId::from("neutral-1"),
        title: "t".into(),
        description: "scenario:record-cwd".into(),
    };
    let here = std::env::current_dir().unwrap().join("cwd-marker.txt");
    let run = agent
        .start(StepRequest {
            step: Step::Understand,
            ticket: &ticket,
            context: "",
        })
        .unwrap();
    for _ in run {}
    let neutral = std::env::temp_dir()
        .join("autopilot-neutral")
        .join("cwd-marker.txt");
    assert!(
        neutral.is_file(),
        "the call did not start in the neutral folder"
    );
    assert!(!here.exists(), "the call started in the process folder");
    std::fs::remove_file(neutral).unwrap();
}
