use std::{path::Path, time::Duration};

use autopilot_core::{Event, NewEvent, RecordedEvent, TicketId};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::{Store, StoreError, TicketState, required_step};

const DATABASE_VERSION: i64 = 1;
const SNAPSHOT_VERSION: i64 = 1;

pub struct SqliteStore {
    connection: Connection,
}

impl SqliteStore {
    /// Open a ticket database, creating and migrating it as needed.
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened or its schema is unsupported.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "wal")?;
        connection.pragma_update(None, "synchronous", "full")?;
        migrate(&mut connection)?;
        Ok(Self { connection })
    }

    /// Open the event log for replay without running migrations or creating a database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be opened or its schema is unsupported.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != DATABASE_VERSION {
            return Err(StoreError::DatabaseVersion(version));
        }
        Ok(Self { connection })
    }
}

impl SqliteStore {
    /// Every ticket that has at least one event, oldest first.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read.
    pub fn tickets(&self) -> Result<Vec<TicketId>, crate::StoreError> {
        let mut select = self
            .connection
            .prepare("SELECT ticket_id FROM events GROUP BY ticket_id ORDER BY MIN(rowid)")?;
        let rows = select.query_map([], |row| row.get::<_, String>(0))?;
        let mut tickets = Vec::new();
        for row in rows {
            tickets.push(TicketId::from(row?));
        }
        Ok(tickets)
    }
}

impl Store for SqliteStore {
    fn append(
        &mut self,
        ticket: &TicketId,
        expected_last_seq: u64,
        events: &[NewEvent],
    ) -> Result<Vec<RecordedEvent>, StoreError> {
        // Lock before looking up the tail so competing writers compare against the committed sequence.
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual = latest_seq(&tx, ticket)?;
        if actual != expected_last_seq {
            return Err(StoreError::SequenceConflict {
                expected: expected_last_seq,
                actual,
            });
        }

        let mut seq = actual;
        let mut written = Vec::with_capacity(events.len());
        {
            let mut insert = tx.prepare(
                "INSERT INTO events (ticket_id, seq, body, answered_request) VALUES (?1, ?2, ?3, ?4)"
            )?;
            for event in events {
                if matches!(
                    event.kind,
                    Event::StepStarted { .. } | Event::StepFinished | Event::StepSkipped { .. }
                ) {
                    required_step(event)?;
                }
                seq = seq.checked_add(1).ok_or(StoreError::SequenceOverflow)?;
                let sql_seq = i64::try_from(seq).map_err(|_| StoreError::SequenceOverflow)?;
                let answer = match &event.kind {
                    Event::Answered { request, .. } => Some(request.as_str()),
                    _ => None,
                };
                let body = serde_json::to_string(event)?;
                insert.execute(params![ticket.as_str(), sql_seq, body, answer])?;
                written.push(RecordedEvent {
                    ticket: ticket.clone(),
                    seq,
                    event: event.clone(),
                });
            }
        }
        tx.commit()?;
        Ok(written)
    }

    fn events(&self, ticket: &TicketId, after_seq: u64) -> Result<Vec<RecordedEvent>, StoreError> {
        let Ok(after) = i64::try_from(after_seq) else {
            return Ok(Vec::new());
        };
        let mut select = self.connection.prepare(
            "SELECT seq, body FROM events WHERE ticket_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let mut rows = select.query(params![ticket.as_str(), after])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let body = row.get_ref(1)?.as_str().map_err(rusqlite::Error::from)?;
            events.push(RecordedEvent {
                ticket: ticket.clone(),
                seq: read_seq(row.get(0)?)?,
                event: serde_json::from_str(body)?,
            });
        }
        Ok(events)
    }

    fn save_snapshot(&mut self, state: &TicketState) -> Result<(), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let latest = latest_seq(&tx, state.ticket())?;
        let seq = state.last_seq();
        if seq == 0 || seq > latest {
            return Err(StoreError::SnapshotSequence {
                snapshot: seq,
                latest,
            });
        }
        let sql_seq = i64::try_from(seq).map_err(|_| StoreError::SequenceOverflow)?;

        tx.execute(
            "INSERT INTO snapshots (ticket_id, last_seq, schema_version, body) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (ticket_id) DO UPDATE SET
                 last_seq = excluded.last_seq, schema_version = excluded.schema_version, body = excluded.body
             WHERE snapshots.last_seq <= excluded.last_seq",
            params![state.ticket().as_str(), sql_seq, SNAPSHOT_VERSION, serde_json::to_string(state)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn load_snapshot(&self, ticket: &TicketId) -> Result<Option<TicketState>, StoreError> {
        let selected = self
            .connection
            .query_row(
                "SELECT last_seq, body FROM snapshots WHERE ticket_id = ?1 AND schema_version = ?2",
                params![ticket.as_str(), SNAPSHOT_VERSION],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;

        let Some((seq, body)) = selected else {
            return Ok(None);
        };
        let state: TicketState = serde_json::from_str(&body)?;
        if state.ticket() != ticket || state.last_seq() != read_seq(seq)? {
            return Err(StoreError::SnapshotMismatch);
        }
        Ok(Some(state))
    }
}

fn latest_seq(connection: &Connection, ticket: &TicketId) -> Result<u64, StoreError> {
    read_seq(connection.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM events WHERE ticket_id = ?1",
        [ticket.as_str()],
        |row| row.get(0),
    )?)
}

fn read_seq(sequence: i64) -> Result<u64, StoreError> {
    u64::try_from(sequence).map_err(|_| StoreError::InvalidSequence(sequence))
}

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        0 => {
            tx.execute_batch(
                "CREATE TABLE events (
                    ticket_id TEXT NOT NULL,
                    seq INTEGER NOT NULL CHECK (seq > 0),
                    body TEXT NOT NULL,
                    answered_request TEXT,
                    PRIMARY KEY (ticket_id, seq),
                    UNIQUE (ticket_id, answered_request)
                ) STRICT;
                CREATE TABLE snapshots (
                    ticket_id TEXT PRIMARY KEY NOT NULL,
                    last_seq INTEGER NOT NULL CHECK (last_seq > 0),
                    schema_version INTEGER NOT NULL,
                    body TEXT NOT NULL
                ) STRICT;",
            )?;
            tx.pragma_update(None, "user_version", DATABASE_VERSION)?;
        }
        DATABASE_VERSION => {}
        newer => return Err(StoreError::DatabaseVersion(newer)),
    }
    tx.commit()?;
    Ok(())
}
