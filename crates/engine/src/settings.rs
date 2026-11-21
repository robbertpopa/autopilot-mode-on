//! Recording settings overrides and reading the project-wide ones.

use std::{fs, io, path::Path};

use autopilot_core::{Event, NewEvent, SettingsScope, Step, StepChange, StepOverride, TicketId};
use autopilot_pipeline::{fold, overrides};
use autopilot_store::{SqliteStore, Store, StoreError};

use crate::{DATABASE, Error, timestamp};

const PROJECT_FILE: &str = "project-overrides.json";

fn read_project(directory: &Path) -> Result<Vec<StepChange>, Error> {
    match fs::read(directory.join(PROJECT_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| Error::History(format!("{PROJECT_FILE}: {error}"))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(error) => Err(error.into()),
    }
}

/// Project-wide overrides for one step. They apply to every ticket in this
/// data directory, below anything recorded on the ticket itself.
///
/// # Errors
/// Returns an error if the project file exists but cannot be read.
pub fn project_overrides(directory: &Path, step: Step) -> Result<StepOverride, Error> {
    let mut merged = StepOverride::default();
    for change in read_project(directory)?.iter().filter(|c| c.step == step) {
        merged.merge(&change.change);
    }
    Ok(merged)
}

/// What the next start of `step` will use: project overrides, then the
/// ticket's own recorded changes on top.
///
/// # Errors
/// Returns an error for an unreadable project file or history.
pub fn effective_overrides(
    directory: &Path,
    ticket: &TicketId,
    step: Step,
) -> Result<StepOverride, Error> {
    let mut merged = project_overrides(directory, step)?;
    let store = SqliteStore::open_read_only(directory.join(DATABASE))?;
    merged.merge(&overrides::for_next_start(&store.events(ticket, 0)?, step));
    Ok(merged)
}

/// Record an override. It never touches a running step: the engine reads
/// settings when a step starts. A project-scope change is also saved to the
/// project file so other tickets see it.
///
/// # Errors
/// Returns an error for an unknown ticket, an invalid history or a store failure.
pub fn override_settings(
    directory: &Path,
    ticket: &TicketId,
    scope: SettingsScope,
    changes: &[StepChange],
) -> Result<(), Error> {
    fs::create_dir_all(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    // No run lock: a change may arrive while a step runs, and the sequence
    // check below keeps the append consistent with that run's own writes.
    for _ in 0..5 {
        let history = store.events(ticket, 0)?;
        let last = history.last().ok_or(Error::UnknownTicket)?.seq;
        fold(&history).map_err(|error| Error::History(error.to_string()))?;
        let kind = Event::SettingsOverridden {
            scope,
            changes: changes.to_vec(),
        };
        let new = NewEvent {
            at_ms: timestamp()?,
            step: None,
            kind,
        };
        match store.append(ticket, last, &[new]) {
            Err(StoreError::SequenceConflict { .. }) => {}
            result => {
                result?;
                if scope == SettingsScope::Project {
                    save_project(directory, changes)?;
                }
                return Ok(());
            }
        }
    }
    Err(Error::Busy)
}

fn save_project(directory: &Path, changes: &[StepChange]) -> Result<(), Error> {
    let mut saved = read_project(directory)?;
    for change in changes {
        match saved.iter_mut().find(|entry| entry.step == change.step) {
            Some(entry) => entry.change.merge(&change.change),
            None => saved.push(change.clone()),
        }
    }
    let bytes =
        serde_json::to_vec_pretty(&saved).map_err(|error| Error::History(error.to_string()))?;
    let temporary = directory.join(format!("{PROJECT_FILE}.tmp"));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, directory.join(PROJECT_FILE))?;
    Ok(())
}
