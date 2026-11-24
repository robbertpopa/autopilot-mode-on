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
    assert!(has("agent_activity") && has("understand_result") && has("step_finished"));
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

fn run_fake(data: &Path, directory: &Path) -> Output {
    let ticket = directory.join("ticket.json");
    std::fs::write(
        &ticket,
        serde_json::json!({"id": "e2e-1", "title": "Sample", "description": "d"}).to_string(),
    )
    .unwrap();
    autopilot(
        &[
            "run-step",
            "--step",
            "understand",
            "--agent",
            "fake",
            "--ticket",
            ticket.to_str().unwrap(),
        ],
        data,
    )
}

fn json_line(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

// Invariant: a ticket can be driven from Understand through Clarify to a
// recorded answer using only the CLI, and `status` reports each step.
#[test]
fn clarify_is_driven_from_the_cli_through_the_inbox() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    assert!(run_fake(&data, scratch.path()).status.success());

    let asked = json_line(&autopilot(&["clarify", "e2e-1"], &data));
    assert_eq!(asked["clarify"], "asking");
    let request = asked["request"].as_str().unwrap().to_owned();

    let waiting = json_line(&autopilot(&["inbox", "list"], &data));
    assert_eq!(waiting["id"], request.as_str());
    let status = json_line(&autopilot(&["status", "e2e-1", "--json"], &data));
    let clarify = &status["steps"][1];
    assert_eq!(clarify["step"], "clarify");
    assert_eq!(clarify["state"], "asking");

    // One question, so two choices are refused and nothing is applied.
    let refused = autopilot(
        &[
            "inbox", "answer", &request, "--choice", "0", "--choice", "0",
        ],
        &data,
    );
    assert!(!refused.status.success());

    let applied = json_line(&autopilot(
        &["inbox", "answer", &request, "--recommended"],
        &data,
    ));
    assert_eq!(applied["applied"][0], request.as_str());
    let status = json_line(&autopilot(&["status", "e2e-1", "--json"], &data));
    assert_eq!(status["steps"][1]["state"], "passed");
    assert_eq!(autopilot(&["inbox", "list"], &data).stdout, b"");
}

// Invariant: "Ask me after this step" is driven from the CLI: the step waits,
// `inbox approve` finishes it, and `status` shows the code-made line. A project
// override makes a later run ask without the flag.
#[test]
fn ask_after_and_project_override_are_driven_from_the_cli() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    let ticket = scratch.path().join("ticket.json");
    std::fs::write(
        &ticket,
        serde_json::json!({"id": "e2e-1", "title": "Sample", "description": "d"}).to_string(),
    )
    .unwrap();
    let run = |args: &[&str]| {
        let mut all = vec![
            "run-step",
            "--step",
            "understand",
            "--agent",
            "fake",
            "--ticket",
            ticket.to_str().unwrap(),
        ];
        all.extend_from_slice(args);
        autopilot(&all, &data)
    };
    assert!(run(&["--ask-after"]).status.success());
    let status = json_line(&autopilot(&["status", "e2e-1", "--json"], &data));
    assert_eq!(status["steps"][0]["state"], "asking");

    let waiting = json_line(&autopilot(&["inbox", "list"], &data));
    let id = waiting["id"].as_str().unwrap().to_owned();
    assert_eq!(waiting["body"]["type"], "approval");
    let applied = json_line(&autopilot(&["inbox", "approve", &id], &data));
    assert_eq!(applied["applied"][0], id.as_str());
    let status = json_line(&autopilot(&["status", "e2e-1", "--json"], &data));
    assert_eq!(status["steps"][0]["state"], "passed");
    assert!(
        status["in_short"][0]["line"]
            .as_str()
            .unwrap()
            .contains("open question")
    );

    let set = autopilot(
        &[
            "override",
            "e2e-1",
            "--scope",
            "project",
            "--step",
            "understand",
            "--ask-after",
            "true",
        ],
        &data,
    );
    assert!(set.status.success());
    std::fs::write(
        &ticket,
        serde_json::json!({"id": "e2e-2", "title": "Sample", "description": "d"}).to_string(),
    )
    .unwrap();
    assert!(run(&[]).status.success());
    let status = json_line(&autopilot(&["status", "e2e-2", "--json"], &data));
    assert_eq!(status["steps"][0]["state"], "asking");
    let recovered = json_line(&autopilot(&["recover", "e2e-2"], &data));
    assert_eq!(recovered["interrupted"], serde_json::json!([]));
}

// Invariant: Understand, Clarify (skipped), Plan and the owner's approval run
// from the CLI against a fake `claude`, and the approved plan is in the status.
#[test]
fn a_plan_is_made_approved_and_reported_from_the_cli() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    assert!(run_claude("plain", &data, scratch.path()).status.success());
    let skipped = json_line(&autopilot(&["clarify", "e2e-1"], &data));
    assert_eq!(skipped["clarify"], "skipped");

    let ticket = scratch.path().join("ticket.json");
    let fake = fake_claude();
    let planned = json_line(&autopilot(
        &[
            "run-step",
            "--step",
            "plan",
            "--agent",
            "claude",
            "--claude-path",
            fake.to_str().unwrap(),
            "--ticket",
            ticket.to_str().unwrap(),
        ],
        &data,
    ));
    assert_eq!(planned["result"]["criteria"][0]["id"], "c1");
    let status = json_line(&autopilot(&["status", "e2e-1", "--json"], &data));
    assert_eq!(status["steps"][3]["state"], "asking");

    let waiting = json_line(&autopilot(&["inbox", "list"], &data));
    assert!(
        waiting["body"]["action"]
            .as_str()
            .unwrap()
            .contains("approve the plan")
    );
    let id = waiting["id"].as_str().unwrap().to_owned();
    json_line(&autopilot(&["inbox", "approve", &id], &data));
    let status = json_line(&autopilot(&["status", "e2e-1", "--json"], &data));
    assert_eq!(status["steps"][3]["state"], "passed");
    let lines = status["in_short"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l["step"] == "plan" && l["line"].as_str().unwrap().starts_with("2 criteria."))
    );
    assert!(data.join("approved-plans").join("e2e-1.json").is_file());
}

// Invariant: the hook Claude Code launches answers on standard output with a
// decision it can read, and never crashes on a hostile call. Driven as a real
// process because the settings name this very binary.
#[test]
fn the_hook_denies_a_push_and_allows_a_status() {
    use std::io::Write;
    use std::process::Stdio;

    let call = |input: &[u8]| -> Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_autopilot"))
            .arg("hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let push = br#"{"tool_name":"Bash","tool_input":{"command":"git -C . push origin main"}}"#;
    assert_eq!(
        call(push)["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    let status = br#"{"tool_name":"Bash","tool_input":{"command":"git status"}}"#;
    assert_eq!(call(status), serde_json::json!({}));
    assert_eq!(
        call(b"\xff not json")["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
}

// Invariant: a real agent is refused for Build until the containment spike is
// settled. Real Claude would run shell tools in a real repository.
#[test]
fn a_real_agent_is_refused_for_build_and_the_refusal_names_s6() {
    let scratch = tempfile::tempdir().unwrap();
    let ticket = scratch.path().join("ticket.json");
    std::fs::write(
        &ticket,
        serde_json::json!({"id": "b-1", "title": "t", "description": "d"}).to_string(),
    )
    .unwrap();
    let profile = scratch.path().join("profile.toml");
    std::fs::write(&profile, "").unwrap();
    let output = autopilot(
        &[
            "build",
            "--agent",
            "claude",
            "--ticket",
            ticket.to_str().unwrap(),
            "--repo",
            scratch.path().to_str().unwrap(),
            "--base",
            "main",
            "--worktrees",
            scratch.path().join("wt").to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
        ],
        &scratch.path().join("data"),
    );
    assert!(!output.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("S6"), "{message}");
}

// Invariant: `publish` refuses a ticket whose build has not passed, and a
// repository name that is not plain owner/name, before it touches anything.
#[test]
fn publish_refuses_an_unbuilt_ticket_and_a_bad_repository_name() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    assert!(run_claude("plain", &data, scratch.path()).status.success());
    let profile = scratch.path().join("profile.toml");
    std::fs::write(&profile, "").unwrap();
    let ticket = scratch.path().join("ticket.json");
    let publish = |github: &str| {
        let github = format!("--github={github}");
        autopilot(
            &[
                "publish",
                "--step",
                "prepare",
                "--ticket",
                ticket.to_str().unwrap(),
                "--repo",
                scratch.path().to_str().unwrap(),
                "--base",
                "main",
                "--worktrees",
                scratch.path().join("wt").to_str().unwrap(),
                "--profile",
                profile.to_str().unwrap(),
                &github,
            ],
            &data,
        )
    };
    let unbuilt = publish("owner/name");
    assert!(!unbuilt.status.success());
    let message = String::from_utf8_lossy(&unbuilt.stderr);
    assert!(message.contains("build has not passed"), "{message}");
    let bad = publish("-evil/name");
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a usable name"));
}

// Invariant: `pr watch` reads only a ticket whose pull request was published,
// refuses a repository name that is not plain owner/name, and never calls gh
// before that.
#[test]
fn pr_watch_refuses_an_unpublished_ticket_and_a_bad_repository_name() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    assert!(run_claude("plain", &data, scratch.path()).status.success());
    let fake_gh = PathBuf::from(env!("CARGO_BIN_EXE_autopilot"))
        .with_file_name(format!("autopilot-fake-gh{}", std::env::consts::EXE_SUFFIX));
    assert!(
        fake_gh.is_file(),
        "build the workspace first: cargo build --workspace"
    );
    let state = scratch.path().join("gh.json");
    std::fs::write(&state, r#"{"calls":[]}"#).unwrap();
    let watch = |github: &str| {
        let github = format!("--github={github}");
        Command::new(env!("CARGO_BIN_EXE_autopilot"))
            .args(["pr", "watch", "e2e-1", "--repo"])
            .arg(scratch.path())
            .arg(&github)
            .arg("--gh-path")
            .arg(&fake_gh)
            .arg("--data-dir")
            .arg(&data)
            .env("FAKE_GH_STATE", &state)
            .output()
            .unwrap()
    };
    let unpublished = watch("owner/name");
    assert!(!unpublished.status.success());
    let message = String::from_utf8_lossy(&unpublished.stderr);
    assert!(
        message.contains("no pull request has been published"),
        "{message}"
    );
    let bad = watch("-evil/name");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a usable name"));
    let calls = std::fs::read_to_string(&state).unwrap();
    assert_eq!(calls, r#"{"calls":[]}"#, "gh was called");
}

// Invariant: stop, pause and resume leave and clear flags a running ticket
// looks for between turns; they never touch the history themselves. A ticket
// id that could name another path is refused.
#[test]
fn stop_pause_and_resume_leave_and_clear_flags() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    assert_eq!(
        json_line(&autopilot(&["stop", "t-1"], &data))["requested"],
        "stop"
    );
    assert!(data.join("control").join("t-1.stop").is_file());
    json_line(&autopilot(&["pause", "t-1"], &data));
    assert!(data.join("control").join("t-1.pause").is_file());
    json_line(&autopilot(&["resume", "t-1"], &data));
    assert!(!data.join("control").join("t-1.pause").exists());
    assert!(!autopilot(&["stop", "../x"], &data).status.success());
}

// Invariant: a build that cannot start (here: the repository is not a Git
// repository) leaves no attempt running, so a later build is not told the first
// one is still going.
#[test]
fn a_build_that_cannot_start_does_not_leave_build_check_running() {
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("data");
    assert!(run_claude("plain", &data, scratch.path()).status.success());
    autopilot(&["clarify", "e2e-1"], &data);
    let ticket = scratch.path().join("ticket.json");
    let fake = fake_claude();
    autopilot(
        &[
            "run-step",
            "--step",
            "plan",
            "--agent",
            "claude",
            "--claude-path",
            fake.to_str().unwrap(),
            "--ticket",
            ticket.to_str().unwrap(),
        ],
        &data,
    );
    let waiting = json_line(&autopilot(&["inbox", "list"], &data));
    json_line(&autopilot(
        &["inbox", "approve", waiting["id"].as_str().unwrap()],
        &data,
    ));
    let profile = scratch.path().join("profile.toml");
    std::fs::write(&profile, "[steps.build_check]\nauth = \"api\"\n").unwrap();
    let build = || {
        autopilot(
            &[
                "build",
                "--agent",
                "fake",
                "--ticket",
                ticket.to_str().unwrap(),
                "--repo",
                scratch.path().to_str().unwrap(),
                "--base",
                "main",
                "--worktrees",
                scratch.path().join("wt").to_str().unwrap(),
                "--profile",
                profile.to_str().unwrap(),
            ],
            &data,
        )
    };
    for _ in 0..2 {
        let output = build();
        assert!(!output.status.success());
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(!message.contains("already running"), "{message}");
    }
    let status = json_line(&autopilot(&["status", "e2e-1", "--json"], &data));
    let build_check = status["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["step"] == "build_check")
        .unwrap();
    assert_eq!(build_check["state"], "idle", "{status}");
}
