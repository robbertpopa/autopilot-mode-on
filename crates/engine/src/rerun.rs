//! Run a step again.
//!
//! A passed or skipped step is settled, so running it again is an event of its
//! own in the history: `StepReset`. The history keeps everything that
//! happened, including the old attempt and what it cost; only the step's
//! state goes back to idle so the machine accepts a new start.

use std::path::Path;

use autopilot_agent::Agent;
use autopilot_core::{AttemptId, Event, Step, Ticket, TicketId, Understanding};
use autopilot_pipeline::{StepState, fold};
use autopilot_store::{SqliteStore, Store};

use crate::{DATABASE, Error, UNDERSTAND, event, lock_run, run_attempt, timestamp};

/// The ticket as it was started, rebuilt from its history: the title and the
/// latest words (an older history has none, and the title stands alone).
///
/// # Errors
/// Returns an error if the history has no ticket.
pub fn ticket_words(
    id: &TicketId,
    history: &[autopilot_core::RecordedEvent],
) -> Result<Ticket, Error> {
    ticket_of(id, history)
}

pub(crate) fn ticket_of(
    id: &TicketId,
    history: &[autopilot_core::RecordedEvent],
) -> Result<Ticket, Error> {
    let title = history
        .iter()
        .find_map(|record| match &record.event.kind {
            Event::TicketCreated { title } => Some(title.clone()),
            _ => None,
        })
        .ok_or(Error::UnknownTicket)?;
    let description = history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::Described { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    Ok(Ticket {
        id: id.clone(),
        title,
        description,
    })
}

/// Put a step back to idle. With `and_after`, every later step that already ran goes back too.
///
/// # Errors
/// Returns an error if another action holds the data directory, the ticket is
/// unknown or done, a step is still active, or the step has not run.
pub fn reset_step(
    directory: &Path,
    ticket: &TicketId,
    step: Step,
    and_after: bool,
) -> Result<(), Error> {
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let mut state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    let reset = event(timestamp()?, Some(step), Event::StepReset { and_after });
    // The machine decides whether the reset is allowed, so there is one set of rules.
    state
        .apply(&reset)
        .map_err(|error| Error::NotReady(error.to_string()))?;
    store.append(ticket, last, &[reset])?;
    Ok(())
}

/// Run Plan again for a ticket that already ran it, with the ticket's own
/// words, its answers and its attachments. A plan that was approved is
/// replaced and must be approved again.
///
/// # Errors
/// Returns an error if the ticket is unknown, done or busy, a step is still
/// active, or the agent or store cannot be used.
pub fn rerun_plan(
    directory: &Path,
    ticket: &TicketId,
    agent: &impl Agent,
) -> Result<autopilot_core::Plan, Error> {
    let settled = {
        let store = SqliteStore::open(directory.join(DATABASE))?;
        let history = store.events(ticket, 0)?;
        let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
        state.step(Step::Plan) != &StepState::Idle
    };
    if settled {
        reset_step(directory, ticket, Step::Plan, true)?;
    }
    crate::plan::plan_ticket(directory, ticket, agent)
}

/// Run Understand again for a ticket that already ran it. The later steps that
/// ran are reset with it, since they were built on the old answer.
///
/// # Errors
/// Returns an error if the ticket is unknown, done or busy, or the agent or
/// store cannot be used.
pub fn rerun_understand(
    directory: &Path,
    ticket: &TicketId,
    agent: &impl Agent,
    ask_after: Option<bool>,
) -> Result<Understanding, Error> {
    let ask_after = match ask_after {
        Some(choice) => choice,
        None => {
            crate::project_overrides(directory, Step::Understand)?
                .ask_after
                .unwrap_or(false)
                && !crate::allowances::allowed(directory, Step::Understand)
        }
    };
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let mut last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let mut state = fold(&history).map_err(|error| Error::History(error.to_string()))?;

    let title = history
        .iter()
        .find_map(|record| match &record.event.kind {
            Event::TicketCreated { title } => Some(title.clone()),
            _ => None,
        })
        .ok_or(Error::UnknownTicket)?;
    // The latest words win; an older history has none, and the title stands alone.
    let description = history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::Described { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let attempts = history
        .iter()
        .filter(|record| {
            record.event.step == UNDERSTAND
                && matches!(record.event.kind, Event::StepStarted { .. })
        })
        .count();

    let now = timestamp()?;
    let mut opening = Vec::new();
    if state.step(Step::Understand) != &StepState::Idle {
        let reset = event(now, UNDERSTAND, Event::StepReset { and_after: true });
        state
            .apply(&reset)
            .map_err(|error| Error::NotReady(error.to_string()))?;
        opening.push(reset);
    }
    let start = event(
        now,
        UNDERSTAND,
        Event::StepStarted {
            attempt: AttemptId::from(format!("understand-{}", attempts + 1)),
            session: None,
        },
    );
    state
        .apply(&start)
        .map_err(|error| Error::NotReady(error.to_string()))?;
    opening.push(start);
    let recorded = store.append(ticket, last, &opening)?;
    last = recorded.last().map_or(last, |record| record.seq);

    let ticket = Ticket {
        id: ticket.clone(),
        title,
        description,
    };
    run_attempt(directory, &mut store, &ticket, last, ask_after, agent, None)
}
