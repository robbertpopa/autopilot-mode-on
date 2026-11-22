//! Settings overrides recorded in a ticket's log.
//!
//! A change applies when the next step starts, never to a running step. The
//! log makes that checkable: what an attempt used is what had been recorded
//! before its `StepStarted`.

use autopilot_core::{Event, RecordedEvent, SettingsScope, Step, StepOverride, UnsureMode};

/// What the next start of `step` will use: every recorded change.
#[must_use]
pub fn for_next_start(history: &[RecordedEvent], step: Step) -> StepOverride {
    collect(history.iter(), step)
}

/// The ticket's own `when_unsure`, the latest one recorded. `None` means the
/// profile setting applies, which is also what every older history means.
#[must_use]
pub fn when_unsure_for_ticket(history: &[RecordedEvent]) -> Option<UnsureMode> {
    history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::SettingsOverridden {
                scope: SettingsScope::Ticket,
                when_unsure: Some(mode),
                ..
            } => Some(*mode),
            _ => None,
        })
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
    use autopilot_core::{AttemptId, NewEvent, StepChange, TicketId};

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
            when_unsure: None,
            changes: vec![StepChange {
                step: Step::Understand,
                change: StepOverride {
                    ask_after,
                    model: model.map(Into::into),
                    effort: None,
                    turns: None,
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

    // Invariant: an override saved before `turns` existed still reads, the
    // turn cap merges like the other fields, and a later override that leaves
    // it unset keeps the earlier cap. Otherwise a model change would silently
    // drop the raised cap a retry depends on.
    #[test]
    fn turns_merge_and_old_overrides_still_read() {
        let old = r#"{"step":"understand","ask_after":null,"model":"opus"}"#;
        let parsed: StepChange = serde_json::from_str(old).unwrap();
        assert_eq!(parsed.change.turns, None);
        let mut merged = StepOverride {
            turns: Some(40),
            ..StepOverride::default()
        };
        merged.merge(&parsed.change);
        assert_eq!(merged.turns, Some(40));
        assert_eq!(merged.model.as_deref(), Some("opus"));
        merged.merge(&StepOverride {
            turns: Some(80),
            ..StepOverride::default()
        });
        assert_eq!(merged.turns, Some(80));
    }

    // Invariant: a history from before per-ticket `when_unsure` reads as "use
    // the profile", and the latest ticket-scope choice wins. A project-scope
    // event never sets it, so one ticket cannot change another.
    #[test]
    fn the_latest_ticket_when_unsure_wins_and_old_logs_have_none() {
        let old = r#"{"type":"settings_overridden","scope":"ticket","changes":[]}"#;
        let parsed: Event = serde_json::from_str(old).unwrap();
        let mut history = vec![record(1, None, parsed)];
        assert_eq!(when_unsure_for_ticket(&history), None);
        let set = |seq, scope, mode| {
            record(
                seq,
                None,
                Event::SettingsOverridden {
                    scope,
                    changes: vec![],
                    when_unsure: Some(mode),
                },
            )
        };
        history.push(set(2, SettingsScope::Ticket, UnsureMode::DecideAndTell));
        history.push(set(3, SettingsScope::Project, UnsureMode::Ask));
        assert_eq!(
            when_unsure_for_ticket(&history),
            Some(UnsureMode::DecideAndTell)
        );
        history.push(set(4, SettingsScope::Ticket, UnsureMode::Ask));
        assert_eq!(when_unsure_for_ticket(&history), Some(UnsureMode::Ask));
    }
}
