//! Preflight from `claude auth status --json`.

use autopilot_profile::Auth;
use serde_json::Value;

/// The `authMethod` Claude Code reports for a subscription sign-in. Observed
/// on 2.1.330. The value for API-key use is not yet recorded; see S1.
const SUBSCRIPTION_METHOD: &str = "claude.ai";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PreflightError {
    #[error("Claude Code is not signed in; run `claude auth login`")]
    NotSignedIn,
    #[error("Claude Code is signed in with {found}, but this step needs {wanted}")]
    WrongMethod { found: String, wanted: &'static str },
    #[error("`claude auth status` returned something unreadable")]
    Unreadable,
}

/// Check the signed-in state against the step's `auth`.
///
/// # Errors
/// Returns an error if the status is unreadable, signed out or the wrong kind.
pub fn check(status: &[u8], exited_ok: bool, wanted: Auth) -> Result<(), PreflightError> {
    let value: Value = serde_json::from_slice(status).map_err(|_| PreflightError::Unreadable)?;
    let signed_in = value.get("loggedIn").and_then(Value::as_bool);
    if !exited_ok || signed_in != Some(true) {
        return Err(PreflightError::NotSignedIn);
    }
    let method = value
        .get("authMethod")
        .and_then(Value::as_str)
        .ok_or(PreflightError::Unreadable)?;
    let is_subscription = method == SUBSCRIPTION_METHOD;
    match (wanted, is_subscription) {
        (Auth::Subscription, true) | (Auth::Api, false) => Ok(()),
        (Auth::Subscription, false) => Err(PreflightError::WrongMethod {
            found: method.chars().take(40).collect(),
            wanted: "a subscription",
        }),
        (Auth::Api, true) => Err(PreflightError::WrongMethod {
            found: method.chars().take(40).collect(),
            wanted: "an API key",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBSCRIPTION: &[u8] = br#"{"loggedIn":true,"authMethod":"claude.ai","extra":1}"#;

    #[test]
    fn the_method_must_match_the_step() {
        assert_eq!(check(SUBSCRIPTION, true, Auth::Subscription), Ok(()));
        assert!(matches!(
            check(SUBSCRIPTION, true, Auth::Api),
            Err(PreflightError::WrongMethod { .. })
        ));
        let key = br#"{"loggedIn":true,"authMethod":"api_key"}"#;
        assert_eq!(check(key, true, Auth::Api), Ok(()));
        assert!(check(key, true, Auth::Subscription).is_err());
    }

    #[test]
    fn signed_out_failed_and_garbled_status_are_refused() {
        let out = br#"{"loggedIn":false}"#;
        assert_eq!(
            check(out, true, Auth::Subscription),
            Err(PreflightError::NotSignedIn)
        );
        assert_eq!(
            check(SUBSCRIPTION, false, Auth::Subscription),
            Err(PreflightError::NotSignedIn)
        );
        assert_eq!(
            check(b"not json", true, Auth::Subscription),
            Err(PreflightError::Unreadable)
        );
        assert_eq!(
            check(br#"{"loggedIn":true}"#, true, Auth::Subscription),
            Err(PreflightError::Unreadable)
        );
    }
}
