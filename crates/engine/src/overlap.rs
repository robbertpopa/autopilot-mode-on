//! Two open tickets that mean to change the same files. After Understand
//! predicts the files a ticket touches, this lists the other open tickets in
//! the same project whose predictions overlap. It only reports: starting,
//! holding or merging a ticket is the owner's call, and nothing is queued.

use std::path::{Component, Path, PathBuf};

use autopilot_core::TicketId;

use crate::{Error, TicketView, load_ticket, tickets};

/// An open ticket that overlaps the one asked about.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketOverlap {
    pub other: TicketId,
    pub title: String,
    /// The path of the other ticket that the two share, as it predicted it.
    pub path: String,
}

/// A prediction is a repository-relative path. Anything else (empty, absolute,
/// climbing out) is dropped here instead of failing the whole comparison, since
/// the predictions come from an agent and may be wrong.
fn usable(path: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(path.trim());
    let plain = !path.trim().is_empty()
        && candidate
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
    plain.then_some(candidate)
}

fn predicted(view: &TicketView) -> Vec<PathBuf> {
    view.outputs
        .iter()
        .find(|(step, _)| *step == autopilot_core::Step::Understand)
        .and_then(|(_, text)| serde_json::from_str::<autopilot_core::Understanding>(text).ok())
        .map(|result| {
            result
                .files_touched
                .iter()
                .filter_map(|path| usable(path))
                .collect()
        })
        .unwrap_or_default()
}

/// The repository-relative paths a ticket predicted it will change, with
/// anything unusable dropped. Empty when nothing was predicted or the history
/// cannot be read: a queue then treats the ticket as touching nothing.
#[must_use]
pub fn predicted_paths(directory: &Path, ticket: &TicketId) -> Vec<PathBuf> {
    load_ticket(directory, ticket)
        .map(|view| predicted(&view))
        .unwrap_or_default()
}

/// The open tickets, other than `ticket`, that predicted a file this one
/// predicts. Tickets in a different project folder are not compared.
///
/// # Errors
/// Returns an error if a history cannot be read.
pub fn find_overlaps(directory: &Path, ticket: &TicketId) -> Result<Vec<TicketOverlap>, Error> {
    let mine = load_ticket(directory, ticket)?;
    let mine_paths = predicted(&mine);
    if mine_paths.is_empty() {
        return Ok(vec![]);
    }
    let folder = mine.workspace.clone();
    let mut found = Vec::new();
    for id in tickets(directory)? {
        if id == *ticket {
            continue;
        }
        let other = load_ticket(directory, &id)?;
        if other.badge == crate::Badge::Done || other.workspace != folder {
            continue;
        }
        let theirs = predicted(&other);
        let overlaps = autopilot_git::overlapping_paths(&mine_paths, &theirs)
            .map_err(|error| Error::NotReady(error.to_string()))?;
        if let Some(first) = overlaps.first() {
            found.push(TicketOverlap {
                other: id,
                title: other.title,
                path: first.active.display().to_string(),
            });
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: only repository-relative, plain paths are compared. An agent
    // that predicts an absolute path or one with `..` must not fail the whole
    // check or make two unrelated tickets look the same.
    #[test]
    fn only_plain_relative_paths_are_compared() {
        assert!(usable("src/export.rs").is_some());
        assert!(usable("  src/a.rs ").is_some());
        for bad in ["", "   ", "../x", "/etc/passwd", "a/../../b"] {
            assert!(usable(bad).is_none(), "{bad}");
        }
    }
}
