use std::collections::{BTreeMap, BTreeSet};

use autopilot_core::{AttemptId, Event, RecordedEvent, RequestId, SessionId, Step, TicketId};
use serde::{Deserialize, Serialize};

use crate::{StoreError, required_step};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TicketStatus {
    Idle,
    Running,
    Paused,
    Interrupted { reason: String },
    Failed { reason: String },
    Done,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StepState {
    Running {
        attempt: AttemptId,
        session: Option<SessionId>,
    },
    Passed,
    Skipped {
        reason: String,
    },
    Interrupted {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TicketState {
    ticket: TicketId,
    last_seq: u64,
    title: Option<String>,
    status: TicketStatus,
    steps: BTreeMap<Step, StepState>,
    pending_requests: BTreeSet<RequestId>,
    reported_cost_microusd: u128,
    estimated_cost_microusd: u128,
}

impl TicketState {
    #[must_use]
    pub fn new(ticket: TicketId) -> Self {
        Self {
            ticket,
            last_seq: 0,
            title: None,
            status: TicketStatus::Idle,
            steps: BTreeMap::new(),
            pending_requests: BTreeSet::new(),
            reported_cost_microusd: 0,
            estimated_cost_microusd: 0,
        }
    }

    #[must_use]
    pub const fn ticket(&self) -> &TicketId {
        &self.ticket
    }

    #[must_use]
    pub const fn last_seq(&self) -> u64 {
        self.last_seq
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub const fn status(&self) -> &TicketStatus {
        &self.status
    }

    #[must_use]
    pub fn step(&self, step: Step) -> Option<&StepState> {
        self.steps.get(&step)
    }

    #[must_use]
    pub const fn pending_requests(&self) -> &BTreeSet<RequestId> {
        &self.pending_requests
    }

    #[must_use]
    pub const fn reported_cost_microusd(&self) -> u128 {
        self.reported_cost_microusd
    }

    #[must_use]
    pub const fn estimated_cost_microusd(&self) -> u128 {
        self.estimated_cost_microusd
    }

    /// Replay consecutive events after this projection's sequence.
    ///
    /// # Errors
    /// Returns an error at the first event with invalid scope or ordering.
    pub fn replay(&mut self, events: &[RecordedEvent]) -> Result<(), StoreError> {
        for record in events {
            self.apply(record)?;
        }
        Ok(())
    }

    fn apply(&mut self, record: &RecordedEvent) -> Result<(), StoreError> {
        if record.ticket != self.ticket {
            return Err(StoreError::TicketMismatch);
        }
        let expected = self
            .last_seq
            .checked_add(1)
            .ok_or(StoreError::SequenceOverflow)?;
        if record.seq != expected {
            return Err(StoreError::SequenceConflict {
                expected,
                actual: record.seq,
            });
        }

        let step = record.event.step;
        match &record.event.kind {
            Event::TicketCreated { title } => self.title = Some(title.clone()),
            Event::StepStarted { attempt, session } => {
                let step = required_step(&record.event)?;
                self.steps.insert(
                    step,
                    StepState::Running {
                        attempt: attempt.clone(),
                        session: session.clone(),
                    },
                );
                self.status = TicketStatus::Running;
            }
            Event::StepFinished => {
                let step = required_step(&record.event)?;
                self.steps.insert(step, StepState::Passed);
            }
            Event::StepSkipped { reason } => {
                let step = required_step(&record.event)?;
                self.steps.insert(
                    step,
                    StepState::Skipped {
                        reason: reason.clone(),
                    },
                );
            }
            Event::Asked { request } => {
                self.pending_requests.insert(request.clone());
            }
            Event::Answered { request, .. } => {
                self.pending_requests.remove(request);
            }
            Event::CostTick { amount, estimated } => {
                let total = if *estimated {
                    &mut self.estimated_cost_microusd
                } else {
                    &mut self.reported_cost_microusd
                };
                // The event count and each amount fit in u64, so their sum fits in u128.
                *total = total
                    .checked_add(u128::from(amount.as_microusd()))
                    .ok_or(StoreError::CostOverflow)?;
            }
            Event::Paused => self.status = TicketStatus::Paused,
            Event::Resumed => self.status = TicketStatus::Running,
            Event::Interrupted { reason } => {
                if let Some(step) = step {
                    self.steps.insert(
                        step,
                        StepState::Interrupted {
                            reason: reason.clone(),
                        },
                    );
                }
                self.status = TicketStatus::Interrupted {
                    reason: reason.clone(),
                };
            }
            Event::Failed { reason } => {
                if let Some(step) = step {
                    self.steps.insert(
                        step,
                        StepState::Failed {
                            reason: reason.clone(),
                        },
                    );
                }
                self.status = TicketStatus::Failed {
                    reason: reason.clone(),
                };
            }
            Event::Done => self.status = TicketStatus::Done,
            Event::AgentActivity { .. }
            | Event::UnderstandResult { .. }
            | Event::PlanResult { .. }
            | Event::PlanApproved { .. }
            | Event::RoundStarted { .. }
            | Event::CheckResult { .. }
            | Event::SettingsOverridden { .. } => {}
        }
        self.last_seq = record.seq;
        Ok(())
    }
}

/// Reconstruct a ticket from its full event history.
///
/// # Errors
/// Returns an error if the history cannot be replayed in order.
pub fn fold(ticket: TicketId, events: &[RecordedEvent]) -> Result<TicketState, StoreError> {
    let mut state = TicketState::new(ticket);
    state.replay(events)?;
    Ok(state)
}
