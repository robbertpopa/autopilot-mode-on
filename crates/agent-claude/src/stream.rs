//! Line decoder for Claude Code `--output-format stream-json`.
//!
//! Every field is optional and unknown frames are ignored, because the format
//! belongs to another program that changes between releases.

use serde_json::Value;

use crate::pricing::Usage;

const SUMMARY_CHARS: usize = 160;

/// The terminal `result` frame, reduced to what the pipeline uses.
#[derive(Debug, Default, PartialEq)]
pub struct Finish {
    pub subtype: String,
    pub is_error: bool,
    pub structured_output: Option<Value>,
    pub cost_usd: Option<f64>,
    pub usage: Option<Usage>,
    /// The model named in the init frame. Used only to price token usage.
    pub model: Option<String>,
    /// The largest prompt one turn sent, from the assistant frames.
    pub peak_context: Option<u64>,
    /// The model's window, from the result frame's `modelUsage`.
    pub context_window: Option<u64>,
    /// The error kind of the assistant frame that preceded the result, such
    /// as `rate_limit`. Kept for the failure message only.
    pub error_kind: Option<String>,
}

#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
    finish: Option<Finish>,
    last_error: Option<String>,
    model: Option<String>,
    peak_context: Option<u64>,
}

impl Decoder {
    /// Feed bytes that may split lines or UTF-8 characters. Each complete
    /// activity line is reported at once so it survives a later failure.
    pub fn push(&mut self, bytes: &[u8], report: &mut impl FnMut(String)) {
        let mut line = std::mem::take(&mut self.pending);
        for part in bytes.split_inclusive(|&byte| byte == b'\n') {
            line.extend_from_slice(part);
            if part.ends_with(b"\n") {
                self.read_line(&line, report);
                line.clear();
            }
        }
        self.pending = line;
    }

    /// Read an undelimited tail. Call this only after a clean exit: after a
    /// kill or an output limit the tail may be a prefix of a different frame.
    pub fn finish_tail(&mut self, report: &mut impl FnMut(String)) {
        let tail = std::mem::take(&mut self.pending);
        self.read_line(&tail, report);
    }

    #[must_use]
    pub fn into_finish(self) -> Option<Finish> {
        self.finish
    }

    fn read_line(&mut self, line: &[u8], report: &mut impl FnMut(String)) {
        if line.trim_ascii().is_empty() {
            return;
        }
        // A corrupt line is skipped, not fatal: complete frames around it
        // still carry activity and possibly the result.
        let Ok(frame) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        match frame.get("type").and_then(Value::as_str) {
            Some("system") => {
                if frame.get("subtype").and_then(Value::as_str) == Some("init") {
                    self.model = frame.get("model").and_then(Value::as_str).map(bounded);
                }
                system(&frame, report);
            }
            Some("assistant") => self.assistant(&frame, report),
            Some("rate_limit_event") => rate_limit(&frame, report),
            Some("result") => self.result(&frame),
            _ => {}
        }
    }

    fn assistant(&mut self, frame: &Value, report: &mut impl FnMut(String)) {
        if let Some(kind) = frame.get("error").and_then(Value::as_str) {
            self.last_error = Some(bounded(kind));
        }
        if let Some(turn) = frame.pointer("/message/usage") {
            let count = |name: &str| turn.get(name).and_then(Value::as_u64).unwrap_or(0);
            let prompt = count("input_tokens")
                .saturating_add(count("cache_read_input_tokens"))
                .saturating_add(count("cache_creation_input_tokens"));
            if prompt > 0 {
                self.peak_context = Some(self.peak_context.map_or(prompt, |peak| peak.max(prompt)));
            }
        }
        let content = frame.pointer("/message/content").and_then(Value::as_array);
        for item in content.into_iter().flatten() {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str)
                        && !text.trim().is_empty()
                    {
                        report(format!("assistant: {}", bounded(text)));
                    }
                }
                Some("tool_use") => {
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    report(format!("tool requested: {}", bounded(name)));
                }
                _ => {}
            }
        }
    }

    fn result(&mut self, frame: &Value) {
        // Claude Code sends one result. A second would be a protocol break, so
        // the first one stays the answer.
        if self.finish.is_some() {
            return;
        }
        self.finish = Some(Finish {
            subtype: frame
                .get("subtype")
                .and_then(Value::as_str)
                .map(bounded)
                .unwrap_or_default(),
            is_error: frame
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            structured_output: frame
                .get("structured_output")
                .filter(|v| !v.is_null())
                .cloned(),
            cost_usd: frame.get("total_cost_usd").and_then(Value::as_f64),
            usage: frame.get("usage").map(usage),
            model: self.model.clone(),
            peak_context: self.peak_context,
            context_window: frame
                .get("modelUsage")
                .and_then(Value::as_object)
                .and_then(|models| {
                    models
                        .values()
                        .filter_map(|each| each.get("contextWindow").and_then(Value::as_u64))
                        .max()
                }),
            error_kind: self.last_error.take(),
        });
    }
}

/// Token counts use the Messages API field names. Missing fields count as zero.
fn usage(value: &Value) -> Usage {
    let count = |pointer: &str| value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
    let write_1h = count("/cache_creation/ephemeral_1h_input_tokens");
    // Without the split, every cache write is priced as the cheaper 5-minute kind.
    let write_total = count("/cache_creation_input_tokens");
    Usage {
        input: count("/input_tokens"),
        output: count("/output_tokens"),
        cache_read: count("/cache_read_input_tokens"),
        cache_write_5m: write_total.saturating_sub(write_1h),
        cache_write_1h: write_1h,
    }
}

fn system(frame: &Value, report: &mut impl FnMut(String)) {
    match frame.get("subtype").and_then(Value::as_str) {
        // The provider's session identifier is deliberately not reported.
        // Version and model go into the history so a surprising result can be
        // tied to the release that produced it. The session id stays out.
        Some("init") => {
            let part = |key| frame.get(key).and_then(Value::as_str).map(bounded);
            let version = part("claude_code_version").map(|v| format!("claude code {v}"));
            let identity: Vec<String> = [version, part("model")].into_iter().flatten().collect();
            report(if identity.is_empty() {
                "session started".into()
            } else {
                format!("session started ({})", identity.join(", "))
            });
        }
        Some("api_retry") => {
            let number = |key| frame.get(key).and_then(Value::as_u64);
            let status = number("error_status").map_or_else(|| "unknown".into(), |n| n.to_string());
            let attempt = number("attempt").map_or_else(|| "?".into(), |n| n.to_string());
            report(format!("api retry {attempt}, status {status}"));
        }
        _ => {}
    }
}

fn rate_limit(frame: &Value, report: &mut impl FnMut(String)) {
    let status = frame
        .pointer("/rate_limit_info/status")
        .and_then(Value::as_str);
    if let Some(status) = status.filter(|status| *status != "allowed") {
        report(format!("rate limit status: {}", bounded(status)));
    }
}

/// Model and tool text is untrusted. Show a short single line without
/// control characters so it cannot rewrite a terminal.
fn bounded(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(SUMMARY_CHARS)
        .collect();
    flat.trim().to_owned()
}

/// Convert a reported dollar figure to millionths of a dollar.
///
/// Returns `None` for anything that is not a finite, non-negative amount.
#[must_use]
pub fn microusd(dollars: f64) -> Option<u64> {
    if !dollars.is_finite() || dollars < 0.0 {
        return None;
    }
    let micro = (dollars * 1_000_000.0).round();
    // 1e15 micro-dollars is a billion dollars: far beyond any real step, and
    // well inside f64's exact integer range.
    // The range check above makes the cast exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (micro <= 1e15).then_some(micro as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUCCESS: &[u8] = include_bytes!("../../../fixtures/claude/synthetic-understand.jsonl");

    fn decode(input: &[u8], clean: bool) -> (Vec<String>, Option<Finish>) {
        let mut decoder = Decoder::default();
        let mut activity = Vec::new();
        let mut report = |summary| activity.push(summary);
        decoder.push(input, &mut report);
        if clean {
            decoder.finish_tail(&mut report);
        }
        (activity, decoder.into_finish())
    }

    #[test]
    fn the_synthetic_transcript_yields_activity_cost_and_structured_output() {
        let (activity, finish) = decode(SUCCESS, true);
        assert!(
            activity
                .iter()
                .any(|a| a == "session started (synthetic-model)")
        );
        assert!(activity.iter().any(|a| a.starts_with("assistant:")));
        let finish = finish.unwrap();
        assert_eq!(finish.subtype, "success");
        assert!(!finish.is_error);
        assert_eq!(finish.cost_usd.and_then(microusd), Some(1_234));
        assert!(finish.structured_output.is_some());
        assert_eq!(finish.model.as_deref(), Some("synthetic-model"));
        let usage = finish.usage.unwrap();
        assert_eq!((usage.input, usage.output, usage.cache_read), (10, 5, 0));
    }

    #[test]
    fn cache_writes_are_split_by_duration_when_reported() {
        let frame = serde_json::json!({
            "cache_creation_input_tokens": 300,
            "cache_creation": {"ephemeral_1h_input_tokens": 100},
            "cache_read_input_tokens": 7
        });
        let usage = usage(&frame);
        assert_eq!(
            (usage.cache_write_5m, usage.cache_write_1h, usage.cache_read),
            (200, 100, 7)
        );
        assert_eq!(super::usage(&serde_json::json!("junk")), Usage::default());
    }

    // Invariant: the history names the Claude Code release and model of a run,
    // and never the provider session id.
    #[test]
    fn the_init_frame_records_version_and_model_but_not_the_session_id() {
        let init = br#"{"type":"system","subtype":"init","claude_code_version":"2.1.330","model":"claude-haiku-5-5","session_id":"abc-secret-id"}"#;
        let (activity, _) = decode(&[init.as_slice(), b"\n"].concat(), true);
        assert_eq!(
            activity,
            ["session started (claude code 2.1.330, claude-haiku-5-5)"]
        );
        let bare = br#"{"type":"system","subtype":"init","session_id":"abc-secret-id"}"#;
        assert_eq!(
            decode(&[bare.as_slice(), b"\n"].concat(), true).0,
            ["session started"]
        );
    }

    #[test]
    fn a_split_character_is_buffered() {
        let line = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"€\"}]}}\n";
        let bytes = line.as_bytes();
        let cut = line.find('€').unwrap() + 1;
        let mut decoder = Decoder::default();
        let mut activity = Vec::new();
        decoder.push(&bytes[..cut], &mut |s| activity.push(s));
        assert_eq!(activity.len(), 0);
        decoder.push(&bytes[cut..], &mut |s| activity.push(s));
        assert_eq!(activity, ["assistant: €"]);
    }

    // Invariant: a retry followed by a terminal error keeps the retry
    // activity and is_error. S2 observed `success` with is_error true.
    #[test]
    fn s2_projection_frames_keep_retries_and_error_flag() {
        let fixture = include_str!("../../../fixtures/claude/s2-loopback-2.1.330.jsonl");
        for case in fixture.lines() {
            let case: Value = serde_json::from_str(case).unwrap();
            let mut lines = String::new();
            for frame in case["frames"].as_array().unwrap() {
                lines.extend([frame.to_string(), "\n".into()]);
            }
            let (activity, finish) = decode(lines.as_bytes(), true);
            let finish = finish.unwrap();
            assert!(
                activity.iter().any(|a| a.starts_with("api retry")),
                "{case}"
            );
            let failed = case["exit"] != 0;
            assert_eq!(finish.is_error, failed, "{}", case["case"]);
            if failed {
                assert!(finish.error_kind.is_some(), "{}", case["case"]);
            }
        }
    }

    #[test]
    fn an_undelimited_tail_counts_only_after_a_clean_exit() {
        let end = SUCCESS.iter().rposition(|&b| b == b'\n').unwrap();
        let without_newline = &SUCCESS[..end];
        assert!(decode(without_newline, false).1.is_none());
        assert!(decode(without_newline, true).1.is_some());
    }

    // Invariant: no input panics, and a truncated stream never loses an
    // activity line that was already complete.
    #[test]
    fn hostile_input_never_panics_and_keeps_complete_activity() {
        let full = decode(SUCCESS, true).0.len();
        let mut previous = 0;
        for cut in 0..=SUCCESS.len() {
            let (activity, _) = decode(&SUCCESS[..cut], false);
            assert!(activity.len() >= previous && activity.len() <= full);
            previous = activity.len();
        }
        for index in 0..SUCCESS.len() {
            for mask in [0x01, 0x80, 0xff] {
                let mut flipped = SUCCESS.to_vec();
                flipped[index] ^= mask;
                let _ = decode(&flipped, true);
            }
        }
        for junk in [
            &b"\xff\xfe\n"[..],
            b"[]\n",
            b"null\n",
            b"{\"type\":7}\n",
            b"{\"type\":\"result\",\"is_error\":\"x\"}\n",
        ] {
            let _ = decode(junk, true);
        }
    }

    #[test]
    fn a_second_result_does_not_replace_the_first() {
        let doubled = [
            SUCCESS,
            b"{\"type\":\"result\",\"subtype\":\"error_max_turns\"}\n",
        ]
        .concat();
        assert_eq!(decode(&doubled, true).1.unwrap().subtype, "success");
    }

    #[test]
    fn terminal_text_is_flattened_and_bounded() {
        let hostile = format!("a\u{1b}[2Jb\n{}", "x".repeat(500));
        let shown = bounded(&hostile);
        assert!(!shown.contains('\u{1b}') && !shown.contains('\n'));
        assert_eq!(shown.chars().count(), SUMMARY_CHARS);
    }

    #[test]
    fn cost_conversion_rejects_bad_amounts() {
        assert_eq!(microusd(0.0123), Some(12_300));
        assert_eq!(microusd(0.0), Some(0));
        for bad in [f64::NAN, f64::INFINITY, -0.01, 1e12] {
            assert_eq!(microusd(bad), None);
        }
    }
}
