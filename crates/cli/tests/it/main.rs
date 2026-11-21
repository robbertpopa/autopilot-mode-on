//! `autopilot run-step --agent claude` and `watch` as real processes, with the
//! fake `claude` standing in for the model.
#![allow(clippy::unwrap_used)] // helpers outside #[test] functions

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;

fn fake_claude() -> PathBuf {
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_autopilot"));
    let fake = cli.with_file_name(format!(
        "autopilot-fake-agent{}",
        std::env::consts::EXE_SUFFIX
    ));
    // Cargo builds the fake with the workspace but not for a single-package run.
    assert!(
        fake.is_file(),
        "build the workspace first: cargo build --workspace"
    );
    fake
}

fn autopilot(args: &[&str], data: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_autopilot"))
        .args(args)
        .arg("--data-dir")
        .arg(data)
        .output()
        .unwrap()
}

fn run_claude(description: &str, data: &Path, directory: &Path) -> Output {
    let ticket = directory.join("ticket.json");
    std::fs::write(
        &ticket,
        serde_json::json!({"id": "e2e-1", "title": "Sample", "description": description})
            .to_string(),
    )
    .unwrap();
    let fake = fake_claude();
    autopilot(
        &[
            "run-step",
            "--step",
            "understand",
            "--agent",
            "claude",
            "--claude-path",
            fake.to_str().unwrap(),
            "--ticket",
            ticket.to_str().unwrap(),
        ],
        data,
    )
}

fn history(data: &Path) -> Vec<Value> {
    let output = autopilot(&["watch", "e2e-1"], data);
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_good_claude_run_is_replayable_from_a_separate_process() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    let output = run_claude("plain", &data, scratch.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed["result"]["summary"], "Export a sample file.");
    let kinds: Vec<_> = history(&data)
        .iter()
        .map(|record| record["event"]["kind"].clone())
        .collect();
    let has = |name: &str| kinds.iter().any(|kind| kind["type"] == name);
    assert!(has("agent_activity") && has("understand_result") && has("done"));
    // The subscription run is labelled an estimate, never a bill.
    assert!(
        kinds
            .iter()
            .any(|kind| kind["type"] == "cost_tick" && kind["estimated"] == true)
    );
}

#[test]
fn a_turn_limit_exits_with_an_error_but_is_stored_as_interrupted() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    let output = run_claude("scenario:max-turns", &data, scratch.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stopped at a limit"));
    let kinds: Vec<_> = history(&data)
        .iter()
        .map(|record| record["event"]["kind"]["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds.last().map(String::as_str), Some("interrupted"));
    assert!(!kinds.iter().any(|kind| kind == "failed" || kind == "done"));
}

// The command reads real local state, so only its shape is asserted: three
// named statuses, and nothing credential-like in the output.
#[test]
fn accounts_prints_three_statuses_and_no_credentials() {
    let scratch = tempfile::tempdir().unwrap();
    let output = autopilot(&["accounts", "--json"], scratch.path());
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    for tool in ["claude", "github", "azure"] {
        assert!(value[tool]["state"].is_string(), "{tool}");
    }
    let lower = text.to_lowercase();
    assert!(!lower.contains("token") && !lower.contains("sk-ant") && !lower.contains("gho_"));
}
