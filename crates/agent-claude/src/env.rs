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
            api_key = Some(value);
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

    #[test]
    fn an_api_step_without_a_dedicated_key_fails_even_if_an_ambient_key_exists() {
        let parent =
            [("ANTHROPIC_API_KEY", "ambient-key")].map(|(name, value)| (name.into(), value.into()));
        assert_eq!(child_env(Auth::Api, parent), Err(EnvError::MissingApiKey));
        let empty = [(API_KEY_SOURCE, "")].map(|(name, value)| (name.into(), value.into()));
        assert_eq!(child_env(Auth::Api, empty), Err(EnvError::MissingApiKey));
    }
}
