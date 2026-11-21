//! The Claude adapter against a fake `claude`: real process, real pipes, no model.
// Helpers outside #[test] functions are not covered by the unwrap allowance.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use autopilot_agent::{Agent, AgentEvent, StepRequest};
use autopilot_agent_claude::{ClaudeAgent, ClaudeConfig};
use autopilot_core::{Step, Ticket};
use autopilot_profile::Auth;

fn agent() -> ClaudeAgent {
    let fake = PathBuf::from(env!("CARGO_BIN_EXE_autopilot-fake-agent"));
    let guardian = fake.with_file_name(format!(
        "autopilot-process-guardian{}",
        std::env::consts::EXE_SUFFIX
    ));
    ClaudeAgent::new(
        ClaudeConfig {
            auth: Auth::Subscription,
            model: "haiku".into(),
            effort: None,
            max_turns: 4,
            max_budget_usd: None,
            executable: Some(fake),
        },
        guardian,
    )
    .unwrap()
}

fn run(description: &str) -> Vec<AgentEvent> {
    let ticket = Ticket {
        id: "claude-1".into(),
        title: "Sample".into(),
        description: description.into(),
    };
    agent()
        .start(StepRequest {
            step: Step::Understand,
            ticket: &ticket,
        })
        .unwrap()
        .collect()
}

#[test]
fn preflight_accepts_the_matching_sign_in() {
    agent().preflight().unwrap();
}

// Invariant: a good run reports activity, an estimated subscription cost and
// the structured result, in that order. A change that drops the cost event
// for subscriptions would leave the ledger blind.
#[test]
fn a_good_run_reports_activity_estimated_cost_and_the_result() {
    let events = run("plain");
    assert!(matches!(events.first(), Some(AgentEvent::Started)));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Message { .. }))
    );
    let cost = events
        .iter()
        .position(|e| {
            matches!(
                e,
                AgentEvent::Cost {
                    estimated: true,
                    ..
                }
            )
        })
        .unwrap();
    let result = events
        .iter()
        .position(|e| matches!(e, AgentEvent::Result(_)))
        .unwrap();
    assert!(cost < result);
}

#[test]
fn a_turn_limit_is_a_stop_that_still_reports_cost() {
    let events = run("scenario:max-turns");
    assert!(matches!(events.last(), Some(AgentEvent::Stopped(_))));
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Cost { .. })));
}

#[test]
fn a_rate_limited_run_fails_but_keeps_its_retry_activity() {
    let events = run("scenario:rate-limited");
    assert!(matches!(events.last(), Some(AgentEvent::Failed(_))));
    assert!(
        events.iter().any(
            |e| matches!(e, AgentEvent::Message { summary } if summary.starts_with("api retry"))
        )
    );
}
