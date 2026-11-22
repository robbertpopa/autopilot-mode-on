//! The token side of the pipeline review: findings computed from recorded
//! usage and cost, no model involved. It reads and reports; it never changes
//! the profile, a pipeline or a ticket.
//!
//! Two rules open a finding. *Peak context*: the largest prompt reached the
//! configured share of the window in enough of a step's latest runs. *Step
//! cost*: a step's latest run cost more than the configured multiple of the
//! median of its earlier runs. A finding is identified by pipeline, step and
//! rule. It is written once when it opens and once when it closes, so a
//! restart or a second pass over the same history changes and announces
//! nothing.
//!
//! False positives are the main risk, so the rules count only settled runs
//! that actually reported the figure tested, skip a run with no known window
//! instead of reading it as 0 percent, never mix exact API cost with
//! subscription estimates, and need a minimum number of runs before saying
//! anything. A ticket that changed pipeline counts under the last one it chose.

use std::{collections::BTreeSet, fs, io, io::Write as _, path::Path};

use autopilot_core::{Money, Step};
use autopilot_pipeline::StepState;
use autopilot_profile::{ReviewMode, ReviewSettings};
use serde::{Deserialize, Serialize};

use crate::{Error, TicketView, load_ticket, tickets};

const FILE: &str = "review-findings.jsonl";
/// The marker file the sample-data window writes. Sample tickets carry made-up
/// cost, so a review of them would report things that are not true.
const DEMO_MARKER: &str = "sample-data.txt";
/// Earlier runs a cost comparison needs, and how many it looks back over.
const MIN_EARLIER_RUNS: usize = 3;
const MAX_EARLIER_RUNS: usize = 20;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    PeakContext,
    StepCost,
}

impl Rule {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PeakContext => "peak context",
            Self::StepCost => "step cost",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub pipeline: String,
    pub step: Step,
    pub rule: Rule,
    /// The numbers behind it, in one line.
    pub evidence: String,
    /// True when the cost involved is a subscription estimate.
    #[serde(default)]
    pub estimated: bool,
}

impl Finding {
    /// What identifies a finding across runs of the review.
    fn key(&self) -> (String, Step, Rule) {
        (self.pipeline.clone(), self.step, self.rule)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewOutcome {
    /// Every finding that is open now.
    pub open: Vec<Finding>,
    /// Findings that opened in this pass: the only ones to announce.
    pub opened: Vec<Finding>,
    pub closed: Vec<Finding>,
}

#[derive(Deserialize, Serialize)]
struct Line {
    at_ms: u64,
    action: Action,
    finding: Finding,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Opened,
    Closed,
}

fn pipeline_of(view: &TicketView) -> &str {
    view.pipeline
        .as_ref()
        .map_or(autopilot_profile::DEFAULT_PIPELINE, |(name, _)| {
            name.as_str()
        })
}

fn settled(view: &TicketView, step: Step) -> bool {
    view.step(step).is_some_and(|card| {
        matches!(
            card.state,
            StepState::Passed | StepState::Failed(_) | StepState::Interrupted(_)
        )
    })
}

/// The highest share of the window one turn of `step` reached, if any run of
/// it reported both the peak and the window.
fn peak_share(view: &TicketView, step: Step) -> Option<u32> {
    view.usage
        .iter()
        .filter(|line| line.step == Some(step))
        .filter_map(|line| line.usage.peak_context_permille())
        .max()
}

/// A step's cost in one ticket, split by kind: exact API and estimated.
fn step_cost(view: &TicketView, step: Step) -> (Money, Money) {
    let mut api = Money::default();
    let mut estimate = Money::default();
    for cost in view.costs.iter().filter(|cost| cost.step == Some(step)) {
        let total = if cost.estimated {
            &mut estimate
        } else {
            &mut api
        };
        *total = total.checked_add(cost.amount).unwrap_or(*total);
    }
    (api, estimate)
}

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        u64::midpoint(values[middle - 1], values[middle])
    }
}

fn percent_text(permille: u32) -> String {
    format!("{}.{}%", permille / 10, permille % 10)
}

fn peak_finding(
    name: &str,
    step: Step,
    runs: &[&TicketView],
    settings: &ReviewSettings,
) -> Option<Finding> {
    let window = usize::try_from(settings.window_runs).unwrap_or(usize::MAX);
    let recent: Vec<u32> = runs
        .iter()
        .rev()
        .filter(|view| settled(view, step))
        .filter_map(|view| peak_share(view, step))
        .take(window)
        .collect();
    let needed = usize::try_from(settings.min_hits).unwrap_or(usize::MAX);
    let hits = recent
        .iter()
        .filter(|share| **share >= settings.peak_context_permille)
        .count();
    (recent.len() >= needed && hits >= needed).then(|| Finding {
        pipeline: name.to_owned(),
        step,
        rule: Rule::PeakContext,
        evidence: format!(
            "peak context at or above {} in {hits} of the last {} reported runs",
            percent_text(settings.peak_context_permille),
            recent.len()
        ),
        estimated: false,
    })
}

fn cost_finding(
    name: &str,
    step: Step,
    runs: &[&TicketView],
    settings: &ReviewSettings,
) -> Option<Finding> {
    for estimated in [false, true] {
        let costs: Vec<u64> = runs
            .iter()
            .filter(|view| settled(view, step))
            .map(|view| {
                let (api, estimate) = step_cost(view, step);
                if estimated { estimate } else { api }.as_microusd()
            })
            .filter(|micro| *micro > 0)
            .collect();
        let Some((latest, earlier)) = costs.split_last() else {
            continue;
        };
        let earlier = &earlier[earlier.len().saturating_sub(MAX_EARLIER_RUNS)..];
        if earlier.len() < MIN_EARLIER_RUNS {
            continue;
        }
        let middle = median(earlier.to_vec());
        if latest.saturating_mul(100)
            > middle.saturating_mul(u64::from(settings.cost_multiple_percent))
        {
            let kind = if estimated { " (estimated)" } else { "" };
            return Some(Finding {
                pipeline: name.to_owned(),
                step,
                rule: Rule::StepCost,
                evidence: format!(
                    "latest run cost {} against a median of {}{kind}",
                    crate::dollars(Money::from_microusd(*latest)),
                    crate::dollars(Money::from_microusd(middle))
                ),
                estimated,
            });
        }
    }
    None
}

/// The findings the given tickets support under these settings. Pure: it
/// reads nothing and writes nothing.
#[must_use]
pub fn findings_for(views: &[TicketView], settings: &ReviewSettings) -> Vec<Finding> {
    let mut sorted: Vec<&TicketView> = views.iter().collect();
    sorted.sort_by_key(|view| view.started_ms);
    let names: BTreeSet<&str> = sorted.iter().map(|view| pipeline_of(view)).collect();
    let mut found = Vec::new();
    for name in names {
        let runs: Vec<&TicketView> = sorted
            .iter()
            .copied()
            .filter(|view| pipeline_of(view) == name)
            .collect();
        for step in Step::ALL {
            found.extend(peak_finding(name, step, &runs, settings));
            found.extend(cost_finding(name, step, &runs, settings));
        }
    }
    found
}

fn read_lines(directory: &Path) -> Result<Vec<Line>, Error> {
    let text = match fs::read_to_string(directory.join(FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    // A damaged line is skipped: the file is a record for the owner, and one
    // bad line must not stop the review from working.
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Line>(line).ok())
        .collect())
}

fn replay_open(lines: &[Line]) -> Vec<Finding> {
    let mut open: Vec<Finding> = Vec::new();
    for line in lines {
        open.retain(|finding| finding.key() != line.finding.key());
        if line.action == Action::Opened {
            open.push(line.finding.clone());
        }
    }
    open
}

/// The findings that are open according to the record, without recomputing.
///
/// # Errors
/// Returns an error if the record exists but cannot be read.
pub fn open_findings(directory: &Path) -> Result<Vec<Finding>, Error> {
    Ok(replay_open(&read_lines(directory)?))
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

/// Review the data directory once. With the review off, or in a sample-data
/// directory, it computes nothing and writes nothing. Otherwise it writes only
/// the findings that opened or closed since the last pass.
///
/// # Errors
/// Returns an error if a ticket history or the record cannot be read or written.
pub fn run_review(directory: &Path, settings: &ReviewSettings) -> Result<ReviewOutcome, Error> {
    if settings.mode == ReviewMode::Off || directory.join(DEMO_MARKER).exists() {
        return Ok(ReviewOutcome::default());
    }
    let mut views = Vec::new();
    for id in tickets(directory)? {
        views.push(load_ticket(directory, &id)?);
    }
    let now = findings_for(&views, settings);
    let before = open_findings(directory)?;
    let opened: Vec<Finding> = now
        .iter()
        .filter(|finding| !before.iter().any(|old| old.key() == finding.key()))
        .cloned()
        .collect();
    let closed: Vec<Finding> = before
        .iter()
        .filter(|old| !now.iter().any(|finding| finding.key() == old.key()))
        .cloned()
        .collect();
    let at_ms = crate::timestamp()?;
    let mut lines: Vec<Line> = closed
        .iter()
        .map(|finding| Line {
            at_ms,
            action: Action::Closed,
            finding: finding.clone(),
        })
        .collect();
    lines.extend(opened.iter().map(|finding| Line {
        at_ms,
        action: Action::Opened,
        finding: finding.clone(),
    }));
    append(directory, &lines)?;
    Ok(ReviewOutcome {
        open: now,
        opened,
        closed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopilot_core::{AttemptId, Event, NewEvent, RecordedEvent, TicketId, UsageReport};

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

    /// A ticket whose Understand step ran and settled, with an optional peak
    /// share (of a 1000-token window) and an optional cost.
    fn run(index: u64, peak: Option<u64>, cost: Option<u64>, estimated: bool) -> TicketView {
        let step = Some(Step::Understand);
        let base = index * 100;
        let mut history = vec![
            at(
                base + 1,
                None,
                Event::TicketCreated {
                    title: format!("t{index}"),
                },
            ),
            at(
                base + 2,
                step,
                Event::StepStarted {
                    attempt: AttemptId::from("understand-1"),
                    session: None,
                },
            ),
        ];
        if let Some(peak) = peak {
            history.push(at(
                base + 3,
                step,
                Event::UsageReported {
                    usage: UsageReport {
                        peak_context_tokens: Some(peak),
                        context_window_tokens: Some(1000),
                        ..UsageReport::default()
                    },
                },
            ));
        }
        if let Some(cost) = cost {
            history.push(at(
                base + 4,
                step,
                Event::CostTick {
                    amount: Money::from_microusd(cost),
                    estimated,
                },
            ));
        }
        history.push(at(base + 5, step, Event::StepFinished));
        // Sequence numbers must rise within one ticket only.
        for (n, record) in history.iter_mut().enumerate() {
            record.seq = n as u64 + 1;
            record.event.at_ms = index * 1_000_000 + n as u64;
        }
        crate::ticket_view(&history).unwrap()
    }

    // Invariant: three of the last five reported runs at or over the line open
    // one finding, two do not, and a run with no window or no report is left
    // out of the count instead of being read as zero. Counting a silent run as
    // 0 percent would hide a real problem; counting it as a hit would invent
    // one.
    #[test]
    fn peak_context_needs_enough_reported_runs_over_the_line() {
        let settings = ReviewSettings::default();
        let hot = |i| run(i, Some(850), None, false);
        let cool = |i| run(i, Some(100), None, false);
        let silent = |i| run(i, None, None, false);
        let three = vec![hot(1), cool(2), hot(3), silent(4), hot(5)];
        let found = findings_for(&three, &settings);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].rule, Rule::PeakContext);
        assert!(found[0].evidence.contains("3 of the last 4 reported runs"));
        let two = vec![hot(1), cool(2), hot(3), silent(4), cool(5)];
        assert_eq!(findings_for(&two, &settings).len(), 0);
        let none = vec![silent(1), silent(2), silent(3), silent(4)];
        assert_eq!(findings_for(&none, &settings).len(), 0);
    }

    // Invariant: a cost finding compares the latest run with the median of
    // earlier ones, needs three earlier runs, and never mixes exact and
    // estimated cost.
    #[test]
    fn a_cost_finding_needs_history_and_keeps_kinds_apart() {
        let settings = ReviewSettings::default();
        let cheap = |i, estimated| run(i, None, Some(100_000), estimated);
        let dear = |i, estimated| run(i, None, Some(500_000), estimated);
        let exact = vec![
            cheap(1, false),
            cheap(2, false),
            cheap(3, false),
            dear(4, false),
        ];
        let found = findings_for(&exact, &settings);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(!found[0].estimated);
        let estimated = vec![
            cheap(1, true),
            cheap(2, true),
            cheap(3, true),
            dear(4, true),
        ];
        let found = findings_for(&estimated, &settings);
        assert!(found[0].estimated && found[0].evidence.contains("(estimated)"));
        // Too few earlier runs, or a spike made of mixed kinds, says nothing.
        let few = vec![cheap(1, false), cheap(2, false), dear(3, false)];
        assert_eq!(findings_for(&few, &settings).len(), 0);
        let mixed = vec![
            cheap(1, true),
            cheap(2, false),
            cheap(3, true),
            dear(4, false),
        ];
        assert_eq!(findings_for(&mixed, &settings).len(), 0);
    }

    // Invariant: the same history reviewed twice writes and announces nothing
    // the second time, and a finding that closes and reopens announces again.
    // This is what keeps one drift from becoming a notification every pass.
    #[test]
    fn a_finding_is_written_once_and_announced_once() {
        let data = tempfile::tempdir().unwrap();
        let first = Finding {
            pipeline: "full".into(),
            step: Step::Plan,
            rule: Rule::PeakContext,
            evidence: "e".into(),
            estimated: false,
        };
        let line = |action, finding: &Finding| Line {
            at_ms: 1,
            action,
            finding: finding.clone(),
        };
        append(data.path(), &[line(Action::Opened, &first)]).unwrap();
        assert_eq!(open_findings(data.path()).unwrap().len(), 1);
        append(data.path(), &[line(Action::Closed, &first)]).unwrap();
        assert_eq!(open_findings(data.path()).unwrap().len(), 0);
        append(data.path(), &[line(Action::Opened, &first)]).unwrap();
        assert_eq!(open_findings(data.path()).unwrap(), vec![first]);
        // A damaged line does not stop the record from being read.
        fs::write(
            data.path().join(FILE),
            format!(
                "not json\n{}\n",
                fs::read_to_string(data.path().join(FILE)).unwrap().trim()
            ),
        )
        .unwrap();
        assert_eq!(open_findings(data.path()).unwrap().len(), 1);
    }

    // Invariant: with the review off, or in a sample-data directory, nothing is
    // computed or written. Sample tickets carry made-up cost.
    #[test]
    fn off_and_sample_data_write_nothing() {
        let data = tempfile::tempdir().unwrap();
        let off = ReviewSettings::default();
        assert_eq!(
            run_review(data.path(), &off).unwrap(),
            ReviewOutcome::default()
        );
        let on = ReviewSettings {
            mode: ReviewMode::NotifyOnly,
            ..ReviewSettings::default()
        };
        fs::write(data.path().join(DEMO_MARKER), "x").unwrap();
        assert_eq!(
            run_review(data.path(), &on).unwrap(),
            ReviewOutcome::default()
        );
        assert!(!data.path().join(FILE).exists());
    }
}
