//! Which accounts the local Claude Code, GitHub CLI and Azure CLI are signed in to.
//!
//! This only reads what each tool already reports about itself. No token is
//! requested, stored or returned: the tools are asked for status, and the
//! parsers keep names, never credentials.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::atomic::AtomicBool,
    thread,
    time::Duration,
};

use autopilot_process::{CommandSpec, Limits, Outcome, Runner};
use serde::Serialize;
use serde_json::Value;

const LIMITS: Limits = Limits {
    // Azure CLI starts a Python runtime and can take several seconds.
    timeout: Duration::from_secs(40),
    bytes_per_stream: 256 * 1024,
};
const SHOWN_CHARS: usize = 80;

/// The tools get only what they need to find their own sign-in. Ambient token
/// variables are left out on purpose, so the answer describes the stored
/// sign-in and not whatever happens to be exported in this shell.
const ENV_ALLOWED: &[&str] = &[
    "APPDATA",
    "AZURE_CONFIG_DIR",
    "CLAUDE_CONFIG_DIR",
    "COMSPEC",
    "GH_CONFIG_DIR",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "LANG",
    "LOCALAPPDATA",
    "NO_PROXY",
    "PATH",
    "PATHEXT",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum Status<T> {
    SignedIn(T),
    SignedOut,
    NotInstalled,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClaudeAccount {
    /// `claude.ai` for a subscription, `api_key` for an API key.
    pub method: String,
    pub email: Option<String>,
    pub organization: Option<String>,
    pub plan: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubAccount {
    pub host: String,
    pub login: String,
    pub active: bool,
    /// False when the stored token was rejected.
    pub valid: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AzureAccount {
    pub user: String,
    pub subscription: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Accounts {
    pub claude: Status<ClaudeAccount>,
    pub github: Status<Vec<GithubAccount>>,
    pub azure: Status<AzureAccount>,
}

/// Ask each tool in parallel. `guardian` is the process guardian binary that
/// Linux needs; Windows ignores it.
///
///  Never fails: a missing or broken tool is a
/// status of its own.
#[must_use]
pub fn collect(guardian: PathBuf) -> Accounts {
    let runner = &Runner::new(guardian);
    let path = env::var_os("PATH").unwrap_or_default();
    let parent: Vec<(OsString, OsString)> = env::vars_os().collect();
    let ask = |launch: Option<Launch>| lookup(runner, &parent, launch);
    thread::scope(|scope| {
        let claude = scope.spawn(|| {
            let launch = on_path(
                &path,
                &["claude.exe", "claude"],
                &["auth", "status", "--json"],
            );
            finish(ask(launch), parse_claude)
        });
        let github = scope.spawn(|| {
            let launch = on_path(
                &path,
                &["gh.exe", "gh"],
                &["auth", "status", "--json", "hosts"],
            );
            finish(ask(launch), parse_github)
        });
        let azure = scope.spawn(|| finish(ask(azure_launch(&path)), parse_azure));
        Accounts {
            claude: claude.join().unwrap_or_else(|_| panicked()),
            github: github.join().unwrap_or_else(|_| panicked()),
            azure: azure.join().unwrap_or_else(|_| panicked()),
        }
    })
}

fn panicked<T>() -> Status<T> {
    Status::Failed("the lookup panicked".into())
}

enum Lookup {
    Output(String),
    Missing,
    SignedOut,
    Failed(String),
}

fn finish<T>(lookup: Lookup, parse: impl FnOnce(&str) -> Status<T>) -> Status<T> {
    match lookup {
        Lookup::Output(text) => parse(&text),
        Lookup::Missing => Status::NotInstalled,
        Lookup::SignedOut => Status::SignedOut,
        Lookup::Failed(message) => Status::Failed(message),
    }
}

/// A program, its arguments and any environment it needs beyond the allowlist.
struct Launch {
    program: PathBuf,
    args: Vec<OsString>,
    extra_env: Vec<(OsString, OsString)>,
}

fn lookup(runner: &Runner, parent: &[(OsString, OsString)], launch: Option<Launch>) -> Lookup {
    let Some(launch) = launch else {
        return Lookup::Missing;
    };
    let env = parent
        .iter()
        .filter(|(name, _)| {
            ENV_ALLOWED.iter().any(|allowed| {
                name.to_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(allowed))
            })
        })
        .cloned()
        .chain(launch.extra_env)
        .collect();
    let spec = CommandSpec {
        program: launch.program,
        args: launch.args,
        env,
        cwd: None,
    };
    let cancel = AtomicBool::new(false);
    let output = match runner.run(&spec, &[], LIMITS, &cancel) {
        Ok(output) => output,
        Err(error) => return Lookup::Failed(clean(&error.to_string())),
    };
    match output.outcome {
        Outcome::Exited(status) if status.success() => {
            Lookup::Output(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Outcome::Exited(_) => {
            let text = String::from_utf8_lossy(&output.stderr).into_owned();
            // Each tool tells a signed-out user to log in, in those words.
            if text.to_ascii_lowercase().contains("login") {
                Lookup::SignedOut
            } else {
                Lookup::Failed(clean(&text))
            }
        }
        Outcome::TimedOut => Lookup::Failed("timed out".into()),
        Outcome::Cancelled | Outcome::OutputLimit => Lookup::Failed("did not finish".into()),
    }
}

fn find(path: &OsStr, names: &[&str]) -> Option<PathBuf> {
    // Names are in preference order, so the first directory is not enough.
    for name in names {
        for directory in env::split_paths(path) {
            let candidate = directory.join(name);
            if directory.is_absolute() && candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn on_path(path: &OsStr, names: &[&str], args: &[&str]) -> Option<Launch> {
    Some(Launch {
        program: find(path, names)?,
        args: args.iter().map(OsString::from).collect(),
        extra_env: Vec::new(),
    })
}

/// The Azure CLI on Windows is a `.cmd` file, and a shell would have to run
/// it. Its job is only to start the bundled Python, so start that directly:
/// no shell means no quoting trap and no way for text to be re-parsed.
fn azure_launch(path: &OsStr) -> Option<Launch> {
    let args = ["account", "show", "--output", "json"];
    if !cfg!(windows) {
        return on_path(path, &["az"], &args);
    }
    let script = find(path, &["az.cmd"])?;
    let python = script.parent()?.parent()?.join("python.exe");
    if !python.is_file() {
        return None;
    }
    let mut full: Vec<OsString> = ["-IBm", "azure.cli"].map(OsString::from).to_vec();
    full.extend(args.map(OsString::from));
    Some(Launch {
        program: python,
        args: full,
        // The script sets this before starting Python.
        extra_env: vec![("AZ_INSTALLER".into(), "MSI".into())],
    })
}

/// One short line, safe to print: tool output is not trusted text.
fn clean(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(SHOWN_CHARS)
        .collect()
}

fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(clean)
        .filter(|text| !text.is_empty())
}

#[must_use]
pub fn parse_claude(input: &str) -> Status<ClaudeAccount> {
    let Ok(value) = serde_json::from_str::<Value>(input) else {
        return Status::Failed("unreadable status".into());
    };
    if value.get("loggedIn").and_then(Value::as_bool) != Some(true) {
        return Status::SignedOut;
    }
    Status::SignedIn(ClaudeAccount {
        method: text(&value, "/authMethod").unwrap_or_else(|| "unknown".into()),
        email: text(&value, "/email"),
        organization: text(&value, "/orgName"),
        plan: text(&value, "/subscriptionType"),
    })
}

#[must_use]
pub fn parse_github(input: &str) -> Status<Vec<GithubAccount>> {
    let Ok(value) = serde_json::from_str::<Value>(input) else {
        return Status::Failed("unreadable status".into());
    };
    let mut accounts = Vec::new();
    if let Some(hosts) = value.get("hosts").and_then(Value::as_object) {
        for (host, entries) in hosts {
            for entry in entries.as_array().into_iter().flatten() {
                let Some(login) = text(entry, "/login") else {
                    continue;
                };
                accounts.push(GithubAccount {
                    host: clean(host),
                    login,
                    active: entry.get("active").and_then(Value::as_bool) == Some(true),
                    valid: entry.get("state").and_then(Value::as_str) == Some("success"),
                });
            }
        }
    }
    if accounts.is_empty() {
        Status::SignedOut
    } else {
        Status::SignedIn(accounts)
    }
}

#[must_use]
pub fn parse_azure(input: &str) -> Status<AzureAccount> {
    let Ok(value) = serde_json::from_str::<Value>(input) else {
        return Status::Failed("unreadable status".into());
    };
    match (text(&value, "/user/name"), text(&value, "/name")) {
        (Some(user), Some(subscription)) => Status::SignedIn(AzureAccount { user, subscription }),
        _ => Status::SignedOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shapes seen from the real tools on 2026-11-21, with synthetic values.
    const CLAUDE: &str = r#"{"loggedIn":true,"authMethod":"claude.ai","email":"a@example.test","orgName":"Org","subscriptionType":"pro","loginExpiresAt":"x"}"#;
    const GITHUB: &str = r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com","login":"one","tokenSource":"keyring","scopes":"repo"},{"state":"error","active":false,"host":"github.com","login":"two","tokenSource":"keyring"}]}}"#;
    const AZURE: &str = r#"{"environmentName":"AzureCloud","id":"sub-id","name":"dev-subscription","tenantId":"t","user":{"name":"me@example.test","type":"user"}}"#;

    #[test]
    fn claude_reports_the_method_and_who_is_signed_in() {
        let Status::SignedIn(account) = parse_claude(CLAUDE) else {
            panic!("expected a sign-in");
        };
        assert_eq!(account.method, "claude.ai");
        assert_eq!(account.email.as_deref(), Some("a@example.test"));
        assert_eq!(account.plan.as_deref(), Some("pro"));
        assert_eq!(parse_claude(r#"{"loggedIn":false}"#), Status::SignedOut);
        assert!(matches!(parse_claude("nope"), Status::Failed(_)));
    }

    // Invariant: every logged-in GitHub account is listed with its active flag,
    // since the owner switches accounts per prompt. Reading only the first or
    // the active one would hide which accounts are available.
    #[test]
    fn github_lists_every_account_and_marks_the_active_and_rejected_ones() {
        let Status::SignedIn(accounts) = parse_github(GITHUB) else {
            panic!("expected accounts");
        };
        assert_eq!(accounts.len(), 2);
        assert!(accounts[0].active && accounts[0].valid);
        assert!(!accounts[1].active && !accounts[1].valid);
        assert_eq!(parse_github(r#"{"hosts":{}}"#), Status::SignedOut);
    }

    #[test]
    fn azure_needs_both_a_user_and_a_subscription() {
        let Status::SignedIn(account) = parse_azure(AZURE) else {
            panic!("expected a sign-in");
        };
        assert_eq!(
            (account.user.as_str(), account.subscription.as_str()),
            ("me@example.test", "dev-subscription")
        );
        assert_eq!(
            parse_azure(r#"{"name":"only-subscription"}"#),
            Status::SignedOut
        );
    }

    // Invariant: nothing credential-shaped from the tools reaches the result,
    // and tool text cannot carry terminal escapes into the output.
    #[test]
    fn tokens_are_never_kept_and_text_is_flattened() {
        let noisy = r#"{"loggedIn":true,"authMethod":"api_key","apiKey":"sk-ant-secret","accessToken":"tok","email":"x\u001b[2Jy@example.test"}"#;
        let shown = format!("{:?}", parse_claude(noisy));
        assert!(!shown.contains("sk-ant") && !shown.contains("tok\""));
        assert!(!shown.contains('\u{1b}'));
        let long = format!(r#"{{"loggedIn":true,"email":"{}"}}"#, "x".repeat(500));
        let Status::SignedIn(account) = parse_claude(&long) else {
            panic!("expected a sign-in");
        };
        assert_eq!(account.email.unwrap().chars().count(), SHOWN_CHARS);
    }

    // Invariant: on Windows the Azure CLI is started as its bundled Python and
    // never through the .cmd file, so no shell parses the command line.
    #[cfg(windows)]
    #[test]
    fn azure_starts_the_bundled_python_and_not_the_cmd_file() {
        let root = std::env::temp_dir().join(format!("accounts-az-{}", std::process::id()));
        let wbin = root.join("CLI2 with space").join("wbin");
        std::fs::create_dir_all(&wbin).unwrap();
        std::fs::write(wbin.join("az.cmd"), "").unwrap();
        let path = std::env::join_paths([&wbin]).unwrap();
        // No python.exe beside it: nothing can be launched.
        assert!(azure_launch(&path).is_none());
        std::fs::write(wbin.parent().unwrap().join("python.exe"), "").unwrap();
        let launch = azure_launch(&path).unwrap();
        assert!(launch.program.ends_with("python.exe"));
        assert_eq!(
            launch.args[..2],
            [OsString::from("-IBm"), OsString::from("azure.cli")]
        );
        assert_eq!(
            launch.extra_env,
            [(OsString::from("AZ_INSTALLER"), OsString::from("MSI"))]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_tool_is_not_installed_and_the_preferred_name_wins() {
        let directory = std::env::temp_dir().join(format!("accounts-find-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("second"), "").unwrap();
        std::fs::write(directory.join("first"), "").unwrap();
        let path = std::env::join_paths([&directory]).unwrap();
        assert_eq!(
            find(&path, &["first", "second"]),
            Some(directory.join("first"))
        );
        assert_eq!(find(&path, &["absent"]), None);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
