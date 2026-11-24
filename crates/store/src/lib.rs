//! Transactional event storage and deterministic ticket replay.

mod replay;
mod sqlite;

pub use replay::{StepState, TicketState, TicketStatus, fold};
pub use sqlite::SqliteStore;

use autopilot_core::{Event, NewEvent, RecordedEvent, Step, TicketId};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("SQLite event storage: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("this request was already answered, and an answer cannot be given twice")]
    AlreadyAnswered,
    #[error("could not encode or decode stored event state: {0}")]
    Json(#[from] serde_json::Error),
    #[error("event sequence conflict: expected {expected}, found {actual}")]
    SequenceConflict { expected: u64, actual: u64 },
    #[error("the event sequence reached its storage limit")]
    SequenceOverflow,
    #[error("database contains an invalid sequence {0}")]
    InvalidSequence(i64),
    #[error("database schema {0} is incompatible with this application")]
    DatabaseVersion(i64),
    #[error("the snapshot sequence {snapshot} is outside the event history ending at {latest}")]
    SnapshotSequence { snapshot: u64, latest: u64 },
    #[error("snapshot metadata does not match its stored state")]
    SnapshotMismatch,
    #[error("replay received an event for a different ticket")]
    TicketMismatch,
    #[error("step event {0} is missing its step")]
    MissingStep(&'static str),
    #[error("stored cost total is outside the replay range")]
    CostOverflow,
}

fn required_step(event: &NewEvent) -> Result<Step, StoreError> {
    event.step.ok_or_else(|| {
        let name = match event.kind {
            Event::StepStarted { .. } => "step_started",
            Event::StepFinished => "step_finished",
            Event::StepSkipped { .. } => "step_skipped",
            _ => "step event",
        };
        StoreError::MissingStep(name)
    })
}

pub trait Store {
    /// Atomically assign new sequence numbers and append a batch to one ticket.
    ///
    /// # Errors
    /// Returns an error for a stale sequence, encoding failure or storage failure.
    fn append(
        &mut self,
        ticket: &TicketId,
        expected_last_seq: u64,
        events: &[NewEvent],
    ) -> Result<Vec<RecordedEvent>, StoreError>;

    /// Load this ticket's events strictly after `after_seq`, in sequence order.
    ///
    /// # Errors
    /// Returns an error if a stored event cannot be read or decoded.
    fn events(&self, ticket: &TicketId, after_seq: u64) -> Result<Vec<RecordedEvent>, StoreError>;

    /// Save a replayed projection. A newer stored snapshot takes precedence.
    ///
    /// # Errors
    /// Returns an error if the projection is outside the event history or cannot be saved.
    fn save_snapshot(&mut self, state: &TicketState) -> Result<(), StoreError>;

    /// Load a compatible snapshot. A missing or old snapshot triggers full replay.
    ///
    /// # Errors
    /// Returns an error if the matching snapshot cannot be read or decoded.
    fn load_snapshot(&self, ticket: &TicketId) -> Result<Option<TicketState>, StoreError>;
}
