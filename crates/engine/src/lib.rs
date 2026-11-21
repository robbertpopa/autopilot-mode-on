//! Local fixture actions shared by the CLI and future clients.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use autopilot_agent::{Agent, AgentError, AgentEvent, StepRequest};
use autopilot_core::{
    AttemptId, Event, NewEvent, OpenQuestion, RecordedEvent, RequestId, Step, Ticket, TicketId,
    Understanding,
};
use autopilot_pipeline::{
    StepState,
    clarify::{self, Choice, ClarifyPlan, QaPair},
    fold,
    inbox::{Inbox, InboxError, Reply, Request, RequestBody, check_reply},
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
    #[error("clarify cannot run now: {0}")]
    NotReady(String),
    #[error(transparent)]
    Answer(#[from] clarify::ClarifyError),
    #[error(transparent)]
    Inbox(#[from] InboxError),
    #[error("the recorded history is invalid: {0}")]
    History(String),
    #[error("ticket has no recorded run")]
    UnknownTicket,
    #[error("the current time is outside the supported range")]
    Clock,
    #[error("agent execution failed: {0}")]
    Agent(#[source] AgentError),
    #[error("the agent stopped at a limit: {0}")]
    Stopped(String),
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
    let mut last = start.last().map_or(0, |record| record.seq);

    let response = match agent.start(StepRequest {
        step: Step::Understand,
        ticket,
    }) {
        Ok(mut run) => loop {
            match run.next() {
                Some(AgentEvent::Started) => {}
                Some(AgentEvent::Message { summary }) => {
                    last = record(
                        &mut store,
                        &ticket.id,
                        last,
                        Event::AgentActivity { summary },
                    )?;
                }
                Some(AgentEvent::Cost { amount, estimated }) => {
                    last = record(
                        &mut store,
                        &ticket.id,
                        last,
                        Event::CostTick { amount, estimated },
                    )?;
                }
                Some(AgentEvent::Result(result)) => break Ok(result),
                Some(AgentEvent::Stopped(reason)) => break Err(Outcome::Stopped(reason)),
                Some(AgentEvent::Failed(error)) => break Err(Outcome::Failed(error)),
                None => {
                    break Err(Outcome::Failed(
                        io::Error::other("agent stream ended without a result").into(),
                    ));
                }
            }
        },
        Err(error) => Err(Outcome::Failed(error)),
    };
    conclude(&mut store, &ticket.id, last, response)
}

/// Record how the attempt ended.
fn conclude(
    store: &mut SqliteStore,
    ticket: &TicketId,
    last: u64,
    response: Result<Understanding, Outcome>,
) -> Result<Understanding, Error> {
    let finished_at = timestamp()?;
    match response {
        Ok(result) => {
            let terminal = [
                event(
                    finished_at,
                    UNDERSTAND,
                    Event::UnderstandResult {
                        result: result.clone(),
                    },
                ),
                event(finished_at, UNDERSTAND, Event::StepFinished),
            ];
            store.append(ticket, last, &terminal)?;
            Ok(result)
        }
        Err(Outcome::Stopped(reason)) => {
            store.append(
                ticket,
                last,
                &[event(
                    finished_at,
                    UNDERSTAND,
                    Event::Interrupted {
                        reason: reason.clone(),
                    },
                )],
            )?;
            Err(Error::Stopped(reason))
        }
        Err(Outcome::Failed(error)) => {
            store.append(
                ticket,
                last,
                &[event(
                    finished_at,
                    UNDERSTAND,
                    Event::Failed {
                        reason: "Understand step failed".into(),
                    },
                )],
            )?;
            Err(Error::Agent(error))
        }
    }
}

/// Append one Understand event and return the new last sequence number.
fn record(
    store: &mut SqliteStore,
    ticket: &TicketId,
    last: u64,
    kind: Event,
) -> Result<u64, Error> {
    let committed = store.append(ticket, last, &[event(timestamp()?, UNDERSTAND, kind)])?;
    Ok(committed.last().map_or(last, |record| record.seq))
}

/// A limit stop is kept apart from failure so callers do not retry it blindly.
enum Outcome {
    Stopped(String),
    Failed(AgentError),
}

/// What Clarify decided after Understand.
#[derive(Debug, PartialEq, Eq)]
pub enum ClarifyStart {
    Skipped(&'static str),
    Asking {
        request: RequestId,
        questions: Vec<OpenQuestion>,
    },
}

const CLARIFY: Option<Step> = Some(Step::Clarify);
const CLARIFY_REQUEST: &str = "clarify-1";

/// Skip Clarify, or start it and ask. The questions come from the recorded
/// Understand result, so a restarted process asks the same ones.
///
/// # Errors
/// Returns an error if Understand has not passed or Clarify already ran.
pub fn start_clarify(directory: &Path, ticket: &TicketId) -> Result<ClarifyStart, Error> {
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::Understand) != &StepState::Passed {
        return Err(Error::NotReady("Understand has not passed".into()));
    }
    if state.step(Step::Clarify) != &StepState::Idle {
        return Err(Error::NotReady("Clarify already started".into()));
    }
    let understanding = latest_understanding(&history)
        .ok_or_else(|| Error::History("no Understand result".into()))?;
    let at = timestamp()?;
    match clarify::plan(understanding) {
        ClarifyPlan::Skip(reason) => {
            let skip = Event::StepSkipped {
                reason: reason.into(),
            };
            store.append(ticket, last, &[event(at, CLARIFY, skip)])?;
            Ok(ClarifyStart::Skipped(reason))
        }
        ClarifyPlan::Ask(questions) => {
            let request = RequestId::from(CLARIFY_REQUEST);
            let started = Event::StepStarted {
                attempt: AttemptId::from("clarify-1"),
                session: None,
            };
            let asked = Event::Asked {
                request: request.clone(),
            };
            store.append(
                ticket,
                last,
                &[event(at, CLARIFY, started), event(at, CLARIFY, asked)],
            )?;
            Inbox::in_data_dir(directory).write_request(&Request {
                id: request.clone(),
                ticket: ticket.clone(),
                body: RequestBody::Question {
                    questions: questions.clone(),
                },
            })?;
            Ok(ClarifyStart::Asking { request, questions })
        }
    }
}

/// Record the owner's answers and finish Clarify. The `Q → A` pairs are stored
/// in the answer event, so they are part of the replayable history.
///
/// # Errors
/// Returns an error if Clarify is not waiting on this request or the answers
/// do not match the questions.
pub fn answer_clarify(
    directory: &Path,
    ticket: &TicketId,
    request: &RequestId,
    choices: &[Choice],
) -> Result<Vec<QaPair>, Error> {
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let pairs = commit_clarify_answer(&mut store, ticket, &history, request, choices)?;
    Inbox::in_data_dir(directory).clean_up(request)?;
    Ok(pairs)
}

fn commit_clarify_answer(
    store: &mut SqliteStore,
    ticket: &TicketId,
    history: &[RecordedEvent],
    request: &RequestId,
    choices: &[Choice],
) -> Result<Vec<QaPair>, Error> {
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = fold(history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::Clarify) != &StepState::Asking(request.clone()) {
        return Err(Error::NotReady(
            "Clarify is not waiting on that request".into(),
        ));
    }
    let understanding = latest_understanding(history)
        .ok_or_else(|| Error::History("no Understand result".into()))?;
    let pairs = clarify::resolve(&understanding.open_questions, choices)?;
    let answer =
        serde_json::to_string(&pairs).map_err(|error| Error::History(error.to_string()))?;
    let at = timestamp()?;
    store.append(
        ticket,
        last,
        &[
            event(
                at,
                CLARIFY,
                Event::Answered {
                    request: request.clone(),
                    answer,
                },
            ),
            event(at, CLARIFY, Event::StepFinished),
        ],
    )?;
    Ok(pairs)
}

/// Reconcile the inbox folder with the event log for one ticket.
///
/// A waiting question gets its request file back if it is missing. A valid
/// answer file is committed once, and only then are the files removed. Files
/// for requests that are no longer waiting (a crash after the commit) are
/// removed without being applied.
///
/// # Errors
/// Returns an error for an invalid history or an unusable answer file. An
/// invalid answer leaves its files in place for the owner to replace.
pub fn sync_inbox(directory: &Path, ticket: &TicketId) -> Result<Vec<RequestId>, Error> {
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let inbox = Inbox::in_data_dir(directory);
    let history = store.events(ticket, 0)?;
    if history.is_empty() {
        return Err(Error::UnknownTicket);
    }
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    let mut applied = Vec::new();
    if let StepState::Asking(request) = state.step(Step::Clarify) {
        let understanding = latest_understanding(&history)
            .ok_or_else(|| Error::History("no Understand result".into()))?;
        let mirror = Request {
            id: request.clone(),
            ticket: ticket.clone(),
            body: RequestBody::Question {
                questions: understanding.open_questions.clone(),
            },
        };
        if inbox.read_request(request)?.is_none() {
            inbox.write_request(&mirror)?;
        }
        if let Some(reply) = inbox.read_answer(request)? {
            check_reply(&mirror, &reply)?;
            let Reply::Question { choices } = reply else {
                return Err(Error::NotReady("the reply is not a question answer".into()));
            };
            commit_clarify_answer(&mut store, ticket, &history, request, &choices)?;
            inbox.clean_up(request)?;
            applied.push(request.clone());
        }
    }
    let state =
        fold(&store.events(ticket, 0)?).map_err(|error| Error::History(error.to_string()))?;
    for request in inbox.requests()? {
        let waiting = state.step(Step::Clarify) == &StepState::Asking(request.id.clone());
        if request.ticket == *ticket && !waiting {
            inbox.clean_up(&request.id)?;
        }
    }
    // The request file is removed first, so a crash can leave a lone answer
    // file. It is only safe to delete when the log proves it was applied.
    for id in inbox.answer_ids()? {
        let committed = history.iter().any(|record| {
            matches!(&record.event.kind, Event::Answered { request, .. } if *request == id)
        });
        if committed {
            inbox.clean_up(&id)?;
        }
    }
    Ok(applied)
}

fn latest_understanding(history: &[RecordedEvent]) -> Option<&Understanding> {
    history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::UnderstandResult { result } => Some(result),
            _ => None,
        })
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
