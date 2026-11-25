//! A run that stops at a limit, through the Claude adapter, against the fake
//! `claude`.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::{path::PathBuf, time::Duration};

use autopilot_agent::{Agent, AgentEvent, StepRequest};
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

struct Seen {
    activity: Vec<String>,
    cost: Vec<(u64, bool)>,
    stopped: Option<String>,
    result: bool,
}

fn run(scenario: &str) -> Seen {
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
        id: TicketId::from("limit-1"),
        title: "t".into(),
        description: scenario.into(),
    };
    let run = agent
        .start(StepRequest {
            step: Step::Understand,
            ticket: &ticket,
            context: "",
        })
        .unwrap();
    let mut seen = Seen {
        activity: Vec::new(),
        cost: Vec::new(),
        stopped: None,
        result: false,
    };
    for event in run {
        match event {
            AgentEvent::Message { summary } => seen.activity.push(summary),
            AgentEvent::Cost { amount, estimated } => {
                seen.cost.push((amount.as_microusd(), estimated));
            }
            AgentEvent::Stopped(reason) => seen.stopped = Some(reason),
            AgentEvent::Result(_) => seen.result = true,
            AgentEvent::Failed(error) => panic!("a limit is a stop, not a failure: {error}"),
            _ => {}
        }
    }
    seen
}

// Invariant: a run that hits the turn limit ends as a stop that names the
// limit, the turns and the money already spent, and no result. The owner
// needs the figure to choose a higher limit; a stop that said only
// "error_max_turns" hides a $1 loss. Nothing else reads the reason text.
#[test]
fn a_turn_limit_stop_says_what_was_spent() {
    let seen = run("scenario:max-turns");
    assert!(!seen.result);
    let reason = seen.stopped.unwrap();
    assert!(reason.contains("turn limit"), "{reason}");
    assert!(reason.contains("20 turns"), "{reason}");
    // Two Opus turns of 10002 prompt tokens and 500 output tokens, priced
    // from the table, which a subscription counts as an estimate.
    let (micro, estimated) = seen.cost[0];
    assert!(estimated);
    assert!(
        reason.contains(&format!(
            "${}.{:02}",
            micro / 1_000_000,
            (micro % 1_000_000).div_ceil(10_000)
        )),
        "{reason} / {micro}"
    );
}

// Invariant: the cost cap is told apart from the turn limit, because the
// owner raises a different setting for each.
#[test]
fn a_cost_cap_stop_names_the_cap() {
    let reason = run("scenario:budget-cap").stopped.unwrap();
    assert!(reason.contains("cost cap"), "{reason}");
}

// Invariant: while a run works, each turn shows once with the money so far,
// and the counter never goes down. Counting frames instead of messages would
// show four turns here, and summing instead of replacing a message's usage
// would double the figure.
#[test]
fn progress_lines_count_each_turn_once_and_never_go_down() {
    let seen = run("scenario:max-turns");
    let lines: Vec<&String> = seen
        .activity
        .iter()
        .filter(|line| line.starts_with("progress: "))
        .collect();
    assert_eq!(lines.len(), 2, "{:?}", seen.activity);
    assert!(
        lines[0].starts_with("progress: turn 1, about $"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("progress: turn 2, about $"),
        "{}",
        lines[1]
    );
    let amount = |line: &str| -> f64 {
        let at = line.find('$').unwrap() + 1;
        line[at..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(amount(lines[1]) >= amount(lines[0]));
}
