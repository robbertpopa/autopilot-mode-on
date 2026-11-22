//! Choosing a pipeline for a ticket. The choice is recorded with the steps it
//! turns off, and each of those steps is skipped with a reason, so the fold
//! and the window show a dashed step and the engine cannot start it.
//!
//! A pipeline decides which steps run. It never changes what a step asks:
//! those rules stay with the profile and the policy.

use std::path::Path;

use autopilot_core::{Event, NewEvent, RecordedEvent, Step, TicketId};
use autopilot_pipeline::{StepState, fold};
use autopilot_profile::{Profile, StepMode};
use autopilot_store::{SqliteStore, Store};

use crate::{DATABASE, Error, lock_run, timestamp};

/// The reason prefix of a step a pipeline turned off. A later choice may
/// undo only skips that carry it, never one the engine decided on its own.
pub(crate) const OFF_PREFIX: &str = "off in pipeline ";

fn off_reason(name: &str) -> String {
    format!("{OFF_PREFIX}{name}")
}

/// The pipeline a history chose, if any.
#[must_use]
pub fn chosen(history: &[RecordedEvent]) -> Option<(String, Vec<Step>, Option<u32>)> {
    history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::PipelineChosen { name, off, rounds } => {
                Some((name.clone(), off.clone(), *rounds))
            }
            _ => None,
        })
}

/// Choose, or change, the pipeline of a ticket. It applies to steps that have
/// not started: a step that already ran cannot be turned off.
///
/// # Errors
/// Returns an error for an unknown pipeline or ticket, a step that already
/// started and would be switched off, or a store failure.
pub fn choose_pipeline(
    directory: &Path,
    ticket: &TicketId,
    profile: &Profile,
    name: &str,
) -> Result<(), Error> {
    let pipeline = profile
        .pipeline(name)
        .ok_or_else(|| Error::NotReady(format!("{name:?} is not a known pipeline")))?;
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let mut state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    let at = timestamp()?;
    let off = pipeline.off();
    let mut events = vec![NewEvent {
        at_ms: at,
        step: None,
        kind: Event::PipelineChosen {
            name: name.to_owned(),
            off: off.clone(),
            rounds: pipeline.rounds,
        },
    }];
    for step in Step::ALL {
        let wanted_off = pipeline.mode(step) == StepMode::Off;
        let reason = off_reason(name);
        let step_events: Vec<Event> = match (state.step(step).clone(), wanted_off) {
            (StepState::Idle, true) => vec![Event::StepSkipped { reason }],
            (StepState::Skipped(old), true) if old.starts_with(OFF_PREFIX) && old != reason => {
                vec![
                    Event::StepReset { and_after: false },
                    Event::StepSkipped { reason },
                ]
            }
            (StepState::Skipped(old), false) if old.starts_with(OFF_PREFIX) => {
                vec![Event::StepReset { and_after: false }]
            }
            (StepState::Idle | StepState::Skipped(_), _) | (_, false) => vec![],
            (_, true) => {
                return Err(Error::NotReady(format!(
                    "{} has already started and cannot be switched off",
                    step.as_str()
                )));
            }
        };
        for kind in step_events {
            let new = NewEvent {
                at_ms: at,
                step: Some(step),
                kind,
            };
            state
                .apply(&new)
                .map_err(|error| Error::NotReady(error.to_string()))?;
            events.push(new);
        }
    }
    store.append(ticket, last, &events)?;
    Ok(())
}

/// Skips to append in the same batch as a reset. A reset puts every later
/// step back to idle, including steps the ticket's pipeline turned off, so
/// they are skipped again; otherwise a retry would quietly turn an ask-only
/// ticket into one that can plan and build.
#[must_use]
pub(crate) fn reapply_off(
    history: &[RecordedEvent],
    state: &autopilot_pipeline::TicketState,
    at_ms: u64,
) -> Vec<NewEvent> {
    let Some((name, off, _)) = chosen(history) else {
        return vec![];
    };
    off.into_iter()
        .filter(|step| state.step(*step) == &StepState::Idle)
        .map(|step| NewEvent {
            at_ms,
            step: Some(step),
            kind: Event::StepSkipped {
                reason: off_reason(&name),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: only a skip that carries the pipeline prefix can be undone by
    // choosing another pipeline. An engine skip such as "the ticket needs no
    // design" is a fact about the ticket, not a choice.
    #[test]
    fn only_pipeline_skips_are_undone() {
        assert!(off_reason("bug-fix").starts_with(OFF_PREFIX));
        assert!(!"the ticket needs no design".starts_with(OFF_PREFIX));
    }
}
