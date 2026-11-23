//! A custom step: the owner's own prompt, run once on what the ticket has so
//! far, after Build and Check and before the pull request.
//!
//! The agent has no tools and writes no file, so a custom step can read and
//! judge but never change anything; its answer is a short report that is kept
//! in the history. The owner's prompt is their instruction. The ticket and the
//! notes are data, like everywhere else. A step that asks after itself waits
//! for the owner, and an owner who says no stops it as interrupted.

use std::path::Path;

use autopilot_agent::{Agent, CustomRequest};
use autopilot_core::{AttemptId, CustomOutcome, Event, RequestId, Step, Ticket};
use autopilot_pipeline::{
    StepState, fold,
    inbox::{Inbox, Request, RequestBody},
};
use autopilot_profile::CustomStep;
use autopilot_store::{SqliteStore, Store};

use crate::{DATABASE, Error, Pumped, event, lock_run, pump, timestamp};

/// What a ticket's custom step reported last, if it has run.
#[must_use]
pub fn latest(history: &[autopilot_core::RecordedEvent], step: Step) -> Option<&CustomOutcome> {
    history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::CustomResult { outcome } if record.event.step == Some(step) => Some(outcome),
            _ => None,
        })
}

/// Why a step cannot start now, or `None` when it can.
fn blocked(state: &autopilot_pipeline::TicketState, step: Step) -> Option<String> {
    if state.step(Step::BuildCheck) != &StepState::Passed {
        return Some("the build has not passed its checks".into());
    }
    match state.step(step) {
        StepState::Idle | StepState::Failed(_) | StepState::Interrupted(_) => {}
        StepState::Skipped(_) => return Some("this pipeline does not run the step".into()),
        _ => return Some("the step already ran or is running".into()),
    }
    None
}

/// Run one custom step and record what it reported.
///
/// Returns the outcome. When the step asks after itself, the ticket then waits
/// for the owner's answer in the inbox.
///
/// # Errors
/// Returns an error if the build has not passed, the step is not runnable, the
/// agent fails or reports something that must not be kept, or the store cannot
/// be used.
pub fn run_custom(
    directory: &Path,
    ticket: &Ticket,
    step: Step,
    definition: &CustomStep,
    ask_after: bool,
    agent: &impl Agent,
) -> Result<CustomOutcome, Error> {
    if !step.is_custom() {
        return Err(Error::NotReady(format!(
            "{} is not a custom step",
            step.as_str()
        )));
    }
    let _owner = lock_run(directory, &ticket.id)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(&ticket.id, 0)?;
    let mut last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    if let Some(why) = blocked(&state, step) {
        return Err(Error::NotReady(why));
    }
    let attempts = history
        .iter()
        .filter(|record| {
            record.event.step == Some(step)
                && matches!(record.event.kind, Event::StepStarted { .. })
        })
        .count();
    let started = Event::StepStarted {
        attempt: AttemptId::from(format!("{}-{}", step.as_str(), attempts + 1)),
        session: None,
    };
    let committed = store.append(
        &ticket.id,
        last,
        &[event(timestamp()?, Some(step), started)],
    )?;
    last = committed.last().map_or(last, |record| record.seq);

    let mut notes = crate::plan::brief(&history);
    let project = crate::workspace::of(&history);
    last = crate::rules::include_in_prompt(
        &mut store,
        directory,
        &ticket.id,
        step,
        last,
        project.as_deref(),
        &mut notes,
    )?;
    let run = agent.start_custom(CustomRequest {
        step,
        ticket,
        name: &definition.name,
        prompt: &definition.prompt,
        context: &notes,
    });
    let (pumped, last) = pump(&mut store, &ticket.id, Some(step), last, run)?;
    let how = Settle {
        directory,
        ticket,
        step,
        definition,
        ask_after,
        last,
    };
    how.apply(&mut store, pumped)
}

/// Everything needed to turn how the run ended into events.
struct Settle<'a> {
    directory: &'a Path,
    ticket: &'a Ticket,
    step: Step,
    definition: &'a CustomStep,
    ask_after: bool,
    last: u64,
}

impl Settle<'_> {
    fn apply(&self, store: &mut SqliteStore, pumped: Pumped) -> Result<CustomOutcome, Error> {
        let at = timestamp()?;
        let id = &self.ticket.id;
        let step = Some(self.step);
        match pumped {
            Pumped::Custom(outcome) => {
                if let Err(reason) = outcome.validate() {
                    let failed = Event::Failed {
                        reason: format!(
                            "{} gave an answer that is not allowed: {reason}",
                            self.definition.name
                        ),
                    };
                    store.append(id, self.last, &[event(at, step, failed)])?;
                    return Err(Error::NotReady(format!(
                        "the answer is not allowed: {reason}"
                    )));
                }
                let result = Event::CustomResult {
                    outcome: outcome.clone(),
                };
                if self.ask_after {
                    let history = store.events(id, 0)?;
                    let request = crate::approval_id(id, &history, self.step);
                    let asked = Event::Asked {
                        request: request.clone(),
                    };
                    store.append(
                        id,
                        self.last,
                        &[event(at, step, result), event(at, step, asked)],
                    )?;
                    Inbox::in_data_dir(self.directory).write_request(&approval(
                        id,
                        &request,
                        self.definition,
                        &outcome,
                    ))?;
                } else {
                    store.append(
                        id,
                        self.last,
                        &[
                            event(at, step, result),
                            event(at, step, Event::StepFinished),
                        ],
                    )?;
                }
                Ok(outcome)
            }
            Pumped::Stopped(reason) => {
                let interrupted = Event::Interrupted {
                    reason: reason.clone(),
                };
                store.append(id, self.last, &[event(at, step, interrupted)])?;
                Err(Error::Stopped(reason))
            }
            Pumped::Failed(error) => {
                let failed = Event::Failed {
                    reason: format!("{} failed", self.definition.name),
                };
                store.append(id, self.last, &[event(at, step, failed)])?;
                Err(Error::Agent(error))
            }
            Pumped::Understanding(_) | Pumped::Plan(_) | Pumped::Design(_) => Err(Error::NotReady(
                "the agent answered a custom step with another step's result".into(),
            )),
        }
    }
}

/// The approval a step that asks after itself waits for. A report that found
/// something is forced, so it cannot become a standing allowance.
fn approval(
    ticket: &autopilot_core::TicketId,
    id: &RequestId,
    definition: &CustomStep,
    outcome: &CustomOutcome,
) -> Request {
    let found = if outcome.ok {
        ""
    } else {
        " (it found something)"
    };
    Request {
        id: id.clone(),
        ticket: ticket.clone(),
        body: RequestBody::Approval {
            action: format!("continue after {}{found}", definition.name),
            forced: !outcome.ok,
        },
    }
}
