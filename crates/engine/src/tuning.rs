//! Automatic tuning of a custom pipeline, tier 1 only: it can lower how hard an
//! early step thinks, which lowers tokens, and nothing else. Built-in
//! pipelines, Design, the pull request, `ask_after`, budgets and
//! `protected_paths` are out of its reach, so nothing here can loosen a safety
//! rule.
//!
//! A tuning is a line in `pipeline-tuning.jsonl`. It sits *below* the project
//! and ticket overrides in `effective_overrides`, so an explicit choice by the
//! owner always wins, and removing the line (Undo) restores the profile's own
//! value. The record is append-only, so the log rebuilds the overlay.
//!
//! The guard: once three runs of the pipeline have settled after a tuning,
//! they are compared with the three before it. If more of them failed or were
//! interrupted, the tuning reverts itself and says why. First-check results
//! and send-backs are not in the ticket view yet, so the guard uses what is:
//! whether the step settled as passed.

use std::{fs, io, io::Write as _, path::Path};

use autopilot_core::Step;
use autopilot_pipeline::StepState;
use autopilot_profile::{Profile, ReviewMode};
use serde::{Deserialize, Serialize};

use crate::{Error, Finding, TicketView, review::pipeline_of};

const FILE: &str = "pipeline-tuning.jsonl";
/// Runs compared on each side of a tuning.
const GUARD_RUNS: usize = 3;
/// Lowest to highest. A value outside this list is never touched.
const EFFORT: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// Without an effort in the profile the review cannot see the agent's own
/// default, so it sets this one and says so in the evidence.
const WHEN_UNSET: &str = "medium";
/// Only the early, read-and-report steps. Later steps change code or the
/// repository, where saving tokens is not worth a quality risk.
const TUNABLE: [Step; 2] = [Step::Understand, Step::Clarify];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Tuning {
    pub pipeline: String,
    pub step: Step,
    /// The profile's value before; `None` when the profile set none.
    pub effort_before: Option<String>,
    pub effort_after: String,
    /// The finding that caused it, in one line.
    pub why: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Line {
    Applied {
        tuning: Tuning,
    },
    Undone {
        at_ms: u64,
        pipeline: String,
        step: Step,
    },
    Reverted {
        at_ms: u64,
        pipeline: String,
        step: Step,
        reason: String,
    },
    /// The guard compared the runs and the tuning stays.
    Kept {
        at_ms: u64,
        pipeline: String,
        step: Step,
    },
}

impl Line {
    fn target(&self) -> (&str, Step) {
        match self {
            Self::Applied { tuning } => (&tuning.pipeline, tuning.step),
            Self::Undone { pipeline, step, .. }
            | Self::Reverted { pipeline, step, .. }
            | Self::Kept { pipeline, step, .. } => (pipeline, *step),
        }
    }
}

/// A tuning and whether the guard still watches it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveTuning {
    pub tuning: Tuning,
    pub guarded: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuningOutcome {
    pub applied: Vec<Tuning>,
    /// Tunings the guard reverted, with the reason.
    pub reverted: Vec<(Tuning, String)>,
}

fn read_lines(directory: &Path) -> Result<Vec<Line>, Error> {
    let text = match fs::read_to_string(directory.join(FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    // A damaged line is skipped: a lost line costs one tuning at worst, while
    // an error here would stop every step from starting.
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Line>(line).ok())
        .collect())
}

fn append(directory: &Path, lines: &[Line]) -> Result<(), Error> {
    if lines.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(directory)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(FILE))?;
    for line in lines {
        let text =
            serde_json::to_string(line).map_err(|error| Error::History(error.to_string()))?;
        writeln!(file, "{text}")?;
    }
    Ok(())
}

fn replay(lines: &[Line]) -> Vec<ActiveTuning> {
    let mut active: Vec<ActiveTuning> = Vec::new();
    for line in lines {
        match line {
            Line::Applied { tuning } => {
                active.retain(|each| {
                    (each.tuning.pipeline.as_str(), each.tuning.step) != line.target()
                });
                active.push(ActiveTuning {
                    tuning: tuning.clone(),
                    guarded: true,
                });
            }
            Line::Kept { .. } => {
                for each in &mut active {
                    if (each.tuning.pipeline.as_str(), each.tuning.step) == line.target() {
                        each.guarded = false;
                    }
                }
            }
            Line::Undone { .. } | Line::Reverted { .. } => {
                active.retain(|each| {
                    (each.tuning.pipeline.as_str(), each.tuning.step) != line.target()
                });
            }
        }
    }
    active
}

/// The tunings in force now.
///
/// # Errors
/// Returns an error if the record exists but cannot be read.
pub fn active_tunings(directory: &Path) -> Result<Vec<ActiveTuning>, Error> {
    Ok(replay(&read_lines(directory)?))
}

/// The effort a tuning sets for this pipeline and step, if one is in force.
///
/// # Errors
/// Returns an error if the record exists but cannot be read.
pub fn tuned_effort(directory: &Path, pipeline: &str, step: Step) -> Result<Option<String>, Error> {
    Ok(active_tunings(directory)?
        .into_iter()
        .find(|each| each.tuning.pipeline == pipeline && each.tuning.step == step)
        .map(|each| each.tuning.effort_after))
}

/// Remove a tuning. The profile's own value applies again at the next start.
///
/// # Errors
/// Returns an error if the record cannot be read or written.
pub fn undo_tuning(directory: &Path, pipeline: &str, step: Step) -> Result<bool, Error> {
    let found = active_tunings(directory)?
        .iter()
        .any(|each| each.tuning.pipeline == pipeline && each.tuning.step == step);
    if found {
        append(
            directory,
            &[Line::Undone {
                at_ms: crate::timestamp()?,
                pipeline: pipeline.to_owned(),
                step,
            }],
        )?;
    }
    Ok(found)
}

/// The next lower effort, or `None` when it is already the lowest or is a
/// value this list does not know.
pub(crate) fn lower(effort: Option<&str>) -> Option<&'static str> {
    let Some(effort) = effort else {
        return Some(WHEN_UNSET);
    };
    let at = EFFORT.iter().position(|known| *known == effort)?;
    at.checked_sub(1).map(|below| EFFORT[below])
}

fn step_passed(view: &TicketView, step: Step) -> Option<bool> {
    match view.step(step)?.state {
        StepState::Passed => Some(true),
        StepState::Failed(_) | StepState::Interrupted(_) => Some(false),
        _ => None,
    }
}

/// Settled runs of one pipeline's step, oldest first, as passed or not.
fn settled_runs(views: &[&TicketView], pipeline: &str, step: Step) -> Vec<(u64, bool)> {
    let mut runs: Vec<(u64, bool)> = views
        .iter()
        .filter(|view| pipeline_of(view) == pipeline)
        .filter_map(|view| step_passed(view, step).map(|passed| (view.started_ms, passed)))
        .collect();
    runs.sort_unstable();
    runs
}

#[derive(Debug, Eq, PartialEq)]
enum Verdict {
    /// Fewer than three runs have settled since the change.
    Waiting,
    Keep,
    Revert(String),
}

fn guard_verdict(runs: &[(u64, bool)], applied_ms: u64) -> Verdict {
    let after: Vec<bool> = runs
        .iter()
        .filter(|(started, _)| *started >= applied_ms)
        .map(|(_, passed)| *passed)
        .take(GUARD_RUNS)
        .collect();
    if after.len() < GUARD_RUNS {
        return Verdict::Waiting;
    }
    let before: Vec<bool> = runs
        .iter()
        .filter(|(started, _)| *started < applied_ms)
        .map(|(_, passed)| *passed)
        .rev()
        .take(GUARD_RUNS)
        .collect();
    let failures = |runs: &[bool]| runs.iter().filter(|passed| !**passed).count();
    let (now, then) = (failures(&after), failures(&before));
    if now > then {
        return Verdict::Revert(format!(
            "{now} of the {GUARD_RUNS} runs after the change did not pass, against {then} of the {} before",
            before.len()
        ));
    }
    Verdict::Keep
}

/// Apply tier 1 changes for the findings that just opened, and let the guard
/// judge the tunings already in force. Does nothing unless the review mode is
/// Notify and tune.
///
/// # Errors
/// Returns an error if a ticket history or the record cannot be read or written.
pub fn run_tuning(
    directory: &Path,
    profile: &Profile,
    opened: &[Finding],
    views: &[TicketView],
) -> Result<TuningOutcome, Error> {
    let mut outcome = TuningOutcome::default();
    if profile.review.mode != ReviewMode::NotifyAndTune {
        return Ok(outcome);
    }
    let refs: Vec<&TicketView> = views.iter().collect();
    let now = crate::timestamp()?;
    let mut lines = Vec::new();

    for each in active_tunings(directory)?.into_iter().filter(|e| e.guarded) {
        let tuning = each.tuning;
        let runs = settled_runs(&refs, &tuning.pipeline, tuning.step);
        match guard_verdict(&runs, tuning.at_ms) {
            Verdict::Waiting => {}
            Verdict::Keep => lines.push(Line::Kept {
                at_ms: now,
                pipeline: tuning.pipeline.clone(),
                step: tuning.step,
            }),
            Verdict::Revert(reason) => {
                lines.push(Line::Reverted {
                    at_ms: now,
                    pipeline: tuning.pipeline.clone(),
                    step: tuning.step,
                    reason: reason.clone(),
                });
                outcome.reverted.push((tuning, reason));
            }
        }
    }

    let in_force = {
        let mut lines_so_far = read_lines(directory)?;
        lines_so_far.extend(lines.iter().cloned());
        replay(&lines_so_far)
    };
    for finding in opened {
        if !TUNABLE.contains(&finding.step) {
            continue;
        }
        // Built-in pipelines are read-only, and an unknown name is a ticket
        // that ran an older pipeline: neither is ours to change.
        let Some(pipeline) = profile.pipeline(&finding.pipeline) else {
            continue;
        };
        if pipeline.builtin {
            continue;
        }
        // One tuning per step at a time; the second waits for the guard.
        if in_force.iter().any(|each| {
            each.tuning.pipeline == finding.pipeline && each.tuning.step == finding.step
        }) || outcome
            .applied
            .iter()
            .any(|each| each.pipeline == finding.pipeline && each.step == finding.step)
        {
            continue;
        }
        let before = profile
            .step(finding.step)
            .and_then(|settings| settings.effort.clone());
        let Some(after) = lower(before.as_deref()) else {
            continue;
        };
        let tuning = Tuning {
            pipeline: finding.pipeline.clone(),
            step: finding.step,
            effort_before: before,
            effort_after: after.to_owned(),
            why: format!("{}: {}", finding.rule.label(), finding.evidence),
            at_ms: now,
        };
        lines.push(Line::Applied {
            tuning: tuning.clone(),
        });
        outcome.applied.push(tuning);
    }
    append(directory, &lines)?;
    Ok(outcome)
}

/// `run_tuning` over every ticket in the data directory.
///
/// # Errors
/// Returns an error if a ticket history or the record cannot be read or written.
pub fn tune_directory(
    directory: &Path,
    profile: &Profile,
    opened: &[Finding],
) -> Result<TuningOutcome, Error> {
    if profile.review.mode != ReviewMode::NotifyAndTune {
        return Ok(TuningOutcome::default());
    }
    let mut views = Vec::new();
    for id in crate::tickets(directory)? {
        views.push(crate::load_ticket(directory, &id)?);
    }
    run_tuning(directory, profile, opened, &views)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: effort only ever goes down one notch, an unknown value is
    // left alone, and the lowest is not lowered. A tuning that raised effort
    // or guessed at an unknown name would spend more or break a step, and no
    // other test looks at the ladder.
    #[test]
    fn effort_goes_down_one_notch_and_never_past_the_floor() {
        assert_eq!(lower(Some("high")), Some("medium"));
        assert_eq!(lower(Some("max")), Some("xhigh"));
        assert_eq!(lower(Some("low")), None);
        assert_eq!(lower(Some("turbo")), None);
        assert_eq!(lower(None), Some(WHEN_UNSET));
    }

    // Invariant: the guard waits for three runs after the change, and reverts
    // only when more of them failed than of the three before. Reverting on one
    // bad run would undo good tunings; never reverting would keep a harmful one.
    #[test]
    fn the_guard_waits_for_three_runs_and_reverts_only_on_a_drop() {
        let before = [(1, true), (2, true), (3, true)];
        let two_after = [(10, false), (11, false)];
        let runs: Vec<_> = before.iter().chain(&two_after).copied().collect();
        assert_eq!(
            guard_verdict(&runs, 5),
            Verdict::Waiting,
            "two runs are too few"
        );

        let worse: Vec<_> = before
            .iter()
            .chain(&[(10, false), (11, true), (12, true)])
            .copied()
            .collect();
        assert!(matches!(guard_verdict(&worse, 5), Verdict::Revert(_)));

        let same: Vec<_> = before
            .iter()
            .chain(&[(10, true), (11, true), (12, true)])
            .copied()
            .collect();
        assert_eq!(guard_verdict(&same, 5), Verdict::Keep);
    }

    // Invariant: the record rebuilds the overlay, and Undo or a revert removes
    // exactly one tuning. If replay kept an undone line the profile's value
    // would never come back.
    #[test]
    fn replay_applies_then_undo_removes_it() {
        let tuning = Tuning {
            pipeline: "mine".into(),
            step: Step::Understand,
            effort_before: Some("high".into()),
            effort_after: "medium".into(),
            why: "peak context".into(),
            at_ms: 5,
        };
        let applied = Line::Applied { tuning };
        assert_eq!(replay(std::slice::from_ref(&applied)).len(), 1);
        let undone = Line::Undone {
            at_ms: 6,
            pipeline: "mine".into(),
            step: Step::Understand,
        };
        assert_eq!(replay(&[applied, undone]).len(), 0);
    }

    fn finding(pipeline: &str) -> Finding {
        Finding {
            pipeline: pipeline.into(),
            step: Step::Understand,
            rule: crate::Rule::PeakContext,
            evidence: "peak context at or above 80.0% in 3 of the last 5 reported runs".into(),
            estimated: false,
        }
    }

    // Invariant: only a custom pipeline is tuned, and only while the mode is
    // Notify and tune; the overlay shows through `tuned_effort` and Undo takes
    // it away; a second finding on the same step does not stack. A tuning that
    // touched a built-in, or ran in Notify only, would change what runs without
    // the owner having asked for it, and no other test covers that path.
    #[test]
    fn only_a_custom_pipeline_is_tuned_and_undo_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let source = "[review]
mode = \"notify_and_tune\"

[steps.understand]
effort = \"high\"

[pipelines.mine]
based_on = \"bug-fix\"
";
        let profile = Profile::parse("profile.toml", source).unwrap();

        let builtin = run_tuning(dir.path(), &profile, &[finding("bug-fix")], &[]).unwrap();
        assert!(builtin.applied.is_empty(), "built-ins are read-only");

        let tuned = run_tuning(dir.path(), &profile, &[finding("mine")], &[]).unwrap();
        assert_eq!(tuned.applied.len(), 1);
        assert_eq!(tuned.applied[0].effort_before.as_deref(), Some("high"));
        assert_eq!(
            tuned_effort(dir.path(), "mine", Step::Understand).unwrap(),
            Some("medium".to_owned())
        );

        let again = run_tuning(dir.path(), &profile, &[finding("mine")], &[]).unwrap();
        assert!(again.applied.is_empty(), "one tuning per step at a time");

        assert!(undo_tuning(dir.path(), "mine", Step::Understand).unwrap());
        assert_eq!(
            tuned_effort(dir.path(), "mine", Step::Understand).unwrap(),
            None
        );

        let notify = Profile::parse(
            "profile.toml",
            "[pipelines.mine]
based_on = \"bug-fix\"
",
        )
        .unwrap();
        let off = run_tuning(dir.path(), &notify, &[finding("mine")], &[]).unwrap();
        assert!(off.applied.is_empty(), "the default mode never tunes");
    }
}
