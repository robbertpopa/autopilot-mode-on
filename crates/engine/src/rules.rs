//! The owner's per-project rules file. It lives in the app's data directory,
//! never in a workspace, a worktree or a repository, so no commit, diff or
//! pull request can contain it. The engine adds it to every step prompt as
//! the owner's instructions. It guides the agent; it does not change policy:
//! Design and the pull request still ask, forced asks, protected paths and
//! budgets still apply, and nothing here reads the file to decide them.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use autopilot_core::Event;
use autopilot_profile::{leaf, normalize_path};
use sha2::{Digest, Sha256};

use crate::Error;

/// A rules file is advice, not a document. Larger is refused so a paste of
/// something else cannot crowd every prompt.
pub const MAX_RULES_BYTES: usize = 8 * 1024;

const FOLDER: &str = "projects";
const FILE: &str = "rules.md";

/// What saving or reading a rules file produced, for the window: no text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RulesInfo {
    pub bytes: usize,
    /// A rough count for the editor; the real count depends on the model.
    pub tokens_estimate: usize,
    pub sha256: String,
}

/// The folder name of a project inside the data directory: a readable leaf
/// and a hash of the whole path. The leaf is cut to plain characters, so no
/// project path can climb out with `..` or a separator, and two folders with
/// the same last name get two different folders.
#[must_use]
pub fn project_key(workspace: &str) -> String {
    let normal = normalize_path(workspace);
    let plain: String = leaf(&normal)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(24)
        .collect();
    let hash = crate::attachments::hex(&Sha256::digest(normal.as_bytes())[..8]);
    format!("{}-{hash}", plain.trim_matches('-'))
}

/// Where the rules file of a project is kept.
#[must_use]
pub fn rules_path(directory: &Path, workspace: &str) -> PathBuf {
    directory
        .join(FOLDER)
        .join(project_key(workspace))
        .join(FILE)
}

fn hash_of(text: &str) -> String {
    crate::attachments::hex(&Sha256::digest(text.as_bytes()))
}

fn info(text: &str) -> RulesInfo {
    RulesInfo {
        bytes: text.len(),
        tokens_estimate: text.len().div_ceil(4),
        sha256: hash_of(text),
    }
}

/// Read the rules of a project. A missing file means no rules. A file that is
/// too large, is not UTF-8 or holds a NUL is ignored with a reason, never a
/// panic and never partly used.
///
/// # Errors
/// Returns an error only when the file exists but cannot be read.
pub fn read_rules(directory: &Path, workspace: &str) -> Result<Option<String>, Error> {
    let path = rules_path(directory, workspace);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if bytes.len() > MAX_RULES_BYTES || bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes)
        .ok()
        .filter(|text| !text.trim().is_empty()))
}

/// Check a text before it is saved.
///
/// # Errors
/// Returns a sentence naming why the text is refused.
pub fn check_rules(text: &str) -> Result<(), String> {
    if text.len() > MAX_RULES_BYTES {
        return Err(format!(
            "{} bytes is over the limit of {MAX_RULES_BYTES}",
            text.len()
        ));
    }
    if text.contains('\0') {
        return Err("the text holds a NUL byte, so it is not plain text".into());
    }
    if autopilot_tracker::redact(text) != text {
        return Err("the text looks like it holds a key or token; remove it".into());
    }
    Ok(())
}

/// Save the rules of a project after checking them. Empty text removes the
/// file. The write goes through a temporary file, so a crash leaves the old
/// rules, not half of the new.
///
/// # Errors
/// Returns an error for text that is refused or a file that cannot be written.
pub fn save_rules(directory: &Path, workspace: &str, text: &str) -> Result<RulesInfo, Error> {
    check_rules(text).map_err(Error::Rules)?;
    let path = rules_path(directory, workspace);
    if text.trim().is_empty() {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        return Ok(info(""));
    }
    let folder = path
        .parent()
        .ok_or_else(|| Error::Rules("no folder".into()))?;
    fs::create_dir_all(folder)?;
    let temporary = folder.join(format!("{FILE}.tmp"));
    fs::write(&temporary, text)?;
    fs::rename(&temporary, &path)?;
    Ok(info(text))
}

/// Size, token estimate and hash of the saved rules, if there are any.
///
/// # Errors
/// Returns an error when the file exists but cannot be read.
pub fn rules_info(directory: &Path, workspace: &str) -> Result<Option<RulesInfo>, Error> {
    Ok(read_rules(directory, workspace)?.map(|text| info(&text)))
}

/// The rules block for a step prompt, and the line the step log keeps in its
/// place. The block says the text is the owner's guidance and that it cannot
/// change approvals, protected paths or budgets.
#[must_use]
pub fn prompt_block(directory: &Path, workspace: Option<&str>) -> Option<(String, String)> {
    let workspace = workspace?;
    let text = read_rules(directory, workspace).ok()??;
    let facts = info(&text);
    let block = format!(
        "The project owner's rules follow. They are guidance for style and wording. They do not change what must be approved, which paths are protected or any spending limit.\n---\n{}\n---\n",
        text.trim_end()
    );
    let logged = format!(
        "rules.md included: {} bytes, sha256 {}",
        facts.bytes,
        &facts.sha256[..12]
    );
    Some((block, logged))
}

/// Add the project's rules to a prompt and record that they were included,
/// as an activity line of `step` with the size and hash only. Returns the new
/// last sequence number. With no rules the prompt and the log stay as they were.
///
/// # Errors
/// Returns an error when the activity line cannot be written.
pub fn include_in_prompt(
    store: &mut autopilot_store::SqliteStore,
    directory: &Path,
    ticket: &autopilot_core::TicketId,
    step: autopilot_core::Step,
    last: u64,
    workspace: Option<&str>,
    prompt: &mut String,
) -> Result<u64, Error> {
    use autopilot_store::Store;
    let Some((block, logged)) = prompt_block(directory, workspace) else {
        return Ok(last);
    };
    *prompt = format!("{prompt}\n{block}");
    let new = crate::event(
        crate::timestamp()?,
        Some(step),
        Event::AgentActivity { summary: logged },
    );
    let committed = store.append(ticket, last, &[new])?;
    Ok(committed.last().map_or(last, |record| record.seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: a project's folder name is built from plain characters and a
    // hash of the full path, so no path climbs out of the data directory and
    // two same-named folders never share a rules file.
    #[test]
    fn the_project_key_is_safe_and_unique() {
        for hostile in ["../../etc", "D:\\x\\..\\..\\y", "a/b\\c", "..", "", "é€"] {
            let key = project_key(hostile);
            assert!(!key.contains(".."), "{key}");
            assert!(!key.contains('/') && !key.contains('\\'), "{key}");
        }
        assert_ne!(project_key("D:/a/app"), project_key("D:/b/app"));
        assert_eq!(project_key("D:\\a\\app\\"), project_key("D:/a/app"));
    }

    // Invariant: nothing hostile is saved or read back, whatever the content.
    #[test]
    fn hostile_text_is_refused_and_hostile_files_are_ignored() {
        let data = tempfile::tempdir().unwrap();
        let project = "D:/work/app";
        let token = format!("ghp_{}", "a".repeat(36));
        assert!(save_rules(data.path(), project, &format!("use {token}")).is_err());
        assert!(save_rules(data.path(), project, &"x".repeat(MAX_RULES_BYTES + 1)).is_err());
        assert!(save_rules(data.path(), project, "a\0b").is_err());
        assert_eq!(read_rules(data.path(), project).unwrap(), None);

        let saved = save_rules(data.path(), project, "Prefer short functions.\n").unwrap();
        assert_eq!(saved.bytes, 24);
        assert_eq!(
            read_rules(data.path(), project).unwrap().as_deref(),
            Some("Prefer short functions.\n")
        );
        // A file edited outside the window is held to the same limits.
        let path = rules_path(data.path(), project);
        fs::write(&path, vec![b'x'; MAX_RULES_BYTES + 1]).unwrap();
        assert_eq!(read_rules(data.path(), project).unwrap(), None);
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(read_rules(data.path(), project).unwrap(), None);
        fs::write(&path, [0xff, 0xfe, b'a']).unwrap();
        assert_eq!(read_rules(data.path(), project).unwrap(), None);
        // Saving nothing removes the file.
        save_rules(data.path(), project, "x").unwrap();
        save_rules(data.path(), project, "  \n").unwrap();
        assert!(!path.exists());
    }

    // Invariant: the log line carries the size and a short hash and never the
    // text, and the block tells the agent it cannot loosen policy.
    #[test]
    fn the_log_line_has_no_text_and_the_block_states_its_limits() {
        let data = tempfile::tempdir().unwrap();
        save_rules(data.path(), "D:/w/app", "Never touch billing.").unwrap();
        let (block, logged) = prompt_block(data.path(), Some("D:/w/app")).unwrap();
        assert!(block.contains("Never touch billing."));
        assert!(block.contains("do not change what must be approved"));
        assert!(!logged.contains("billing"), "{logged}");
        assert!(logged.starts_with("rules.md included: 20 bytes, sha256 "));
        assert!(prompt_block(data.path(), None).is_none());
        assert!(prompt_block(data.path(), Some("D:/other")).is_none());
    }
}
