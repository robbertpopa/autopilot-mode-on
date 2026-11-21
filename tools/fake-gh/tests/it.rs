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
