//! A small log of the actions that failed, in the data folder. The release
//! window has no console, so without this a failure is seen once, in a banner,
//! and then gone.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const FILE: &str = "autopilot.log";
/// One older file is kept, so the log holds at most about twice this.
const LIMIT_BYTES: u64 = 256 * 1024;
const LINE_CHARS: usize = 600;
/// Where the provider key lives. Its value is cut out of every line even when
/// it has no recognisable prefix.
const KEY_VARIABLE: &str = "AUTOPILOT_ANTHROPIC_API_KEY";

#[must_use]
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// Add one line for a failed action. A log that cannot be written must never
/// turn one failure into two, so write errors are dropped.
pub fn record(data_dir: &Path, at_ms: u64, what: &str, why: &str) {
    let line = line(
        at_ms,
        what,
        why,
        std::env::var(KEY_VARIABLE).ok().as_deref(),
    );
    let file = path(data_dir);
    if fs::metadata(&file).is_ok_and(|meta| meta.len() > LIMIT_BYTES) {
        let _ = fs::rename(&file, data_dir.join(format!("{FILE}.1")));
    }
    let _ = fs::create_dir_all(data_dir);
    if let Ok(mut open) = OpenOptions::new().create(true).append(true).open(&file) {
        let _ = open.write_all(line.as_bytes());
    }
}

fn line(at_ms: u64, what: &str, why: &str, key: Option<&str>) -> String {
    let mut text = autopilot_tracker::redact(why);
    if let Some(key) = key.filter(|key| !key.trim().is_empty()) {
        text = text.replace(key.trim(), "[redacted]");
    }
    // One line per failure, whatever the message holds.
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(LINE_CHARS)
        .collect();
    format!("{at_ms} {what}: {flat}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: a failed action leaves exactly one line, and no key or token
    // that was in the message. A log that copied the message as it came could
    // write a key to disk in clear. The tracker's word filter does not know a
    // key without a prefix, so the configured key is cut out by value too.
    #[test]
    fn a_failure_is_one_line_without_a_key_or_token() {
        let directory = tempfile::tempdir().unwrap();
        record(
            directory.path(),
            42,
            "Build",
            "claude failed\nusing sk-ant-api03-abcdefghijklmnopqrstuvwxyz and ghp_abcdefghijklmnopqrstuvwxyz0123 plain-secret-value-1",
        );
        let text = fs::read_to_string(path(directory.path())).unwrap();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert!(text.starts_with("42 Build: claude failed"), "{text:?}");
        assert!(
            !text.contains("sk-ant") && !text.contains("ghp_"),
            "{text:?}"
        );
        let keyed = line(
            1,
            "Plan",
            "bad key plain-secret-value-1 here",
            Some("plain-secret-value-1"),
        );
        assert!(!keyed.contains("plain-secret"), "{keyed:?}");
    }

    // Invariant: the log stays bounded. Without the rotation a window left open
    // for weeks grows the file without end.
    #[test]
    fn the_log_does_not_grow_without_bound() {
        let directory = tempfile::tempdir().unwrap();
        let long = "x".repeat(500);
        for at in 0..2_000 {
            record(directory.path(), at, "Build", &long);
        }
        let size =
            |name: &str| fs::metadata(directory.path().join(name)).map_or(0, |meta| meta.len());
        assert!(size(FILE) <= LIMIT_BYTES + 1_000);
        assert!(size("autopilot.log.1") > 0, "the older half is kept");
        assert!(size(FILE) + size("autopilot.log.1") < 2 * LIMIT_BYTES + 2_000);
    }
}
