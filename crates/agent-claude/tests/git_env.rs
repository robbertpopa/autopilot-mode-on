//! The build turn's environment against the real `git`.
#![allow(clippy::unwrap_used)]

use std::{ffi::OsString, process::Command};

use autopilot_agent_claude::build_env;
use autopilot_profile::Auth;

fn parent() -> Vec<(OsString, OsString)> {
    [
        ("GITHUB_TOKEN", "gh-secret"),
        ("GH_TOKEN", "gh-secret"),
        ("SystemRoot", "C:\\Windows"),
    ]
    .map(|(name, value)| (name.into(), value.into()))
    .to_vec()
}

// Invariant: Git run by a build turn has no credential helper and cannot
// prompt, even when the machine's own config names one. Checked with the real
// git against a config file that sets a helper, since the whole point is that
// command-scope config wins over it. This PC's Credential Manager holds real
// tokens, so a push that slipped past the hook must not be able to use them.
#[test]
fn git_in_a_build_turn_has_its_credential_helpers_reset() {
    let directory = tempfile::tempdir().unwrap();
    let global = directory.path().join("gitconfig");
    std::fs::write(&global, "[credential]\n\thelper = manager\n").unwrap();

    let env = build_env(Auth::Subscription, parent()).unwrap();
    let mut command = Command::new("git");
    command
        .args(["config", "--get-all", "credential.helper"])
        .env_clear()
        .env("GIT_CONFIG_GLOBAL", &global)
        .env("PATH", std::env::var_os("PATH").unwrap());
    for (name, value) in env
        .iter()
        .filter(|(name, _)| name.to_string_lossy().starts_with("GIT_CONFIG_"))
    {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    // Git lists every value in order; an empty last value resets the list.
    assert_eq!(text.lines().last(), Some(""), "{text:?}");

    let names: Vec<String> = env
        .iter()
        .map(|(name, _)| name.to_string_lossy().to_lowercase())
        .collect();
    assert!(!names.contains(&"gh_token".to_string()));
    assert!(!names.contains(&"github_token".to_string()));
    assert!(
        env.iter()
            .any(|(name, value)| name == "GIT_TERMINAL_PROMPT" && value == "0")
    );
}
