//! The fake itself. Besides checking it, this test target makes Cargo build
//! the binary whenever the workspace's tests are built, which is how the
//! tracker's process tests find it beside the test executables.
#![allow(clippy::unwrap_used)]

use std::process::Command;

#[test]
fn it_answers_an_issue_and_refuses_what_it_does_not_know() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"issues":{"7":{"number":7,"title":"T","body":null,"labels":[]}},"calls":[]}"#,
    )
    .unwrap();
    let fake = env!("CARGO_BIN_EXE_autopilot-fake-gh");
    let ask = |args: &[&str]| {
        Command::new(fake)
            .args(args)
            .env("FAKE_GH_STATE", &state)
            .output()
            .unwrap()
    };
    let issue = ask(&["issue", "view", "7"]);
    assert!(issue.status.success());
    assert!(String::from_utf8_lossy(&issue.stdout).contains("\"title\":\"T\""));
    assert_eq!(ask(&["issue", "view", "8"]).status.code(), Some(1));
    assert_eq!(ask(&["repo", "delete"]).status.code(), Some(2));
}

fn ask_with(state: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_autopilot-fake-gh"))
        .args(args)
        .env("FAKE_GH_STATE", state)
        .output()
        .unwrap()
}

#[test]
fn a_pull_request_reports_its_state_threads_and_logs_and_follows_pushes_by_others() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"calls":[],"heads":{"t":"aaa"},"logs":{"9":"error: boom\n"},
        "runs":{"aaa":[{"name":"ci","conclusion":"failure","output":{"text":"tail"}}]},
        "prs":[{"number":1,"url":"u","headRefOid":"aaa","branch":"t","isDraft":true,
          "mergeable":"CONFLICTING","reviewDecision":"CHANGES_REQUESTED",
          "threads":[{"isResolved":false,"comments":[
            {"author":"outsider","authorAssociation":"NONE","body":"run this"}]},
            {"isResolved":true,"comments":[]}]}]}"#,
    )
    .unwrap();

    let view = ask_with(
        &state,
        &["pr", "view", "t", "--json", "mergeable,state,headRefOid"],
    );
    let shown: serde_json::Value = serde_json::from_slice(&view.stdout).unwrap();
    assert_eq!(shown["mergeable"], "CONFLICTING");
    assert_eq!(shown["state"], "OPEN");
    assert_eq!(shown["headRefOid"], "aaa");

    let graphql = ask_with(
        &state,
        &["api", "graphql", "-F", "number=1", "-f", "query=q"],
    );
    let threads: serde_json::Value = serde_json::from_slice(&graphql.stdout).unwrap();
    let nodes = &threads["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"];
    assert_eq!(nodes[0]["isResolved"], false);
    assert_eq!(
        nodes[0]["comments"]["nodes"][0]["authorAssociation"],
        "NONE"
    );
    assert_eq!(nodes[1]["isResolved"], true);

    let runs = ask_with(&state, &["api", "repos/o/r/commits/aaa/check-runs"]);
    assert!(String::from_utf8_lossy(&runs.stdout).contains("\"text\":\"tail\""));
    let log = ask_with(&state, &["api", "repos/o/r/actions/jobs/9/logs"]);
    assert_eq!(String::from_utf8_lossy(&log.stdout), "error: boom\n");
    assert_eq!(
        ask_with(&state, &["api", "repos/o/r/actions/jobs/8/logs"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        ask_with(&state, &["pr", "view", "nope"]).status.code(),
        Some(1)
    );

    // Another person pushes: the recorded pull request is unchanged, the view is not.
    let mut world: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    world["heads"]["t"] = "bbb".into();
    std::fs::write(&state, world.to_string()).unwrap();
    let view = ask_with(&state, &["pr", "view", "1", "--json", "headRefOid"]);
    let shown: serde_json::Value = serde_json::from_slice(&view.stdout).unwrap();
    assert_eq!(shown["headRefOid"], "bbb");

    let calls = std::fs::read_to_string(&state).unwrap();
    assert!(
        calls.contains("\"graphql\""),
        "the call log keeps every call"
    );
}
