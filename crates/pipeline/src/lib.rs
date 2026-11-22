//! The step state machine. A ticket's state is a fold over its events, and the
//! only way to change it is to append an event the machine accepts.
//!
//! `Queued` is not a state here: nothing in the history says a step is
//! waiting, because that is a scheduler decision rather than a fact. The
//! scheduler can hold queued steps itself.

use std::collections::BTreeMap;

pub mod analyze;
pub mod clarify;
pub mod inbox;
pub mod overrides;
pub mod refine;
pub mod scheduler;
pub mod summary;

use autopilot_core::{Event, Money, NewEvent, RecordedEvent, RequestId, Step};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StepState {
    #[default]
    Idle,
    Running,
    /// Waiting for the owner. This is the "Needs you" state.
    Asking(RequestId),
    Paused,
    Skipped(String),
    Passed,
    Failed(String),
    Interrupted(String),
}

impl StepState {
    /// A step in one of these states holds the ticket: nothing else may start.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::Asking(_) | Self::Paused)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TicketState {
    created: bool,
    done: bool,
    steps: BTreeMap<Step, StepState>,
    cost: Money,
    /// True once any cost tick was an estimate, so a total is never shown as a bill.
    cost_estimated: bool,
}

impl TicketState {
    #[must_use]
    pub fn step(&self, step: Step) -> &StepState {
        const IDLE: StepState = StepState::Idle;
        self.steps.get(&step).unwrap_or(&IDLE)
    }

    /// The step that holds the ticket, if any.
    #[must_use]
    pub fn active(&self) -> Option<Step> {
        self.steps
            .iter()
            .find(|(_, state)| state.is_active())
            .map(|(step, _)| *step)
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    #[must_use]
    pub fn cost(&self) -> (Money, bool) {
        (self.cost, self.cost_estimated)
    }

    /// Apply one event.
    ///
    /// # Errors
    /// Returns an error, leaving the state unchanged, if the event is not
    /// allowed from the current state.
    pub fn apply(&mut self, event: &NewEvent) -> Result<(), TransitionError> {
        // Work on a copy so a rejected event leaves no half-applied change.
        let mut next = self.clone();
        next.transition(event)?;
        *self = next;
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one flat match over the event kinds; splitting it would hide the state machine"
    )]
    fn transition(&mut self, event: &NewEvent) -> Result<(), TransitionError> {
        use TransitionError as E;
        if self.done {
            return Err(E::AfterDone);
        }
        if !self.created && !matches!(event.kind, Event::TicketCreated { .. }) {
            return Err(E::BeforeCreation);
        }
        match &event.kind {
            Event::TicketCreated { .. } => {
                if self.created {
                    return Err(E::AlreadyCreated);
                }
                self.created = true;
            }
            Event::StepStarted { .. } => {
                let step = Self::need_step(event)?;
                if let Some(active) = self.active() {
                    return Err(E::StepBusy { active });
                }
                // A failed or interrupted step may be tried again. A passed or
                // skipped one is settled.
                if matches!(self.step(step), StepState::Passed | StepState::Skipped(_)) {
                    return Err(E::NotRestartable(step));
                }
                self.steps.insert(step, StepState::Running);
            }
            Event::StepSkipped { reason } => {
                let step = Self::need_step(event)?;
                if !matches!(self.step(step), StepState::Idle) {
                    return Err(E::NotIdle(step));
                }
                self.steps.insert(step, StepState::Skipped(reason.clone()));
            }
            Event::StepReset { and_after } => {
                let step = Self::need_step(event)?;
                if let Some(active) = self.active() {
                    return Err(E::StepBusy { active });
                }
                if matches!(self.step(step), StepState::Idle) {
                    return Err(E::NothingToReset(step));
                }
                // `Step` is ordered like the pipeline, so "later" is `>`.
                for each in Step::ALL {
                    if each == step || (*and_after && each > step) {
                        self.steps.remove(&each);
                    }
                }
            }
            Event::AgentActivity { .. }
            | Event::UnderstandResult { .. }
            | Event::PlanResult { .. }
            | Event::PlanApproved { .. }
            | Event::RoundStarted { .. }
            | Event::SessionStarted { .. }
            | Event::CheckResult { .. } => {
                self.require(event, |s| matches!(s, StepState::Running))?;
            }
            Event::CostTick { amount, estimated } => {
                // A cost can arrive for the ticket or for a step, even a stopped one.
                self.cost = self.cost.checked_add(*amount).ok_or(E::CostOverflow)?;
                self.cost_estimated |= *estimated;
            }
            Event::Asked { request } => {
                let step = self.require(event, |s| matches!(s, StepState::Running))?;
                self.steps.insert(step, StepState::Asking(request.clone()));
            }
            Event::Answered { request, .. } => {
                let step = self.require(event, |s| matches!(s, StepState::Asking(_)))?;
                if self.step(step) != &StepState::Asking(request.clone()) {
                    return Err(E::WrongRequest);
                }
                self.steps.insert(step, StepState::Running);
            }
            Event::Paused => {
                let step = self.require(event, |s| matches!(s, StepState::Running))?;
                self.steps.insert(step, StepState::Paused);
            }
            Event::Resumed => {
                let step = self.require(event, |s| matches!(s, StepState::Paused))?;
                self.steps.insert(step, StepState::Running);
            }
            Event::StepFinished => {
                let step = self.require(event, |s| matches!(s, StepState::Running))?;
                self.steps.insert(step, StepState::Passed);
            }
            Event::Interrupted { reason } => {
                let step = self.require(event, StepState::is_active)?;
                self.steps
                    .insert(step, StepState::Interrupted(reason.clone()));
            }
            Event::Failed { reason } => {
                let step = self.require(event, StepState::is_active)?;
                self.steps.insert(step, StepState::Failed(reason.clone()));
            }
            // Recorded for the log. A settings change applies when the next step
            // starts, and the ticket text is only kept for a later re-run, so
            // neither alters any state here.
            Event::SettingsOverridden { .. }
            | Event::Described { .. }
            | Event::Attached { .. }
            | Event::Explained { .. }
            | Event::PipelineChosen { .. }
            | Event::UsageReported { .. }
            | Event::AssumptionsCorrected { .. }
            | Event::Workspace { .. }
            | Event::Triaged { .. }
            | Event::DesignResult { .. } => {}
            Event::Done => {
                if let Some(active) = self.active() {
                    return Err(E::StepBusy { active });
                }
                self.done = true;
            }
        }
        Ok(())
    }

    fn need_step(event: &NewEvent) -> Result<Step, TransitionError> {
        event.step.ok_or(TransitionError::MissingStep)
    }

    /// The event's step, if it is in a state the caller accepts.
    fn require(
        &self,
        event: &NewEvent,
        allowed: impl Fn(&StepState) -> bool,
    ) -> Result<Step, TransitionError> {
        let step = Self::need_step(event)?;
        if allowed(self.step(step)) {
            Ok(step)
        } else {
            Err(TransitionError::WrongState(step))
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("the ticket already has a creation event")]
    AlreadyCreated,
    #[error("the first event of a ticket must create it")]
    BeforeCreation,
    #[error("the ticket is done and takes no more events")]
    AfterDone,
    #[error("the event needs a step and had none")]
    MissingStep,
    #[error("{active:?} is still active")]
    StepBusy { active: Step },
    #[error("{0:?} already passed or was skipped")]
    NotRestartable(Step),
    #[error("{0:?} was not idle, so it cannot be skipped")]
    NotIdle(Step),
    #[error("{0:?} has not run, so there is nothing to reset")]
    NothingToReset(Step),
    #[error("the event is not allowed while {0:?} is in its current state")]
    WrongState(Step),
    #[error("the answer is for a different request than the one being asked")]
    WrongRequest,
    #[error("the total cost no longer fits in a counter")]
    CostOverflow,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("event {seq} was rejected: {reason}")]
pub struct FoldError {
    pub seq: u64,
    pub reason: TransitionError,
}

/// The state a recorded history leads to.
///
/// # Errors
/// Returns the first event the machine would not accept.
pub fn fold<'a>(
    events: impl IntoIterator<Item = &'a RecordedEvent>,
) -> Result<TicketState, FoldError> {
    let mut state = TicketState::default();
    for record in events {
        state.apply(&record.event).map_err(|reason| FoldError {
            seq: record.seq,
            reason,
        })?;
    }
    Ok(state)
}

#[cfg(test)]
mod refine_tests;
#[cfg(test)]
mod tests;
