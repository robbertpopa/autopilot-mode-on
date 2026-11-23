//! The project folder a ticket is about.
//!
//! The folder is recorded once in the ticket's history, so every later step of
//! the ticket looks at the same place. Only the read-only steps are given the
//! folder; writing code is a different step with its own containment.

use std::path::Path;

use autopilot_core::{Event, RecordedEvent, TicketId};
use autopilot_store::{SqliteStore, Store};

use crate::{DATABASE, Error, event, lock_run, timestamp};

/// The latest folder recorded in a history, if there is one.
#[must_use]
pub fn of(history: &[RecordedEvent]) -> Option<String> {
    history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::Workspace { path } => Some(path.clone()),
            _ => None,
        })
}

/// A folder as the history keeps it: resolved, without the verbatim prefix
/// Windows adds. The project and its rules file are found by this spelling.
#[must_use]
pub fn canonical(folder: &Path) -> Option<String> {
    let real = std::fs::canonicalize(folder)
        .ok()
        .filter(|path| path.is_dir())?;
    let text = real.to_string_lossy().into_owned();
    Some(text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned())
}

/// Record the folder for a ticket and return the path as kept. The path is
/// resolved first, so a link or a `..` cannot hide where it really leads.
///
/// # Errors
/// Returns an error if the path is not a folder, the ticket is unknown or
/// busy, or the store cannot be used.
pub fn record(directory: &Path, ticket: &TicketId, folder: &Path) -> Result<String, Error> {
    let text = canonical(folder)
        .ok_or_else(|| Error::NotReady(format!("{} is not a folder", folder.display())))?;
    let _owner = lock_run(directory, ticket)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    if of(&history).as_deref() == Some(text.as_str()) {
        return Ok(text);
    }
    store.append(
        ticket,
        last,
        &[event(
            timestamp()?,
            None,
            Event::Workspace { path: text.clone() },
        )],
    )?;
    Ok(text)
}
