//! The child environment is built from nothing, so a credential in the parent
//! can only reach Claude Code by being named here.

use std::ffi::{OsStr, OsString};

use autopilot_profile::Auth;

/// The only place the dedicated API key may come from. The ambient
/// `ANTHROPIC_API_KEY` is never read, so a developer's own key cannot be spent
/// by a step that did not ask for API billing.
pub const API_KEY_SOURCE: &str = "AUTOPILOT_ANTHROPIC_API_KEY";

const API_KEY_TARGET: &str = "ANTHROPIC_API_KEY";

/// Variables Claude Code needs to start, find its sign-in and reach the
/// network. Everything else, including every token, is left behind.
const ALLOWED: &[&str] = &[
    "APPDATA",
    "CLAUDE_CONFIG_DIR",
    "COMSPEC",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "LANG",
    "LOCALAPPDATA",
    "NODE_EXTRA_CA_CERTS",
    "NO_PROXY",
    "PATH",
    "PATHEXT",
    "PROGRAMDATA",
    "SSL_CERT_FILE",
    "SYSTEMDRIVE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "TMPDIR",
    "USERNAME",
    "USERPROFILE",
    "WINDIR",
    "XDG_CONFIG_HOME",
];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvError {
    #[error("API steps need a dedicated key in {API_KEY_SOURCE}")]
    MissingApiKey,
}

/// Build the environment for one step from an explicit snapshot of the parent's.
///
/// Taking the snapshot as an argument keeps this a pure function. Tests can
/// prove the isolation without mutating the real process environment.
///
/// # Errors
/// Returns an error if an API step has no dedicated key.
pub fn child_env(
    auth: Auth,
    parent: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<Vec<(OsString, OsString)>, EnvError> {
    let mut child = Vec::new();
    let mut api_key = None;
    for (name, value) in parent {
        if name_is(&name, API_KEY_SOURCE) {
            api_key = Some(trimmed(value));
        } else if ALLOWED.iter().any(|allowed| name_is(&name, allowed)) {
            child.push((name, value));
        }
    }
    if auth == Auth::Api {
        match api_key {
            Some(key) if !key.is_empty() => child.push((API_KEY_TARGET.into(), key)),
            _ => return Err(EnvError::MissingApiKey),
        }
    }
    Ok(child)
}

/// A key copied from a file with Windows line endings ends in a carriage
/// return, which the sandbox proxy refuses as a header value.
fn trimmed(value: OsString) -> OsString {
    match value.to_str() {
        Some(text) => text.trim().into(),
        None => value,
    }
}

/// Settings that leave Git with no way to authenticate.
const NO_GIT_CREDENTIALS: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
    ("GIT_ASKPASS", ""),
    ("SSH_ASKPASS", ""),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "credential.helper"),
    ("GIT_CONFIG_VALUE_0", ""),
];

/// The environment for a build turn: the same allowlist, plus settings that
/// leave Git with no way to authenticate. A push the hook missed would
/// otherwise use whatever credential helper this machine has configured, which
/// can hold the owner's real tokens.
///
/// # Errors
/// Returns an error if an API step has no dedicated key.
pub fn build_env(
    auth: Auth,
    parent: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<Vec<(OsString, OsString)>, EnvError> {
    let mut child = child_env(auth, parent)?;
    // Command-scope config has the highest precedence. An empty value resets
    // the helper list, so the system and global helpers are dropped.
    for (name, value) in NO_GIT_CREDENTIALS {
        child.push(((*name).into(), (*value).into()));
    }
    Ok(child)
}

/// The whole environment of a build turn inside the Linux boundary. It is
/// built from nothing: the dedicated key (the only secret), the folders the
/// turn may write, and switches that stop Claude Code reaching any host but the
/// provider. Nothing of the Windows environment is carried over, and a
/// subscription sign-in is never available there.
///
/// # Errors
/// Returns an error if there is no dedicated key.
pub fn sandbox_env(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
    config_dir: &str,
    tmp_dir: &str,
) -> Result<Vec<(OsString, OsString)>, EnvError> {
    let key = parent
        .into_iter()
        .find(|(name, value)| name_is(name, API_KEY_SOURCE) && !value.is_empty())
        .map(|(_, value)| trimmed(value))
        .ok_or(EnvError::MissingApiKey)?;
    let mut child: Vec<(OsString, OsString)> = vec![
        (API_KEY_TARGET.into(), key),
        ("CLAUDE_CONFIG_DIR".into(), config_dir.into()),
        ("CLAUDE_CODE_TMPDIR".into(), tmp_dir.into()),
        ("DISABLE_TELEMETRY".into(), "1".into()),
        ("DISABLE_ERROR_REPORTING".into(), "1".into()),
        ("DISABLE_AUTOUPDATER".into(), "1".into()),
        (
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
            "1".into(),
        ),
        ("LANG".into(), "C.UTF-8".into()),
    ];
    for (name, value) in NO_GIT_CREDENTIALS {
        child.push(((*name).into(), (*value).into()));
    }
    Ok(child)
}

/// The name of the secret in the environment a sandboxed turn is given.
pub const SANDBOX_SECRETS: &[&str] = &[API_KEY_TARGET];

/// Windows names are case-insensitive. Matching that way everywhere is safe
/// because no allowed name differs from another only by case.
fn name_is(name: &OsStr, expected: &str) -> bool {
    name.to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> Vec<(OsString, OsString)> {
        [
            ("ANTHROPIC_API_KEY", "ambient-key"),
            ("ANTHROPIC_AUTH_TOKEN", "ambient-token"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("CLAUDE_CODE_USE_VERTEX", "1"),
            ("GITHUB_TOKEN", "gh-secret"),
            ("GH_TOKEN", "gh-secret"),
            ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
            (API_KEY_SOURCE, "dedicated-key"),
            ("SystemRoot", "C:\\Windows"),
            ("userprofile", "C:\\Users\\someone"),
            ("CLAUDE_CONFIG_DIR", "C:\\config"),
        ]
        .map(|(name, value)| (name.into(), value.into()))
        .to_vec()
    }

    fn names(env: &[(OsString, OsString)]) -> Vec<String> {
        let mut names: Vec<_> = env
            .iter()
            .map(|(name, _)| name.to_string_lossy().to_lowercase())
            .collect();
        names.sort();
        names
    }

    // Invariant: a subscription step never carries a credential from the
    // parent. A change from allowlist to blocklist would let a new secret
    // variable through, and no other test inspects the child environment.
    #[test]
    fn a_subscription_step_gets_no_credentials() {
        let env = child_env(Auth::Subscription, parent()).unwrap();
        assert_eq!(
            names(&env),
            ["claude_config_dir", "systemroot", "userprofile"]
        );
        assert!(
            env.iter()
                .all(|(_, value)| !value.to_string_lossy().contains("key"))
        );
    }

    #[test]
    fn an_api_step_gets_only_the_dedicated_key_and_never_the_ambient_one() {
        let env = child_env(Auth::Api, parent()).unwrap();
        let key: Vec<_> = env
            .iter()
            .filter(|(name, _)| name == API_KEY_TARGET)
            .collect();
        assert_eq!(key.len(), 1);
        assert_eq!(key[0].1, "dedicated-key");
        assert!(!names(&env).contains(&"anthropic_auth_token".to_string()));
        assert!(!names(&env).contains(&"github_token".to_string()));
    }

    // Invariant: whitespace around the dedicated key never reaches the agent.
    // A change that breaks it: passing the value through, so a key read from a
    // CRLF file makes every request fail with an invalid header.
    #[test]
    fn a_key_with_a_trailing_line_ending_is_trimmed() {
        let env = sandbox_env(
            [(
                "AUTOPILOT_ANTHROPIC_API_KEY".into(),
                " dedicated-key\r\n".into(),
            )],
            "/c",
            "/t",
        )
        .unwrap();
        let key = env.iter().find(|(name, _)| name == "ANTHROPIC_API_KEY");
        assert_eq!(
            key.map(|(_, v)| v.to_string_lossy().into_owned())
                .as_deref(),
            Some("dedicated-key")
        );
    }

    // Invariant: a turn inside the boundary gets the dedicated key and a fixed
    // set of settings, and nothing from the parent: not the ambient key, a
    // token or a path. A change that breaks it: starting from the parent
    // environment, as the host-side steps do.
    #[test]
    fn a_sandboxed_turn_gets_only_the_dedicated_key_and_fixed_settings() {
        let env = sandbox_env(parent(), "/mnt/d/c", "/mnt/d/c/tmp").unwrap();
        let get = |name: &str| {
            env.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        assert_eq!(get("ANTHROPIC_API_KEY").as_deref(), Some("dedicated-key"));
        assert_eq!(get("CLAUDE_CONFIG_DIR").as_deref(), Some("/mnt/d/c"));
        assert_eq!(get("CLAUDE_CODE_TMPDIR").as_deref(), Some("/mnt/d/c/tmp"));
        assert_eq!(get("GIT_ASKPASS").as_deref(), Some(""));
        for leaked in [
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "SystemRoot",
        ] {
            assert_eq!(get(leaked), None, "{leaked}");
        }
        assert_eq!(
            sandbox_env([("ANTHROPIC_API_KEY".into(), "ambient".into())], "/c", "/t"),
            Err(EnvError::MissingApiKey)
        );
    }

    #[test]
    fn an_api_step_without_a_dedicated_key_fails_even_if_an_ambient_key_exists() {
        let parent =
            [("ANTHROPIC_API_KEY", "ambient-key")].map(|(name, value)| (name.into(), value.into()));
        assert_eq!(child_env(Auth::Api, parent), Err(EnvError::MissingApiKey));
        let empty = [(API_KEY_SOURCE, "")].map(|(name, value)| (name.into(), value.into()));
        assert_eq!(child_env(Auth::Api, empty), Err(EnvError::MissingApiKey));
    }
}
