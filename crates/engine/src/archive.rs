//! Putting a ticket away, bringing it back, and deleting it. All three are
//! events on the ticket's own history, so nothing is destroyed: a deleted
//! ticket stays in the log and is only listed nowhere.

use std::path::Path;

use autopilot_core::{Event, TicketId};
use autopilot_pipeline::fold;
use autopilot_store::{SqliteStore, Store};

use crate::{DATABASE, Error, event, lock_run, timestamp};

fn record(directory: &Path, ticket: &TicketId, kind: Event) -> Result<(), Error> {
    // The lock is the run's: while a step works on the ticket this refuses,
    // so a ticket is never put away from under its own run.
    let _owner = lock_run(directory, ticket)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let new = event(timestamp()?, None, kind);
    let mut state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    state
        .apply(&new)
        .map_err(|error| Error::NotReady(error.to_string()))?;
    store.append(ticket, last, &[new])?;
    Ok(())
}

/// Hide the ticket from the lists. Its history and its work stay.
///
/// # Errors
/// Returns an error if a step is running, the ticket is unknown or already
/// archived, or the log cannot be written.
pub fn archive_ticket(directory: &Path, ticket: &TicketId) -> Result<(), Error> {
    record(directory, ticket, Event::Archived)
}

/// Bring an archived ticket back to the lists.
///
/// # Errors
/// Returns an error if the ticket is unknown or not archived.
pub fn restore_ticket(directory: &Path, ticket: &TicketId) -> Result<(), Error> {
    record(directory, ticket, Event::Restored)
}

/// Delete the ticket: it is listed nowhere and takes no more events. The log
/// is kept. Removing its worktree and branch is a separate, deliberate step
/// of the `git` crate.
///
/// # Errors
/// Returns an error if a step is running or the ticket is unknown.
pub fn delete_ticket(directory: &Path, ticket: &TicketId) -> Result<(), Error> {
    record(directory, ticket, Event::Deleted)
}

#[cfg(test)]
mod tests {
    use autopilot_core::AttemptId;

    use super::*;

    fn ticket_with(directory: &Path, running: bool) -> TicketId {
        let id = TicketId::from("t-1");
        let mut store = SqliteStore::open(directory.join(DATABASE)).unwrap();
        let mut events = vec![event(
            1,
            None,
            Event::TicketCreated {
                title: "Put away".into(),
            },
        )];
        if running {
            events.push(event(
                2,
                Some(autopilot_core::Step::Understand),
                Event::StepStarted {
                    attempt: AttemptId::from("a"),
                    session: None,
                },
            ));
        }
        store.append(&id, 0, &events).unwrap();
        id
    }

    // Invariant: archive hides a ticket and restore brings it back with its
    // history whole; delete is final but the log stays; a step in progress
    // blocks archive and delete. Without the history check a ticket could be
    // put away from under its own run, or be restored when never archived.
    #[test]
    fn archive_restore_and_delete_are_events_that_keep_the_history() {
        let directory = tempfile::tempdir().unwrap();
        let id = ticket_with(directory.path(), false);
        assert!(restore_ticket(directory.path(), &id).is_err());
        archive_ticket(directory.path(), &id).unwrap();
        assert!(archive_ticket(directory.path(), &id).is_err());
        let view = crate::load_ticket(directory.path(), &id).unwrap();
        assert!(view.archived && !view.deleted);
        restore_ticket(directory.path(), &id).unwrap();
        assert!(!crate::load_ticket(directory.path(), &id).unwrap().archived);
        delete_ticket(directory.path(), &id).unwrap();
        let view = crate::load_ticket(directory.path(), &id).unwrap();
        assert!(view.deleted);
        assert!(restore_ticket(directory.path(), &id).is_err());
        let store = SqliteStore::open(directory.path().join(DATABASE)).unwrap();
        assert_eq!(store.events(&id, 0).unwrap().len(), 4);
    }

    #[test]
    fn a_running_ticket_cannot_be_archived_or_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let id = ticket_with(directory.path(), true);
        assert!(archive_ticket(directory.path(), &id).is_err());
        assert!(delete_ticket(directory.path(), &id).is_err());
    }
}
