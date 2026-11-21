//! The deny rule behind the agent's `PreToolUse` hook.
//!
//! This is a tripwire, not containment: it stops the common ways an agent
//! would publish work (`git push`, `gh`), and it leans toward denying when a
//! command is hard to read. Keeping the token out of reach and the host as the
//! only publisher is what actually protects the remote.

use serde_json::Value;

const MAX_COMMAND: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
    Allow,
    Deny(String),
}

/// Git subcommands that send data to a remote.
const PUBLISHING: &[&str] = &[
    "push",
    "send-pack",
    "http-push",
    "remote-http",
    "remote-https",
];

/// Global options that take the next word as their value.
const WITH_VALUE: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--super-prefix",
    "--config-env",
];

/// Decide what to do with one tool call. `input` is the hook's standard input.
/// Anything that cannot be read is denied, because the hook only matters for a
/// call it can see.
#[must_use]
pub fn decide(input: &[u8]) -> Verdict {
    let Ok(value) = serde_json::from_slice::<Value>(input) else {
        return Verdict::Deny("the tool call could not be read".into());
    };
    let Some(tool) = value.get("tool_name").and_then(Value::as_str) else {
        return Verdict::Deny("the tool call has no tool name".into());
    };
    if tool != "Bash" {
        return Verdict::Allow;
    }
    let Some(command) = value
        .get("tool_input")
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
    else {
        return Verdict::Deny("the shell command could not be read".into());
    };
    match denied(command) {
        Some(reason) => Verdict::Deny(reason),
        None => Verdict::Allow,
    }
}

/// The JSON the hook prints. Claude Code reads the decision from it.
#[must_use]
pub fn response(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Allow => "{}".to_owned(),
        Verdict::Deny(reason) => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format!(
                    "{reason}. Publishing is done by the host, not by the agent."
                ),
            }
        })
        .to_string(),
    }
}

fn denied(command: &str) -> Option<String> {
    if command.len() > MAX_COMMAND {
        return Some("the command is too long to check".into());
    }
    // Quotes are not honoured when splitting: `$(...)` and backticks run even
    // inside double quotes, so a split inside quotes is the safe mistake.
    for segment in command.split([';', '&', '|', '\n', '\r', '(', ')', '`']) {
        if let Some(reason) = denied_segment(segment) {
            return Some(reason);
        }
    }
    None
}

/// The program names a word could spell once quotes and escapes are gone.
fn spellings(word: &str) -> Vec<String> {
    let bare: String = word
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | '^'))
        .collect();
    let without_backslashes = bare.replace('\\', "");
    [bare, without_backslashes]
        .into_iter()
        .map(|text| {
            let name = text.rsplit(['/', '\\']).next().unwrap_or_default();
            let name = name.to_ascii_lowercase();
            name.strip_suffix(".exe")
                .map_or(name.clone(), str::to_owned)
        })
        .collect()
}

fn is_program(word: &str, program: &str) -> bool {
    spellings(word).iter().any(|name| name == program)
}

fn denied_segment(segment: &str) -> Option<String> {
    let words = tokens(segment);
    for (at, word) in words.iter().enumerate() {
        // A quoted word with spaces is often a whole command for a shell
        // (`bash -c "git push"`). It is always shorter than its segment, so
        // looking inside it ends.
        if word.contains(char::is_whitespace)
            && let Some(reason) = denied(word)
        {
            return Some(reason);
        }
        if is_program(word, "gh") || is_program(word, "hub") {
            return Some("the GitHub CLI is not available to the agent".into());
        }
        if is_program(word, "git")
            && let Some(reason) = denied_git(&words[at + 1..])
        {
            return Some(reason);
        }
    }
    None
}

/// Split on whitespace outside quotes and drop the quote characters, so
/// `-C "some dir"` is one value and `g"i"t` is `git`.
fn tokens(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut open: Option<char> = None;
    let mut started = false;
    for c in segment.chars() {
        match (open, c) {
            (Some(quote), c) if c == quote => open = None,
            (None, '"' | '\'') => {
                open = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (_, c) => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

fn clean(word: &str) -> String {
    word.chars()
        .filter(|c| !matches!(c, '"' | '\'' | '^' | '\\'))
        .collect()
}

fn denied_git(args: &[String]) -> Option<String> {
    let mut rest = args.iter().map(|word| clean(word));
    while let Some(word) = rest.next() {
        let attached_alias = word.starts_with("-c") && word[2..].starts_with("alias.");
        if attached_alias || word.starts_with("--config-env=alias.") {
            return Some("a git alias could hide a push".into());
        }
        if WITH_VALUE.contains(&word.as_str()) {
            let value = rest.next().unwrap_or_default();
            if matches!(word.as_str(), "-c" | "--config-env")
                && value.to_ascii_lowercase().starts_with("alias.")
            {
                return Some("a git alias could hide a push".into());
            }
            continue;
        }
        if word.starts_with('-') {
            continue;
        }
        let lowered = word.to_ascii_lowercase();
        return PUBLISHING
            .contains(&lowered.as_str())
            .then(|| format!("git {lowered} is not allowed"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(command: &str) -> Vec<u8> {
        serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}})
            .to_string()
            .into_bytes()
    }

    fn denies(command: &str) -> bool {
        matches!(decide(&bash(command)), Verdict::Deny(_))
    }

    // Invariant: every common way to spell a push is caught. Each spelling is
    // a separate parsing path (global options, chains, wrappers, quoting), so
    // dropping one would reopen exactly that path.
    #[test]
    fn spellings_of_a_push_are_denied() {
        for command in [
            "git push",
            "git push origin main",
            "git push --force",
            "git -C . push",
            "git -C \"some dir\" push",
            "git -c k=v push",
            "git -ck=v push",
            "git --git-dir=.git push",
            "git --no-pager push",
            "git -c alias.p=push p",
            "git -calias.p=push p",
            "git.exe push",
            "GIT.EXE PUSH",
            "C:\\Program Files\\Git\\cmd\\git.exe push",
            "/usr/bin/git push",
            "\"git\" push",
            "g\"i\"t push",
            "'git' 'push'",
            "git pu\\sh",
            "cd a && git push",
            "cd a; git push",
            "echo hi | git push",
            "echo hi\ngit push",
            "true || git push",
            "(git push)",
            "echo $(git push)",
            "echo `git push`",
            "bash -c \"git push\"",
            "sh -c 'git push origin'",
            "pwsh -Command \"git push\"",
            "cmd /c git push",
            "env GIT_TRACE=1 git push",
            "time git push",
            "git send-pack origin",
            "gh pr create",
            "gh auth token",
            "C:\\tools\\gh.exe repo view",
        ] {
            assert!(denies(command), "{command:?} must be denied");
        }
    }

    #[test]
    fn ordinary_work_is_allowed() {
        for command in [
            "git status",
            "git diff --stat",
            "git -C . log --oneline",
            "git commit -m \"fix the parser\"",
            "git add -A",
            "cargo test",
            "ls -la",
            "echo hello",
            "grep -rn pusher src",
            "git checkout -b feature/x",
        ] {
            assert!(!denies(command), "{command:?} must be allowed");
        }
    }

    #[test]
    fn only_shell_calls_are_inspected() {
        let write = serde_json::json!({"tool_name": "Write", "tool_input": {"file_path": "a"}});
        assert_eq!(decide(write.to_string().as_bytes()), Verdict::Allow);
    }

    // Invariant: input the hook cannot read is denied and never panics. The
    // sweep covers truncation at every byte, wrong shapes and a huge command.
    #[test]
    fn unreadable_input_is_denied_without_panicking() {
        let good = bash("git status");
        for cut in 0..good.len() {
            let _ = decide(&good[..cut]);
        }
        assert!(matches!(decide(&good[..good.len() - 1]), Verdict::Deny(_)));
        for bad in [
            &b""[..],
            b"null",
            b"[]",
            b"{\"tool_name\":\"Bash\"}",
            b"{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":7}}",
            b"\xff\xfe\x00",
        ] {
            assert!(matches!(decide(bad), Verdict::Deny(_)), "{bad:?}");
        }
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        for _ in 0..200 {
            let bytes: Vec<u8> = (0..64)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed.to_be_bytes()[0]
                })
                .collect();
            let _ = decide(&bytes);
        }
        assert!(denies(&"a ".repeat(MAX_COMMAND)));
    }

    #[test]
    fn the_response_names_the_decision_and_stays_valid_json() {
        let deny: Value = serde_json::from_str(&response(&Verdict::Deny("no".into()))).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(response(&Verdict::Allow), "{}");
    }
}
