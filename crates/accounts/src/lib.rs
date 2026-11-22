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
pub struct CodexAccount {
    /// `ChatGPT subscription` or `API key`.
    pub method: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Accounts {
    pub claude: Status<ClaudeAccount>,
    pub codex: Status<CodexAccount>,
    pub github: Status<Vec<GithubAccount>>,
    pub azure: Status<AzureAccount>,
    /// Every Azure subscription the CLI knows, so the owner can switch.
    pub azure_all: Vec<AzureChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AzureChoice {
    pub id: String,
    pub user: String,
    pub subscription: String,
    pub active: bool,
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
        let codex = scope.spawn(|| {
            let launch = on_path(&path, &["codex.exe", "codex"], &["login", "status"]);
            finish(ask(launch), parse_codex)
        });
        let github = scope.spawn(|| {
            let launch = on_path(
                &path,
                &["gh.exe", "gh"],
                &["auth", "status", "--json", "hosts"],
            );
            finish(ask(launch), parse_github)
        });
        let azure = scope.spawn(|| finish(ask(azure_launch(&path, &SHOW)), parse_azure));
        let azure_all = scope.spawn(|| match ask(azure_launch(&path, &LIST)) {
            Lookup::Output(text) => parse_azure_list(&text),
            _ => Vec::new(),
        });
        Accounts {
            claude: claude.join().unwrap_or_else(|_| panicked()),
            codex: codex.join().unwrap_or_else(|_| panicked()),
            github: github.join().unwrap_or_else(|_| panicked()),
            azure: azure.join().unwrap_or_else(|_| panicked()),
            azure_all: azure_all.join().unwrap_or_default(),
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
            // Some tools print a plain status line on stderr instead.
            let from = if output.stdout.is_empty() {
                &output.stderr
            } else {
                &output.stdout
            };
            Lookup::Output(String::from_utf8_lossy(from).into_owned())
        }
        Outcome::Exited(_) => {
            let text = String::from_utf8_lossy(&output.stderr).into_owned();
            // Each tool tells a signed-out user to log in, in those words.
            let lower = text.to_ascii_lowercase();
            if lower.contains("login") || lower.contains("not logged in") {
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

const SHOW: [&str; 4] = ["account", "show", "--output", "json"];
const LIST: [&str; 4] = ["account", "list", "--output", "json"];

/// The Azure CLI on Windows is a `.cmd` file, and a shell would have to run
/// it. Its job is only to start the bundled Python, so start that directly:
/// no shell means no quoting trap and no way for text to be re-parsed.
fn azure_launch(path: &OsStr, args: &[&str]) -> Option<Launch> {
    if !cfg!(windows) {
        return on_path(path, &["az"], args);
    }
    let script = find(path, &["az.cmd"])?;
    let python = script.parent()?.parent()?.join("python.exe");
    if !python.is_file() {
        return None;
    }
    let mut full: Vec<OsString> = ["-IBm", "azure.cli"].map(OsString::from).to_vec();
    full.extend(args.iter().map(OsString::from));
    Some(Launch {
        program: python,
        args: full,
        // The script sets this before starting Python.
        extra_env: vec![("AZ_INSTALLER".into(), "MSI".into())],
    })
}

/// A name the tools gave us is passed as one argument, never through a shell,
/// but a leading dash would still read as an option.
fn plain_argument(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('-') && !value.chars().any(char::is_control)
}

/// Make another logged-in GitHub account the active one. Both values come
/// from the account list, and only the CLI's own switch command runs.
///
/// # Errors
/// Returns a short reason when the CLI is missing, refuses, or the values
/// are not plain names.
pub fn switch_github(guardian: PathBuf, host: &str, login: &str) -> Result<(), String> {
    if !plain_argument(host) || !plain_argument(login) {
        return Err("not a plain account name".into());
    }
    let path = env::var_os("PATH").unwrap_or_default();
    let args = ["auth", "switch", "--hostname", host, "--user", login];
    run_change(guardian, on_path(&path, &["gh.exe", "gh"], &args))
}

/// Make another Azure subscription the default one.
///
/// # Errors
/// Same as [`switch_github`].
pub fn switch_azure(guardian: PathBuf, subscription_id: &str) -> Result<(), String> {
    if !plain_argument(subscription_id) {
        return Err("not a plain subscription id".into());
    }
    let path = env::var_os("PATH").unwrap_or_default();
    let args = ["account", "set", "--subscription", subscription_id];
    run_change(guardian, azure_launch(&path, &args))
}

fn run_change(guardian: PathBuf, launch: Option<Launch>) -> Result<(), String> {
    let runner = Runner::new(guardian);
    let parent: Vec<(OsString, OsString)> = env::vars_os().collect();
    match lookup(&runner, &parent, launch) {
        Lookup::Output(_) => Ok(()),
        Lookup::Missing => Err("the tool is not installed".into()),
        Lookup::SignedOut => Err("not signed in".into()),
        Lookup::Failed(reason) => Err(reason),
    }
}

/// Whether a variable is saved in the Windows user environment. `setx` writes
/// there, and a program that is already running never sees it, so the window
/// uses this to say "restart the app" instead of "not set". Only presence is
/// reported; the value is dropped here and never returned.
#[must_use]
pub fn saved_in_user_environment(guardian: PathBuf, name: &str) -> bool {
    if !cfg!(windows) || !plain_argument(name) {
        return false;
    }
    let root = env::var_os("SYSTEMROOT").unwrap_or_default();
    let reg = PathBuf::from(root).join("System32").join("reg.exe");
    if !reg.is_file() {
        return false;
    }
    let launch = Launch {
        program: reg,
        args: ["query", r"HKCU\Environment", "/v", name]
            .map(OsString::from)
            .to_vec(),
        extra_env: Vec::new(),
    };
    let runner = Runner::new(guardian);
    let parent: Vec<(OsString, OsString)> = env::vars_os().collect();
    match lookup(&runner, &parent, Some(launch)) {
        Lookup::Output(text) => reg_value_present(&text, name),
        _ => false,
    }
}

/// `reg query` prints `    NAME    REG_SZ    value`; true when the value is not empty.
fn reg_value_present(output: &str, name: &str) -> bool {
    output.lines().any(|line| {
        let mut parts = line.split_whitespace();
        parts.next().is_some_and(|n| n.eq_ignore_ascii_case(name))
            && parts.next().is_some_and(|kind| kind.starts_with("REG_"))
            && parts.next().is_some()
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

/// `codex login status` says `Logged in using ChatGPT` or `Logged in using an API key`.
#[must_use]
pub fn parse_codex(input: &str) -> Status<CodexAccount> {
    let lower = input.to_ascii_lowercase();
    if lower.contains("not logged in") {
        Status::SignedOut
    } else if lower.contains("chatgpt") {
        Status::SignedIn(CodexAccount {
            method: "ChatGPT subscription".into(),
        })
    } else if lower.contains("api key") {
        Status::SignedIn(CodexAccount {
            method: "API key".into(),
        })
    } else if lower.contains("logged in") {
        Status::SignedIn(CodexAccount {
            method: "signed in".into(),
        })
    } else {
        Status::Failed(clean(input))
    }
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

/// Every subscription from `az account list`, with the signed-in user.
#[must_use]
pub fn parse_azure_list(input: &str) -> Vec<AzureChoice> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(input) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(AzureChoice {
                id: text(item, "/id")?,
                user: text(item, "/user/name")?,
                subscription: text(item, "/name")?,
                active: item.get("isDefault").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect()
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
    fn a_saved_variable_is_found_only_when_it_has_a_value() {
        let set = "    AUTOPILOT_ANTHROPIC_API_KEY    REG_SZ    sk-test
";
        let empty = "    AUTOPILOT_ANTHROPIC_API_KEY    REG_SZ    
";
        assert!(reg_value_present(set, "AUTOPILOT_ANTHROPIC_API_KEY"));
        assert!(!reg_value_present(empty, "AUTOPILOT_ANTHROPIC_API_KEY"));
        assert!(!reg_value_present(set, "OTHER"));
    }

    #[test]
    fn azure_subscriptions_are_listed_with_the_default_marked() {
        let list = r#"[{"id":"a","name":"one","isDefault":true,"user":{"name":"me@example.test"}},{"id":"b","name":"two","isDefault":false,"user":{"name":"you@example.test"}}]"#;
        let all = parse_azure_list(list);
        assert_eq!(all.len(), 2);
        assert!(all[0].active && !all[1].active);
        assert_eq!(parse_azure_list("nope").len(), 0);
    }

    #[test]
    fn a_switch_refuses_a_name_that_looks_like_an_option() {
        assert!(switch_github(PathBuf::new(), "github.com", "--token").is_err());
        assert!(switch_azure(PathBuf::new(), "-x").is_err());
    }

    #[test]
    fn codex_reports_how_it_is_signed_in_and_when_it_is_not() {
        let method = |text: &str| match parse_codex(text) {
            Status::SignedIn(account) => account.method,
            other => format!("{other:?}"),
        };
        assert_eq!(
            method(
                "Logged in using ChatGPT
"
            ),
            "ChatGPT subscription"
        );
        assert_eq!(method("Logged in using an API key - sk-***"), "API key");
        assert_eq!(parse_codex("Not logged in"), Status::SignedOut);
        assert!(matches!(parse_codex("boom"), Status::Failed(_)));
    }

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
        assert!(azure_launch(&path, &SHOW).is_none());
        std::fs::write(wbin.parent().unwrap().join("python.exe"), "").unwrap();
        let launch = azure_launch(&path, &SHOW).unwrap();
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
