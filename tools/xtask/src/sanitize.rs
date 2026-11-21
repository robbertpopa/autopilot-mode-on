//! Turns a raw Claude Code capture into a fixture that is safe to commit.
//!
//! The rules are deliberately blunt: anything that could identify a person, a
//! machine or a session is replaced, and the output is refused if a scan still
//! finds something. A fixture that loses detail can be recaptured; one that
//! leaks cannot be taken back from git history.

use std::collections::HashMap;

use serde_json::Value;

const MIN_LOCAL_LEN: usize = 3;

pub(crate) struct Sanitizer {
    ids: HashMap<String, String>,
    local: Vec<String>,
}

impl Sanitizer {
    /// `local` holds strings that must never survive, such as the home
    /// directory and user name of the machine that made the capture.
    pub(crate) fn new(local: &[String]) -> Self {
        Self {
            ids: HashMap::new(),
            local: local
                .iter()
                .filter(|value| value.len() >= MIN_LOCAL_LEN)
                .cloned()
                .collect(),
        }
    }

    /// Sanitize a whole JSON Lines capture.
    ///
    /// # Errors
    /// Returns an error naming the line if it is not JSON, or if the scan finds
    /// a leftover secret, address or local name.
    pub(crate) fn run(&mut self, raw: &str) -> Result<String, String> {
        let mut out = String::new();
        for (number, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let mut value: Value = serde_json::from_str(line)
                .map_err(|error| format!("line {} is not JSON: {error}", number + 1))?;
            self.clean(None, &mut value);
            out.push_str(&value.to_string());
            out.push('\n');
        }
        self.scan(&out)?;
        Ok(out)
    }

    fn clean(&mut self, key: Option<&str>, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (name, child) in map.iter_mut() {
                    if is_environment_inventory(name) {
                        // The count is not interesting and the names describe one
                        // machine, so the whole value goes.
                        match child {
                            Value::Array(items) => items.clear(),
                            Value::Object(entries) => entries.clear(),
                            _ => self.clean(Some(name), child),
                        }
                    } else {
                        self.clean(Some(name), child);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.clean(key, item);
                }
            }
            Value::String(text) => {
                if let Some(replacement) = self.replace_by_key(key, text) {
                    *text = replacement;
                } else {
                    *text = self.generic(text);
                }
            }
            _ => {}
        }
    }

    fn replace_by_key(&mut self, key: Option<&str>, text: &str) -> Option<String> {
        let key = key?.to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "session_id"
                | "sessionid"
                | "uuid"
                | "tool_use_id"
                | "parent_tool_use_id"
                | "id"
                | "request_id"
        ) {
            return Some(self.alias(text));
        }
        // Names the credential source, such as ANTHROPIC_API_KEY, never its value.
        if key == "apikeysource" {
            return None;
        }
        if [
            "token", "secret", "password", "email", "org", "account", "api_key", "apikey",
        ]
        .iter()
        .any(|needle| key.contains(needle))
        {
            return Some("<redacted>".into());
        }
        if key == "cwd"
            || key.ends_with("directory")
            || key.ends_with("dir")
            || key.ends_with("path")
        {
            return Some("<path>".into());
        }
        // Model prose is arbitrary text from a prompt we do not control. The
        // parser only needs it to be present, so its length is all that stays.
        if matches!(
            key.as_str(),
            "text" | "thinking" | "signature" | "result" | "content"
        ) {
            return Some(format!("<text {} chars>", text.chars().count()));
        }
        None
    }

    /// The same input always maps to the same alias, so a resumed session still
    /// shows one identifier across frames.
    fn alias(&mut self, original: &str) -> String {
        let next = self.ids.len() + 1;
        self.ids
            .entry(original.to_owned())
            .or_insert_with(|| {
                if original.len() == 36 && original.matches('-').count() == 4 {
                    format!("00000000-0000-4000-8000-{next:012}")
                } else {
                    format!("id-{next}")
                }
            })
            .clone()
    }

    fn generic(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for local in &self.local {
            text = replace_ignore_case(&text, local, "<local>");
        }
        text = redact_keys(&text);
        if looks_like_path(&text) {
            return "<path>".into();
        }
        text
    }

    fn scan(&self, output: &str) -> Result<(), String> {
        let lower = output.to_lowercase();
        if lower.contains("sk-ant-") {
            return Err("an Anthropic key pattern survived".into());
        }
        for local in &self.local {
            if lower.contains(&local.to_lowercase()) {
                return Err("a local name survived".into());
            }
        }
        if has_email(output) {
            return Err("an email address survived".into());
        }
        Ok(())
    }
}

/// Init-frame fields that list what is installed on the capturing machine.
fn is_environment_inventory(key: &str) -> bool {
    matches!(
        key,
        "plugins"
            | "skills"
            | "slash_commands"
            | "terminal_slash_commands"
            | "agents"
            | "mcp_servers"
            | "memory_paths"
            | "capabilities"
            | "additional_directories"
            // Keyed by provider tool-call ids and repeats inputs seen elsewhere.
            | "wire_tool_inputs"
    )
}

fn replace_ignore_case(text: &str, needle: &str, with: &str) -> String {
    let (lower, needle) = (text.to_lowercase(), needle.to_lowercase());
    // Lowercasing can change byte lengths for some scripts. Skip rather than
    // slice at a wrong offset; the final scan then refuses the output.
    if lower.len() != text.len() {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find(&needle) {
        let start = from + found;
        out.push_str(&text[from..start]);
        out.push_str(with);
        from = start + needle.len();
    }
    out.push_str(&text[from..]);
    out
}

fn redact_keys(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("sk-ant-") {
        out.push_str(&rest[..at]);
        out.push_str("<redacted>");
        let tail = &rest[at..];
        let len = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        rest = &tail[len..];
    }
    out.push_str(rest);
    out
}

fn looks_like_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    let drive = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unix = ["/home/", "/Users/", "/tmp/", "/var/", "/root/"]
        .iter()
        .any(|p| text.starts_with(p));
    drive || unix
}

fn has_email(text: &str) -> bool {
    text.match_indices('@').any(|(at, _)| {
        let before = text[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after = &text[at + 1..];
        let domain: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
            .collect();
        before && domain.contains('.') && !domain.ends_with('.')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sanitize(raw: &str) -> Result<String, String> {
        Sanitizer::new(&["alice".into(), "C:\\Users\\alice".into()]).run(raw)
    }

    #[test]
    fn identifiers_map_consistently_and_differ_per_original() {
        let raw = concat!(
            "{\"type\":\"system\",\"session_id\":\"11111111-2222-3333-4444-555555555555\"}\n",
            "{\"type\":\"result\",\"session_id\":\"11111111-2222-3333-4444-555555555555\"}\n",
            "{\"type\":\"result\",\"session_id\":\"99999999-2222-3333-4444-555555555555\"}\n"
        );
        let out = sanitize(raw).unwrap();
        let ids: Vec<_> = out
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap()["session_id"].to_string())
            .collect();
        assert_eq!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
        assert!(!out.contains("11111111"));
    }

    // Invariant: structured output is kept, since the parser tests need it,
    // while prose, paths, keys, addresses and local names are not.
    #[test]
    fn it_keeps_structure_and_removes_identifying_content() {
        let raw = r#"{"type":"result","result":"private words","cwd":"C:\\Users\\alice\\repo","structured_output":{"summary":"Fix alice's export","assumptions":["see C:\\Users\\alice\\x"]},"email":"a@b.example","note":"key sk-ant-api03-AbC_123 end","total_cost_usd":0.5}"#;
        let out = sanitize(raw).unwrap();
        let value: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(value["total_cost_usd"], 0.5);
        assert_eq!(
            value["structured_output"]["summary"],
            "Fix <local>'s export"
        );
        assert_eq!(value["cwd"], "<path>");
        assert_eq!(value["email"], "<redacted>");
        assert_eq!(value["result"], "<text 13 chars>");
        assert_eq!(value["note"], "key <redacted> end");
        assert!(!out.to_lowercase().contains("alice"));
    }

    #[test]
    fn the_name_of_a_credential_source_is_kept_but_a_key_value_is_not() {
        let raw = r#"{"type":"system","apiKeySource":"ANTHROPIC_API_KEY","api_key":"abc"}"#;
        let value: Value = serde_json::from_str(sanitize(raw).unwrap().trim()).unwrap();
        assert_eq!(value["apiKeySource"], "ANTHROPIC_API_KEY");
        assert_eq!(value["api_key"], "<redacted>");
    }

    #[test]
    fn installed_plugins_skills_and_paths_are_emptied_but_tools_are_kept() {
        let raw = r#"{"type":"system","plugins":[{"name":"private-plugin"}],"skills":["mine"],"memory_paths":{"auto":"somewhere"},"tools":["StructuredOutput"]}"#;
        let out = sanitize(raw).unwrap();
        assert!(
            !out.contains("private-plugin") && !out.contains("mine") && !out.contains("somewhere")
        );
        let value: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(value["plugins"], serde_json::json!([]));
        assert_eq!(value["memory_paths"], serde_json::json!({}));
        assert_eq!(value["tools"], serde_json::json!(["StructuredOutput"]));
    }

    #[test]
    fn invalid_json_is_refused_with_its_line_number() {
        let error = sanitize("{\"type\":\"system\"}\nnot json\n").unwrap_err();
        assert!(error.starts_with("line 2"));
    }

    #[test]
    fn a_leftover_address_or_key_blocks_the_output() {
        let raw = r#"{"type":"system","model":"someone@example.com"}"#;
        assert!(sanitize(raw).is_err());
        let hidden = r#"{"type":"system","list":["x"],"model":"sk-ant-zzz"}"#;
        assert!(sanitize(hidden).is_ok_and(|out| !out.contains("sk-ant-")));
    }
}
