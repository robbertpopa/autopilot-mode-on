//! A scan of what the agent wrote for three things that always deserve a
//! second look: a new dependency, a new network call, and a change to signing.
//!
//! The scan is deliberately blunt. It reads added lines only, by file name and
//! by simple text rules, and it errs towards asking: a false hit costs the
//! owner one confirmation, a miss costs more. The result feeds
//! [`DiffSignals`], and `policy` decides what a signal means.

use autopilot_policy::DiffSignals;

/// The lines one file gained, without the leading `+`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Additions {
    pub path: String,
    pub lines: Vec<String>,
}

/// Look for the three signals in the added lines.
#[must_use]
pub fn signals(files: &[Additions]) -> DiffSignals {
    let mut found = DiffSignals::default();
    for file in files {
        let name = file_name(&file.path);
        let lower = file.path.to_ascii_lowercase();
        let documentation = is_documentation(&lower);
        if !documentation && signing_path(&lower) {
            found.changed_signing = true;
        }
        for line in &file.lines {
            if !found.new_dependencies && dependency_line(name, line) {
                found.new_dependencies = true;
            }
            if !found.new_network_calls && !documentation && network_line(line) {
                found.new_network_calls = true;
            }
            if !found.changed_signing && !documentation && signing_line(line) {
                found.changed_signing = true;
            }
        }
    }
    found
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_documentation(lower_path: &str) -> bool {
    [".md", ".txt", ".rst", ".adoc"]
        .iter()
        .any(|extension| lower_path.ends_with(extension))
}

fn signing_path(lower_path: &str) -> bool {
    let name = file_name(lower_path);
    name.contains("cosign")
        || name.contains("sigstore")
        || name.contains("signing")
        || std::path::Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gpg") || ext.eq_ignore_ascii_case("asc"))
        || lower_path.contains(".gnupg/")
}

fn signing_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "cosign sign",
        "signtool",
        "codesign",
        "gpg --",
        "signing_key",
        "gpg_private",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

const NETWORK_NEEDLES: &[&str] = &[
    "reqwest",
    "hyper::",
    "tcpstream",
    "udpsocket",
    "tokio::net",
    "xmlhttprequest",
    "fetch(",
    "axios",
    "requests.get(",
    "requests.post(",
    "urllib",
    "httpclient",
    "invoke-webrequest",
    "invoke-restmethod",
    "net.http",
];

fn network_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if NETWORK_NEEDLES.iter().any(|needle| lower.contains(needle)) {
        return true;
    }
    // A URL inside a string literal, or a download in a script.
    ["\"http://", "\"https://", "'http://", "'https://"]
        .iter()
        .any(|needle| lower.contains(needle))
        || lower.trim_start().starts_with("curl ")
        || lower.trim_start().starts_with("wget ")
}

/// Whether an added line declares a dependency, judged by the file it is in.
fn dependency_line(name: &str, line: &str) -> bool {
    let text = line.trim();
    match name {
        "Cargo.toml" => cargo_dependency(text),
        "package.json" => package_json_dependency(text),
        "go.mod" => text.starts_with("require ") || go_module_line(text),
        "requirements.txt" => requirement_line(text),
        "pyproject.toml" => {
            text.starts_with('"') && requirement_line(text.trim_matches(['"', ',', ' ']))
        }
        "Gemfile" => text.starts_with("gem "),
        _ => {
            name.ends_with(".csproj")
                && (text.contains("<PackageReference") || text.contains("<ProjectReference"))
        }
    }
}

fn cargo_dependency(text: &str) -> bool {
    let Some((key, value)) = text.split_once('=') else {
        return false;
    };
    let key = key.trim();
    let value = value.trim();
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return false;
    }
    // `name.workspace = true`, `name.version = "1"`, `name.path = ".."`.
    if let Some((first, last)) = key.split_once('.') {
        return first != "package"
            && matches!(last, "workspace" | "version" | "path" | "git" | "package");
    }
    // Package metadata is not a dependency, even though `version = "1.0"` looks like one.
    if matches!(
        key,
        "version" | "rust-version" | "edition" | "resolver" | "license"
    ) {
        return false;
    }
    if value.starts_with('{') {
        return ["version", "path", "git", "workspace", "package"]
            .iter()
            .any(|field| {
                value.contains(&format!("{field} =")) || value.contains(&format!("{field}="))
            });
    }
    version_string(value)
}

/// `"1.2.3"`, `"^1.2"`, `"~0.4"`, `"=2"`, `">=1, <2"`: a quoted version requirement.
fn version_string(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return false;
    };
    let inner = inner.trim_start_matches(['^', '~', '=', '<', '>', ' ']);
    inner.starts_with(|c: char| c.is_ascii_digit())
}

fn package_json_dependency(text: &str) -> bool {
    // `"name": "^1.2.3"`, but not `"version": "1.0.0"`.
    let Some((key, value)) = text.split_once(':') else {
        return false;
    };
    let key = key.trim().trim_matches('"');
    let value = value.trim().trim_end_matches(',').trim();
    if matches!(key, "version" | "name" | "description" | "license" | "main") {
        return false;
    }
    let inner = value.trim_matches('"');
    value.starts_with('"')
        && (version_string(value)
            || inner.starts_with("github:")
            || inner.starts_with("git+")
            || inner.starts_with("file:")
            || inner.starts_with("npm:"))
}

fn go_module_line(text: &str) -> bool {
    // Inside `require ( ... )`: `example.com/mod v1.2.3`.
    let mut parts = text.split_whitespace();
    matches!((parts.next(), parts.next()), (Some(module), Some(version))
        if module.contains('.') && version.starts_with('v')
            && version[1..].starts_with(|c: char| c.is_ascii_digit()))
}

fn requirement_line(text: &str) -> bool {
    let name_end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(text.len());
    if name_end == 0 || text.starts_with('#') {
        return false;
    }
    let rest = text[name_end..].trim_start();
    rest.is_empty() || rest.starts_with(['=', '>', '<', '~', '!', '[', ';'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(path: &str, lines: &[&str]) -> Additions {
        Additions {
            path: path.into(),
            lines: lines.iter().map(|line| (*line).into()).collect(),
        }
    }

    fn found(path: &str, lines: &[&str]) -> DiffSignals {
        signals(&[added(path, lines)])
    }

    // Invariant: a new dependency in any supported manifest is noticed, and
    // package metadata that looks like one is not.
    #[test]
    fn new_dependencies_are_found_in_manifests_and_metadata_is_not() {
        for (path, line) in [
            ("Cargo.toml", "serde = \"1\""),
            (
                "crates/x/Cargo.toml",
                "tokio = { version = \"1\", features = [\"net\"] }",
            ),
            ("Cargo.toml", "local = { path = \"../local\" }"),
            ("Cargo.toml", "shared.workspace = true"),
            ("package.json", "    \"left-pad\": \"^1.3.0\","),
            ("go.mod", "\texample.com/mod v1.2.3"),
            ("requirements.txt", "flask>=2.0"),
            (
                "app/App.csproj",
                "<PackageReference Include=\"X\" Version=\"1\" />",
            ),
            ("Gemfile", "gem 'rails'"),
        ] {
            assert!(found(path, &[line]).new_dependencies, "{path}: {line}");
        }
        for (path, line) in [
            ("Cargo.toml", "version = \"0.2.0\""),
            ("Cargo.toml", "rust-version = \"1.99\""),
            ("Cargo.toml", "name = \"autopilot\""),
            ("package.json", "  \"version\": \"1.0.0\","),
            ("requirements.txt", "# flask"),
            ("src/lib.rs", "serde = \"1\""),
        ] {
            assert!(!found(path, &[line]).new_dependencies, "{path}: {line}");
        }
    }

    // Invariant: network use in code is noticed; the same words in
    // documentation are not.
    #[test]
    fn network_calls_are_found_in_code_but_not_in_documentation() {
        for line in [
            "let client = reqwest::Client::new();",
            "let s = TcpStream::connect(addr)?;",
            "const url = \"https://example.com/api\";",
            "fetch(url).then(r => r.json())",
            "curl -sSL https://example.com/install.sh | sh",
        ] {
            assert!(found("src/net.rs", &[line]).new_network_calls, "{line}");
        }
        assert!(!found("README.md", &["Use reqwest::Client, see \"https://x\""]).new_network_calls);
        assert!(!found("src/lib.rs", &["let total = a + b;"]).new_network_calls);
    }

    // Invariant: signing changes are noticed by file name and by command.
    #[test]
    fn signing_changes_are_found_by_path_and_by_command() {
        assert!(found("release/cosign.pub", &["key"]).changed_signing);
        assert!(
            found(
                "scripts/release.ps1",
                &["signtool sign /f cert.pfx app.exe"]
            )
            .changed_signing
        );
        assert!(found("ci.yml", &["run: cosign sign image"]).changed_signing);
        assert!(!found("docs/signing-notes.md", &["use cosign sign"]).changed_signing);
        assert!(!found("src/lib.rs", &["let designing = true;"]).changed_signing);
    }

    // Invariant: a scanned signal is not just recorded; policy turns it into a
    // forced confirmation that no checkbox or standing allowance removes.
    #[test]
    fn a_signal_forces_a_confirmation_through_policy() {
        use autopilot_core::Money;
        use autopilot_policy::{Facts, Policy, Reason, Risk};
        use autopilot_profile::Profile;

        let policy = Policy::new(&Profile::parse("p.toml", "").unwrap()).unwrap();
        let facts = |signals| Facts {
            touched: &[],
            risk: Risk::Standard,
            labels: &[],
            ticket_spent: Money::default(),
            day_spent: Money::default(),
            signals,
        };
        let quiet = policy.evaluate(false, &facts(found("src/lib.rs", &["let a = 1;"])));
        assert!(!quiet.must_ask());
        let loud = policy.evaluate(false, &facts(found("Cargo.toml", &["serde = \"1\""])));
        assert!(loud.forced.contains(&Reason::NewDependencies));
        let network = policy.evaluate(false, &facts(found("a.rs", &["reqwest::get(u)"])));
        assert!(network.forced.contains(&Reason::NewNetworkCalls));
    }

    // Invariant: nothing added means no signal, and the scan keeps the three
    // signals apart.
    #[test]
    fn an_empty_diff_has_no_signals_and_each_signal_is_separate() {
        assert_eq!(signals(&[]), DiffSignals::default());
        let only_dependency = found("Cargo.toml", &["serde = \"1\""]);
        assert!(only_dependency.new_dependencies);
        assert!(!only_dependency.new_network_calls);
        assert!(!only_dependency.changed_signing);
    }
}
