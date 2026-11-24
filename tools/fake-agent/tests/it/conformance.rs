//! Both adapters held to the same rules by the shared suite in `agent`.
#![allow(clippy::unwrap_used)]

use std::{path::PathBuf, time::Duration};

use autopilot_agent::{
    Agent, AgentError, AgentRun, StepRequest,
    conformance::{self, Scenario, Subject},
};
use autopilot_agent_claude::{ClaudeAgent, ClaudeConfig};
use autopilot_agent_fixture::FixtureAgent;
use autopilot_core::{Step, Ticket};
use autopilot_profile::Auth;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_autopilot-fake-agent"))
}

fn ticket(description: &str) -> Ticket {
    Ticket {
        id: "conformance-1".into(),
        title: "Sample".into(),
        description: description.into(),
    }
}

fn understand(agent: &dyn Agent, ticket: &Ticket) -> Result<AgentRun, AgentError> {
    agent.start(StepRequest {
        step: Step::Understand,
        ticket,
        context: "",
    })
}

fn claude() -> ClaudeAgent {
    let fake = fake();
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

// Invariant: the Claude adapter keeps every rule of the shared suite. A change
// to its stream handling that turns an unknown frame into an error, or lets a
// cut-short stream pass as a result, fails here. The adapter's own tests check
// single cases; none of them checks all the rules together.
#[test]
fn the_claude_adapter_passes_the_conformance_suite() {
    let agent = claude();
    let sample = ticket("plain");
    let start = |scenario| {
        let description = match scenario {
            Scenario::Normal => "plain",
            Scenario::UnknownEvents => "scenario:unknown-events",
            Scenario::NoResult => "scenario:truncated",
            Scenario::Hang => "scenario:hang",
        };
        understand(&agent, &ticket(description))
    };
    let result = conformance::run(&Subject {
        agent: &agent,
        ticket: &sample,
        start: &start,
        cancel_within: Duration::from_secs(10),
    });
    assert!(result.is_ok(), "{result:?}");
}

// Invariant: the practice agent keeps the same rules, so the engine can swap
// one for the other in tests without a behaviour change.
#[test]
fn the_fixture_adapter_passes_the_conformance_suite() {
    let cli = fake().with_file_name(format!("autopilot{}", std::env::consts::EXE_SUFFIX));
    let sample = ticket("plain");
    let agent = FixtureAgent::beside(&cli).unwrap();
    let start = |scenario| {
        let faulty = match scenario {
            Scenario::Normal => FixtureAgent::beside(&cli),
            Scenario::UnknownEvents => FixtureAgent::beside(&cli).map(|a| a.with_fault("unknown")),
            Scenario::NoResult => FixtureAgent::beside(&cli).map(|a| a.with_fault("truncated")),
            Scenario::Hang => FixtureAgent::beside(&cli).map(|a| a.with_fault("hang")),
        }?;
        understand(&faulty, &sample)
    };
    let result = conformance::run(&Subject {
        agent: &agent,
        ticket: &sample,
        start: &start,
        cancel_within: Duration::from_secs(10),
    });
    assert!(result.is_ok(), "{result:?}");
}
