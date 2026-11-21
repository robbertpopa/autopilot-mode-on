//! The Claude adapter against a fake `claude`: real process, real pipes, no model.
// Helpers outside #[test] functions are not covered by the unwrap allowance.
#![allow(clippy::unwrap_used)]

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

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
            timeout: Duration::from_secs(20),
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

#[test]
fn a_crash_after_activity_fails_and_keeps_the_activity() {
    let events = run("scenario:crash");
    assert!(matches!(events.last(), Some(AgentEvent::Failed(_))));
    assert!(events.iter().any(
        |e| matches!(e, AgentEvent::Message { summary } if summary.starts_with("session started"))
    ));
}

// Invariant: a result cut short by the process ending is never accepted, even
// after a clean exit. Parsing the partial tail as a frame would invent a result.
#[test]
fn a_truncated_result_is_a_failure_not_a_result() {
    let events = run("scenario:truncated");
    assert!(matches!(events.last(), Some(AgentEvent::Failed(_))));
    assert!(!events.iter().any(|e| matches!(e, AgentEvent::Result(_))));
}

#[test]
fn output_beyond_the_stream_cap_fails_the_run() {
    let events = run("scenario:oversized");
    assert!(matches!(events.last(), Some(AgentEvent::Failed(_))));
}

#[test]
fn cancelling_a_hung_run_ends_it_promptly() {
    let ticket = Ticket {
        id: "claude-hang".into(),
        title: "Sample".into(),
        description: "scenario:hang".into(),
    };
    let run = agent()
        .start(StepRequest {
            step: Step::Understand,
            ticket: &ticket,
        })
        .unwrap();
    let cancel = run.cancellation();
    let started = Instant::now();
    let mut last = None;
    for event in run {
        if matches!(event, AgentEvent::Message { .. }) {
            cancel.cancel();
        }
        last = Some(event);
    }
    assert!(matches!(last, Some(AgentEvent::Failed(_))));
    assert!(started.elapsed() < Duration::from_secs(20));
}

// Invariant: the configured wall limit, not the 300 s default, ends a hung
// run. A constant in place of the config would leave this test hanging.
#[test]
fn a_hung_run_is_killed_at_the_configured_timeout() {
    let fake = PathBuf::from(env!("CARGO_BIN_EXE_autopilot-fake-agent"));
    let guardian = fake.with_file_name(format!(
        "autopilot-process-guardian{}",
        std::env::consts::EXE_SUFFIX
    ));
    let agent = ClaudeAgent::new(
        ClaudeConfig {
            auth: Auth::Subscription,
            model: "haiku".into(),
            effort: None,
            max_turns: 4,
            max_budget_usd: None,
            executable: Some(fake),
            timeout: Duration::from_secs(2),
        },
        guardian,
    )
    .unwrap();
    let ticket = Ticket {
        id: "claude-timeout".into(),
        title: "Sample".into(),
        description: "scenario:hang".into(),
    };
    let started = Instant::now();
    let events: Vec<_> = agent
        .start(StepRequest {
            step: Step::Understand,
            ticket: &ticket,
        })
        .unwrap()
        .collect();
    assert!(
        matches!(events.last(), Some(AgentEvent::Failed(error)) if error.to_string().contains("timed out"))
    );
    assert!(started.elapsed() < Duration::from_secs(15));
}
