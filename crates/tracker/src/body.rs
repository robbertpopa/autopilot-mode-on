use std::fmt::Write as _;

/// One check that ran, with how it ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckLine {
    pub name: String,
    pub passed: bool,
}

/// The pull request text, built by code from what happened. It lists only the
/// checks it is given, so it cannot claim one that did not run, and it never
/// holds a credential-shaped string.
#[must_use]
pub fn description(summary: &str, checks: &[CheckLine]) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{}\n", summary.trim());
    if checks.is_empty() {
        text.push_str("No checks ran before this was opened.\n");
    } else {
        text.push_str("Checks that ran:\n");
        for check in checks {
            let mark = if check.passed { "passed" } else { "failed" };
            let _ = writeln!(text, "- {}: {mark}", check.name);
        }
    }
    redact(&text)
}

const PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "sk-ant-",
    "sk-",
    "AKIA",
    "xoxb-",
    "xoxp-",
];

/// Replace anything that looks like a token or key with `[redacted]`. A word
/// counts when it starts with a known prefix and is long enough to be real,
/// and so does the value after `Bearer`.
#[must_use]
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut after_bearer = false;
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest
            .find(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '=' | '+' | '/'))
            })
            .unwrap_or(rest.len());
        if end == 0 {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let word = &rest[..end];
        let secret = word.len() >= 20
            && (after_bearer || PREFIXES.iter().any(|prefix| word.starts_with(prefix)));
        if secret {
            out.push_str("[redacted]");
        } else {
            out.push_str(word);
        }
        after_bearer = word.eq_ignore_ascii_case("bearer");
        rest = &rest[end..];
        // A space between "Bearer" and its value keeps the flag alive.
        if after_bearer {
            let trimmed = rest.trim_start_matches(' ');
            out.push_str(&rest[..rest.len() - trimmed.len()]);
            rest = trimmed;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: the description lists exactly the checks it was given, and
    // says so when there were none.
    #[test]
    fn the_description_states_only_checks_that_ran() {
        let text = description(
            "Adds the export.",
            &[
                CheckLine {
                    name: "build".into(),
                    passed: true,
                },
                CheckLine {
                    name: "test".into(),
                    passed: false,
                },
            ],
        );
        assert!(text.contains("- build: passed"));
        assert!(text.contains("- test: failed"));
        assert!(!text.contains("clippy"));
        assert!(description("x", &[]).contains("No checks ran"));
    }

    // Invariant: a token in a summary or a failure note never reaches GitHub.
    #[test]
    fn credential_shaped_text_is_redacted() {
        let token = format!("ghp_{}", "a".repeat(36));
        let text = redact(&format!(
            "use {token} here, Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123"
        ));
        assert!(!text.contains(&token), "{text}");
        assert!(!text.contains("abcdefghijklmnop"), "{text}");
        assert!(text.contains("use [redacted] here"));
        assert_eq!(
            redact("short sk-x and ordinary words"),
            "short sk-x and ordinary words"
        );
        assert_eq!(redact("héllo wörld ✓"), "héllo wörld ✓");
    }
}
