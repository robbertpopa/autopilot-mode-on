//! Local fixture actions shared by the CLI and future clients.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use autopilot_agent::{Agent, AgentError};
use autopilot_core::{
    AttemptId, Event, NewEvent, RecordedEvent, Step, Ticket, TicketId, Understanding,
};
use autopilot_store::{SqliteStore, Store, StoreError};

const DATABASE: &str = "events.sqlite3";
const UNDERSTAND: Option<Step> = Some(Step::Understand);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ticket ID and title must be non-empty")]
    InvalidTicket,
    #[error("an action is already running in this data directory")]
    Busy,
    #[error("this ticket already has a run in this data directory")]
    AlreadyStarted,
    #[error("ticket has no recorded run")]
    UnknownTicket,
    #[error("the current time is outside the supported range")]
    Clock,
    #[error("fixture execution failed: {0}")]
    Agent(#[source] AgentError),
    #[error(transparent)]
    Storage(#[from] StoreError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Run and durably record one Understand attempt.
///
/// # Errors
/// Returns an error if the ticket, agent, data directory or store cannot be used.
pub fn run_understand(
    directory: &Path,
    ticket: &Ticket,
    agent: &impl Agent,
) -> Result<Understanding, Error> {
    if ticket.id.as_str().trim().is_empty() || ticket.title.trim().is_empty() {
        return Err(Error::InvalidTicket);
    }
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let started_at = timestamp()?;
    let start_events = [
        event(
            started_at,
            None,
            Event::TicketCreated {
                title: ticket.title.clone(),
            },
        ),
        event(
            started_at,
            UNDERSTAND,
            Event::StepStarted {
                attempt: AttemptId::from("understand-1"),
                session: None,
            },
        ),
    ];
    let start = match store.append(&ticket.id, 0, &start_events) {
        Err(StoreError::SequenceConflict { .. }) => return Err(Error::AlreadyStarted),
        result => result?,
    };
    let last = start.last().map_or(0, |record| record.seq);

    let response = agent.understand(ticket);
    let finished_at = timestamp()?;
    match response {
        Ok(response) => {
            let mut terminal = response
                .activity
                .into_iter()
                .map(|summary| event(finished_at, UNDERSTAND, Event::AgentActivity { summary }))
                .collect::<Vec<_>>();
            terminal.push(event(
                finished_at,
                UNDERSTAND,
                Event::UnderstandResult {
                    result: response.result.clone(),
                },
            ));
            terminal.push(event(finished_at, UNDERSTAND, Event::StepFinished));
            terminal.push(event(finished_at, None, Event::Done));
            store.append(&ticket.id, last, &terminal)?;
            Ok(response.result)
        }
        Err(error) => {
            store.append(
                &ticket.id,
                last,
                &[event(
                    finished_at,
                    UNDERSTAND,
                    Event::Failed {
                        reason: "Understand fixture failed".into(),
                    },
                )],
            )?;
            Err(Error::Agent(error))
        }
    }
}

/// Replay committed events after a cursor through a read-only connection.
///
/// # Errors
/// Returns an error for an unknown ticket or an unreadable database.
pub fn watch(directory: &Path, ticket: &TicketId, after: u64) -> Result<Vec<RecordedEvent>, Error> {
    let store = SqliteStore::open_read_only(directory.join(DATABASE))?;
    let events = store.events(ticket, after)?;
    if events.is_empty() && (after == 0 || store.events(ticket, 0)?.is_empty()) {
        return Err(Error::UnknownTicket);
    }
    Ok(events)
}

fn lock_run(directory: &Path) -> Result<File, Error> {
    fs::create_dir_all(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("run.lock"))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(Error::Busy),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

fn timestamp() -> Result<u64, Error> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Clock)?;
    u64::try_from(duration.as_millis()).map_err(|_| Error::Clock)
}

fn event(at_ms: u64, step: Option<Step>, kind: Event) -> NewEvent {
    NewEvent { at_ms, step, kind }
}
