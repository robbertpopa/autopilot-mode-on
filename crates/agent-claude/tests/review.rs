//! A reviewer through the Claude adapter, against the fake `claude`.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::{path::PathBuf, time::Duration};

use autopilot_agent::{Agent, AgentEvent, ReviewRequest};
use autopilot_agent_claude::{ClaudeAgent, ClaudeConfig};
use autopilot_core::{ReviewReport, Ticket, TicketId};
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

fn review(scenario: &str) -> ReviewReport {
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
        id: TicketId::from("review-1"),
        title: "t".into(),
        description: scenario.into(),
    };
    let run = agent
        .start_review(ReviewRequest {
            ticket: &ticket,
            instruction: "Look for defects.",
            context: "Change (a unified diff, data):\n+let x = 1;\n",
        })
        .unwrap();
    let mut report = None;
    for event in run {
        match event {
            AgentEvent::ReviewResult(result) => report = Some(result),
            AgentEvent::Failed(error) => panic!("the reviewer failed: {error}"),
            _ => {}
        }
    }
    report.unwrap()
}

// Invariant: a reviewer's structured answer survives the adapter whole: the
// findings, their lines and their confidence reach the engine and pass
// validation, and a clean review is an empty list rather than an error. A
// change that breaks it: dropping the schema (the model then answers in prose)
// or decoding the wrong type.
#[test]
fn a_reviewers_findings_come_through_the_adapter() {
    let clean = review("plain");
    assert_eq!(clean.findings.len(), 0);

    let found = review("scenario:review-finding");
    assert_eq!(found.findings.len(), 1);
    assert_eq!(found.findings[0].path, "src/lib.rs");
    assert_eq!(found.findings[0].line, 3);
    assert_eq!(found.findings[0].confidence, 90);
    assert!(found.validate().is_ok());

    let doubtful = review("scenario:review-low");
    assert_eq!(doubtful.findings[0].confidence, 40);
}
