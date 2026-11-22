//! What each step and each pipeline used, read from the recorded history.
//! Nothing here is guessed: a figure an adapter did not report is missing,
//! and a count says how many runs reported and how many did not.

use std::path::Path;

use autopilot_core::{Money, Step, UsageReport};

use crate::{Error, TicketView, load_ticket, tickets};

/// One reported usage figure, for the window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsageLine {
    pub step: Option<Step>,
    pub at_ms: u64,
    pub usage: UsageReport,
}

/// A step's usage over a set of tickets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepMetrics {
    pub step: Step,
    /// Tickets in which the step ran at all.
    pub runs: usize,
    /// Of those, how many had an adapter report usage.
    pub reported: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The highest context share any run reached, in thousandths, if known.
    pub peak_context_permille: Option<u32>,
    /// Exact API spend and estimated subscription spend, kept apart.
    pub api: Money,
    pub subscription: Money,
    /// Build rounds summed over the runs (Build only; zero elsewhere).
    pub rounds: u32,
    /// Runs whose first round passed every check (Build only).
    pub first_round_passed: usize,
}

/// The metrics of the last runs of one pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipelineMetrics {
    pub pipeline: String,
    pub tickets: usize,
    pub steps: Vec<StepMetrics>,
}

fn pipeline_of(view: &TicketView) -> &str {
    // A ticket with no choice recorded ran before pipelines existed: `full`.
    view.pipeline
        .as_ref()
        .map_or(autopilot_profile::DEFAULT_PIPELINE, |(name, _)| {
            name.as_str()
        })
}

/// Totals for the given tickets, which must all have run the same pipeline.
#[must_use]
pub fn metrics_of(name: &str, views: &[&TicketView]) -> PipelineMetrics {
    let mut steps = Vec::new();
    for step in Step::ALL {
        let mut item = StepMetrics {
            step,
            runs: 0,
            reported: 0,
            input_tokens: 0,
            output_tokens: 0,
            peak_context_permille: None,
            api: Money::default(),
            subscription: Money::default(),
            rounds: 0,
            first_round_passed: 0,
        };
        for view in views {
            let ran = view.step(step).is_some_and(|card| {
                !matches!(
                    card.state,
                    autopilot_pipeline::StepState::Idle | autopilot_pipeline::StepState::Skipped(_)
                )
            });
            if !ran {
                continue;
            }
            item.runs += 1;
            let lines: Vec<&UsageLine> = view
                .usage
                .iter()
                .filter(|line| line.step == Some(step))
                .collect();
            if !lines.is_empty() {
                item.reported += 1;
            }
            for line in lines {
                item.input_tokens = item.input_tokens.saturating_add(line.usage.input_tokens);
                item.output_tokens = item.output_tokens.saturating_add(line.usage.output_tokens);
                item.peak_context_permille = item
                    .peak_context_permille
                    .max(line.usage.peak_context_permille());
            }
            for cost in view.costs.iter().filter(|cost| cost.step == Some(step)) {
                let total = if cost.estimated {
                    &mut item.subscription
                } else {
                    &mut item.api
                };
                *total = total.checked_add(cost.amount).unwrap_or(*total);
            }
            if step == Step::BuildCheck {
                item.rounds = item
                    .rounds
                    .saturating_add(u32::try_from(view.rounds.len()).unwrap_or(u32::MAX));
                if view
                    .rounds
                    .first()
                    .is_some_and(|round| round.failure().is_none())
                {
                    item.first_round_passed += 1;
                }
            }
        }
        steps.push(item);
    }
    PipelineMetrics {
        pipeline: name.to_owned(),
        tickets: views.len(),
        steps,
    }
}

/// The metrics of the last `last` tickets that ran `pipeline`, newest first
/// by start time.
///
/// # Errors
/// Returns an error if a history cannot be read.
pub fn pipeline_metrics(
    directory: &Path,
    pipeline: &str,
    last: usize,
) -> Result<PipelineMetrics, Error> {
    let mut views = Vec::new();
    for id in tickets(directory)? {
        let view = load_ticket(directory, &id)?;
        if pipeline_of(&view) == pipeline {
            views.push(view);
        }
    }
    views.sort_by_key(|view| std::cmp::Reverse(view.started_ms));
    views.truncate(last);
    let refs: Vec<&TicketView> = views.iter().collect();
    Ok(metrics_of(pipeline, &refs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopilot_core::{AttemptId, Event, NewEvent, RecordedEvent, TicketId};

    fn at(seq: u64, step: Option<Step>, kind: Event) -> RecordedEvent {
        RecordedEvent {
            ticket: TicketId::from("t"),
            seq,
            event: NewEvent {
                at_ms: seq * 1000,
                step,
                kind,
            },
        }
    }

    fn ran(usage: Option<UsageReport>) -> TicketView {
        let understand = Some(Step::Understand);
        let mut history = vec![
            at(1, None, Event::TicketCreated { title: "t".into() }),
            at(
                2,
                understand,
                Event::StepStarted {
                    attempt: AttemptId::from("understand-1"),
                    session: None,
                },
            ),
        ];
        if let Some(usage) = usage {
            history.push(at(3, understand, Event::UsageReported { usage }));
        }
        history.push(at(4, understand, Event::StepFinished));
        crate::ticket_view(&history).unwrap()
    }

    // Invariant: a run whose adapter reported nothing is counted as a run but
    // not as reported, and adds no tokens, so a missing figure is never shown
    // as zero. The window can then print "not reported" for it.
    #[test]
    fn a_run_with_no_report_counts_as_not_reported() {
        let reported = ran(Some(UsageReport {
            input_tokens: 100,
            output_tokens: 20,
            peak_context_tokens: Some(800),
            context_window_tokens: Some(1_000),
            ..UsageReport::default()
        }));
        let silent = ran(None);
        let all = metrics_of("full", &[&reported, &silent]);
        let understand = &all.steps[0];
        assert_eq!(understand.step, Step::Understand);
        assert_eq!((understand.runs, understand.reported), (2, 1));
        assert_eq!(understand.input_tokens, 100);
        assert_eq!(understand.peak_context_permille, Some(800));
        let none = metrics_of("full", &[&silent]);
        assert_eq!(none.steps[0].peak_context_permille, None);
        // Steps that did not run have no runs.
        assert_eq!(all.steps[1].runs, 0);
    }

    // Invariant: a log written before usage existed still folds into a view
    // with no usage lines, and a ticket with no pipeline choice is `full`.
    #[test]
    fn an_old_history_has_no_usage_and_counts_as_full() {
        let old = ran(None);
        assert_eq!(old.usage.len(), 0);
        assert_eq!(pipeline_of(&old), "full");
        let saved = r#"{"type":"usage_reported","usage":{"input_tokens":1,"output_tokens":2,"cache_read_tokens":0,"cache_write_tokens":0}}"#;
        let event: Event = serde_json::from_str(saved).unwrap();
        assert!(matches!(
            event,
            Event::UsageReported { usage } if usage.peak_context_tokens.is_none()
        ));
    }
}
