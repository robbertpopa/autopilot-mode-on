//! "In short" lines made by code from structured results. No model is asked,
//! so the line is free, instant and always agrees with the data it describes.

use autopilot_core::{Clarity, Event, RecordedEvent, Step, Understanding};

use crate::{analyze, clarify::QaPair};

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

#[must_use]
pub fn understanding(result: &Understanding) -> String {
    let a = &result.ambiguity;
    let all = [
        a.functional_scope,
        a.domain_and_data,
        a.interaction_and_ux,
        a.non_functional,
        a.integrations,
        a.edge_cases_and_failures,
        a.constraints_and_trade_offs,
        a.terminology,
        a.completion_signals,
        a.misc,
    ];
    let missing = all.iter().filter(|c| **c == Clarity::Missing).count();
    let partial = all.iter().filter(|c| **c == Clarity::Partial).count();
    let clarity = if missing + partial == 0 {
        "Everything is clear.".to_owned()
    } else {
        format!("{missing} of 10 areas missing, {partial} partial.")
    };
    let design = if result.needs_design {
        "It needs a design."
    } else {
        "No design needed."
    };
    format!(
        "{clarity} {}, {}. {design}",
        plural(result.open_questions.len(), "open question"),
        plural(result.assumptions.len(), "assumption"),
    )
}

#[must_use]
pub fn clarification(pairs: &[QaPair]) -> String {
    format!("Answered {}.", plural(pairs.len(), "question"))
}

/// What one check run says, from the fields of its log event.
#[must_use]
pub fn check_line(
    check: &str,
    passed: bool,
    note: &str,
    unknowns: &[String],
    reused: bool,
) -> String {
    let reused = if reused { " (kept from before)" } else { "" };
    if passed {
        return format!("{check} passed{reused}.");
    }
    if !unknowns.is_empty() {
        return format!("{check} cannot pass yet: {}.", unknowns.join("; "));
    }
    let first = note.lines().next().unwrap_or_default();
    format!("{check} failed: {first}")
}

/// The latest round of the build loop, from the check results after the last
/// `RoundStarted`. Checks stop at the first failure, so "ran" counts only the
/// checks that got a turn.
fn round_line(history: &[RecordedEvent]) -> Option<String> {
    let start = history
        .iter()
        .rposition(|record| matches!(record.event.kind, Event::RoundStarted { .. }))?;
    let Event::RoundStarted { round } = &history[start].event.kind else {
        return None;
    };
    let mut ran = 0_usize;
    let mut passed = 0_usize;
    let mut stopped = None;
    for record in &history[start..] {
        if let Event::CheckResult {
            check,
            passed: ok,
            note,
            unknowns,
            reused,
            ..
        } = &record.event.kind
        {
            ran += 1;
            if *ok {
                passed += 1;
            } else {
                stopped = Some(check_line(check, false, note, unknowns, *reused));
            }
        }
    }
    let mut line = format!(
        "Round {round}. {passed} of {} passed.",
        plural(ran, "check")
    );
    if let Some(stopped) = stopped {
        line.push(' ');
        line.push_str(&stopped);
    }
    Some(line)
}

/// One line per step that has a result in the history, in pipeline order.
#[must_use]
pub fn from_history(history: &[RecordedEvent]) -> Vec<(Step, String)> {
    let mut lines = Vec::new();
    let understood = history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::UnderstandResult { result } => Some(result),
            _ => None,
        });
    if let Some(result) = understood {
        lines.push((Step::Understand, understanding(result)));
    }
    let answered = history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::Answered { answer, .. } if record.event.step == Some(Step::Clarify) => {
                serde_json::from_str::<Vec<QaPair>>(answer).ok()
            }
            _ => None,
        });
    if let Some(pairs) = answered {
        lines.push((Step::Clarify, clarification(&pairs)));
    }
    let planned = history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::PlanResult { plan } => Some(plan),
            _ => None,
        });
    if let Some(plan) = planned {
        let analysis = analyze::analyze(plan);
        lines.push((
            Step::Plan,
            format!(
                "{}. Analysis: {}.",
                plural(plan.criteria.len(), "criterion").replace("criterions", "criteria"),
                analysis.describe()
            ),
        ));
    }
    if let Some(line) = round_line(history) {
        lines.push((Step::BuildCheck, line));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopilot_core::{Ambiguity, OpenQuestion};

    fn result(clarity: Clarity, questions: usize, design: bool) -> Understanding {
        Understanding {
            summary: "s".into(),
            ambiguity: Ambiguity::all(clarity),
            open_questions: (0..questions)
                .map(|_| OpenQuestion {
                    question: "q".into(),
                    options: vec!["a".into(), "b".into()],
                    recommended: 0,
                })
                .collect(),
            assumptions: vec!["x".into()],
            needs_design: design,
            ui_files: vec![],
            files_touched: vec![],
        }
    }

    // Invariant: the line is a pure template over the result, with correct
    // singular and plural wording. A snapshot of the text keeps the window and
    // the CLI saying the same thing.
    #[test]
    fn understanding_lines_follow_the_data() {
        assert_eq!(
            understanding(&result(Clarity::Clear, 0, false)),
            "Everything is clear. 0 open questions, 1 assumption. No design needed."
        );
        assert_eq!(
            understanding(&result(Clarity::Missing, 1, true)),
            "10 of 10 areas missing, 0 partial. 1 open question, 1 assumption. It needs a design."
        );
    }

    fn check_event(check: &str, passed: bool, note: &str, unknowns: &[&str]) -> Event {
        Event::CheckResult {
            check: check.into(),
            passed,
            note: note.into(),
            artifacts: vec![],
            unknowns: unknowns.iter().map(|u| (*u).into()).collect(),
            reused: false,
        }
    }

    fn record(seq: u64, kind: Event) -> RecordedEvent {
        RecordedEvent {
            ticket: "t".into(),
            seq,
            event: autopilot_core::NewEvent {
                at_ms: 0,
                step: Some(Step::BuildCheck),
                kind,
            },
        }
    }

    // Invariant: the round line describes only the latest round and names the
    // check that stopped it, with the first line of its note. It must not
    // count results from an earlier round.
    #[test]
    fn the_round_line_covers_the_latest_round_only() {
        let history = [
            record(1, Event::RoundStarted { round: 1 }),
            record(2, check_event("build", false, "error[E0425]\nmore", &[])),
            record(3, Event::RoundStarted { round: 2 }),
            record(4, check_event("build", true, "", &[])),
            record(5, check_event("test", false, "1 failed\ndetails", &[])),
        ];
        assert_eq!(
            from_history(&history),
            [(
                Step::BuildCheck,
                "Round 2. 1 of 2 checks passed. test failed: 1 failed".to_owned()
            )]
        );
    }

    #[test]
    fn a_check_line_says_when_something_cannot_pass_yet() {
        assert_eq!(
            check_line(
                "criteria",
                false,
                "",
                &["c1 has no way to be verified".into()],
                false
            ),
            "criteria cannot pass yet: c1 has no way to be verified."
        );
        assert_eq!(
            check_line("test", true, "", &[], true),
            "test passed (kept from before)."
        );
    }

    #[test]
    fn clarification_counts_answers() {
        let pair = QaPair {
            question: "q".into(),
            answer: "a".into(),
        };
        assert_eq!(
            clarification(std::slice::from_ref(&pair)),
            "Answered 1 question."
        );
        assert_eq!(
            clarification(&[pair.clone(), pair]),
            "Answered 2 questions."
        );
    }
}
