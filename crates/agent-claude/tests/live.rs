//! Spends real money on the dedicated API workspace. Run only on purpose:
//! `cargo test -p autopilot-agent-claude --features live -- --ignored`
//! with `AUTOPILOT_ANTHROPIC_API_KEY` set to the spend-limited key.
#![cfg(feature = "live")]

use std::path::PathBuf;

use autopilot_agent::{Agent, AgentEvent, StepRequest};
use autopilot_agent_claude::{ClaudeAgent, ClaudeConfig};
use autopilot_core::{Step, Ticket};
use autopilot_profile::Auth;

#[test]
#[ignore = "calls the real Claude API; capped at $0.05"]
fn a_synthetic_understand_call_returns_structured_output_within_the_cap() {
    let guardian =
        std::env::var_os("AUTOPILOT_PROCESS_GUARDIAN").map_or_else(PathBuf::new, PathBuf::from);
    let agent = ClaudeAgent::new(
        ClaudeConfig {
            auth: Auth::Api,
            model: "haiku".into(),
            effort: None,
            max_turns: 4,
            max_budget_usd: Some("0.05".into()),
            executable: None,
            timeout: std::time::Duration::from_secs(120),
        },
        guardian,
    )
    .unwrap();
    agent.preflight().unwrap();
    let ticket = Ticket {
        id: "live-1".into(),
        title: "Export a sample file".into(),
        description: "Let a user download the sample list as CSV.".into(),
    };
    let run = agent
        .start(StepRequest {
            step: Step::Understand,
            ticket: &ticket,
        })
        .unwrap();
    let events: Vec<_> = run.collect();
    assert!(
        matches!(events.last(), Some(AgentEvent::Result(_))),
        "{events:?}"
    );
    let cost = events.iter().find_map(|e| match e {
        AgentEvent::Cost { amount, .. } => Some(amount.as_microusd()),
        _ => None,
    });
    assert!(cost.is_some_and(|micro| micro <= 50_000), "{cost:?}");
}
