//! Settings overrides recorded in a ticket's log.
//!
//! A change applies when the next step starts, never to a running step. The
//! log makes that checkable: what an attempt used is what had been recorded
//! before its `StepStarted`.

use autopilot_core::{Event, RecordedEvent, Step, StepOverride};

/// What the next start of `step` will use: every recorded change.
#[must_use]
pub fn for_next_start(history: &[RecordedEvent], step: Step) -> StepOverride {
    collect(history.iter(), step)
}

/// What the latest attempt of `step` started with: only changes recorded
/// before that attempt began. Empty if the step never started.
#[must_use]
pub fn used_by_latest_attempt(history: &[RecordedEvent], step: Step) -> StepOverride {
    let started = history
        .iter()
        .rev()
        .find(|record| {
            record.event.step == Some(step)
                && matches!(record.event.kind, Event::StepStarted { .. })
        })
        .map(|record| record.seq);
    match started {
        Some(seq) => collect(history.iter().filter(|record| record.seq < seq), step),
        None => StepOverride::default(),
    }
}

fn collect<'a>(records: impl Iterator<Item = &'a RecordedEvent>, step: Step) -> StepOverride {
    let mut merged = StepOverride::default();
    for record in records {
        if let Event::SettingsOverridden { changes, .. } = &record.event.kind {
            for change in changes.iter().filter(|change| change.step == step) {
                merged.merge(&change.change);
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopilot_core::{AttemptId, NewEvent, SettingsScope, StepChange, TicketId};

    fn record(seq: u64, step: Option<Step>, kind: Event) -> RecordedEvent {
        RecordedEvent {
            ticket: TicketId::from("t"),
            seq,
            event: NewEvent {
                at_ms: 0,
                step,
                kind,
            },
        }
    }

    fn change(ask_after: Option<bool>, model: Option<&str>) -> Event {
        Event::SettingsOverridden {
            scope: SettingsScope::Ticket,
            changes: vec![StepChange {
                step: Step::Understand,
                change: StepOverride {
                    ask_after,
                    model: model.map(Into::into),
                    effort: None,
                },
            }],
        }
    }

    fn started(seq: u64) -> RecordedEvent {
        record(
            seq,
            Some(Step::Understand),
            Event::StepStarted {
                attempt: AttemptId::from("a"),
                session: None,
            },
        )
    }

    // Invariant: a change made while a step runs is visible to the next start
    // but was not used by the running attempt. If it leaked in, a model switch
    // could change a step halfway through and make its cost unexplainable.
    #[test]
    fn a_change_waits_for_the_next_step_start() {
        let history = vec![
            record(1, None, change(Some(true), None)),
            started(2),
            record(3, None, change(Some(false), Some("opus"))),
        ];
        let used = used_by_latest_attempt(&history, Step::Understand);
        assert_eq!(used.ask_after, Some(true));
        assert_eq!(used.model, None);
        let next = for_next_start(&history, Step::Understand);
        assert_eq!(next.ask_after, Some(false));
        assert_eq!(next.model.as_deref(), Some("opus"));
    }

    #[test]
    fn unset_fields_keep_older_values_and_other_steps_are_untouched() {
        let history = vec![
            record(1, None, change(Some(true), Some("haiku"))),
            record(2, None, change(None, Some("sonnet"))),
        ];
        let next = for_next_start(&history, Step::Understand);
        assert_eq!(next.ask_after, Some(true));
        assert_eq!(next.model.as_deref(), Some("sonnet"));
        assert_eq!(
            for_next_start(&history, Step::Plan),
            StepOverride::default()
        );
        assert_eq!(
            used_by_latest_attempt(&history, Step::Understand),
            StepOverride::default()
        );
    }
}
