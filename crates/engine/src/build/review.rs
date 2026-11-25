//! Check 5: several focused reviewers read the committed change, each in a
//! fresh context, and the findings they are sure of fail the check. It runs only
//! after the checks that execute code have passed.
//!
//! One reviewer asked "is this right?" finds what it happens to notice. Four
//! asked narrower questions (defects, the owner's rules, tests that would not
//! notice a break, swallowed errors) cover different ground, and each reports
//! how sure it is, so a doubtful remark does not send the agent round again.
//! The reviewers have no tools: they are given the diff as text, so there is
//! nothing for a hostile change to run, and everything they read is data.

use std::{cell::Cell, cell::RefCell, collections::HashMap, fmt::Write as _, rc::Rc};

use autopilot_agent::{Agent, AgentEvent, ReviewRequest};
use autopilot_checks::{Check, Context, Error as CheckError, Report};
use autopilot_core::{Event, FindingKind, Money, Plan, ReviewFinding, Ticket};

use super::Journal;

/// More of a diff than this is cut, and the reviewers are told so. A review that
/// silently saw part of a change would pass what it never read.
pub(crate) const MAX_DIFF: usize = 60_000;

/// A finding below this confidence is not acted on. A fixed number, not a
/// setting: a gate that each project can lower is not a gate.
pub(crate) const THRESHOLD: u8 = 80;

/// How many findings the note carries. The rest are counted, not listed.
const SHOWN: usize = 10;

struct Reviewer {
    name: &'static str,
    /// Only the rules reviewer reads the owner's rules, and it is skipped when
    /// there are none: there is nothing to check against.
    needs_rules: bool,
    instruction: &'static str,
}

const REVIEWERS: &[Reviewer] = &[
    Reviewer {
        name: "defects",
        needs_rules: false,
        instruction: "Look only for defects in what this change does: wrong results, edge cases it \
            misses, unsafe handling of input, and assumptions about callers that do not hold. \
            Ignore style, naming and anything that was already there before the change.",
    },
    Reviewer {
        name: "rules",
        needs_rules: true,
        instruction: "Check the change against the project owner's rules given below. Report only a \
            clear departure from a rule you can name, and quote the rule in the issue.",
    },
    Reviewer {
        name: "tests",
        needs_rules: false,
        instruction: "Look at whether the tests this change adds or edits would fail if the \
            behaviour they cover were broken. Report behaviour that changed but that no test \
            exercises, and tests that assert nothing meaningful.",
    },
    Reviewer {
        name: "errors",
        needs_rules: false,
        instruction: "Look for errors that are swallowed, hidden or replaced by a default value: \
            ignored results, empty handlers, fallbacks that make a failure look like success, \
            and failures that are not passed on to the caller.",
    },
];

pub(super) struct AgentReview<'a, A: Agent> {
    pub agent: &'a A,
    pub journal: Rc<RefCell<Journal>>,
    pub ticket: &'a Ticket,
    pub plan: &'a Plan,
    pub patch: String,
    /// The owner's rules block, if the project has one. It comes from the data
    /// folder, which the agent cannot write.
    pub rules: Option<&'a str>,
    /// What the reviewers spent, for the step's dollar cap.
    pub spent: Rc<Cell<Money>>,
}

impl<A: Agent> AgentReview<'_, A> {
    fn context(&self, with_rules: bool) -> Result<String, CheckError> {
        let plan = serde_json::to_string_pretty(self.plan).map_err(|error| launch(&error))?;
        let mut text = String::new();
        let _ = write!(text, "Plan:\n{plan}\n\n");
        if with_rules && let Some(rules) = self.rules {
            let _ = writeln!(text, "{rules}");
        }
        text.push_str(&diff_text(&self.patch));
        Ok(text)
    }

    fn charge(&self, amount: Money, estimated: bool) -> Result<(), CheckError> {
        self.spent.set(
            self.spent
                .get()
                .checked_add(amount)
                .unwrap_or(self.spent.get()),
        );
        self.journal
            .borrow_mut()
            .push(Event::CostTick { amount, estimated })
            .map_err(|error| launch(&error))
    }
}

impl<A: Agent> Check for AgentReview<'_, A> {
    fn id(&self) -> &'static str {
        "review"
    }

    fn applies(&self, _: &Context<'_>) -> bool {
        true
    }

    fn can_refine(&self) -> bool {
        true
    }

    fn configuration(&self) -> String {
        let mut text = format!("agent-review-v2\0threshold={THRESHOLD}");
        for reviewer in REVIEWERS {
            let _ = write!(text, "\0{}={}", reviewer.name, reviewer.instruction);
        }
        if let Some(rules) = self.rules {
            let _ = write!(text, "\0rules={rules}");
        }
        text
    }

    fn run(&self, _: &Context<'_>) -> Result<Report, CheckError> {
        let (found, asked) = run_reviewers(
            self.agent,
            self.ticket,
            self.rules.is_some(),
            |with_rules| self.context(with_rules),
            |event| match event {
                AgentEvent::Cost { amount, estimated } => self.charge(*amount, *estimated),
                AgentEvent::Usage(usage) => self
                    .journal
                    .borrow_mut()
                    .push(Event::UsageReported { usage: *usage })
                    .map_err(|error| launch(&error)),
                _ => Ok(()),
            },
        )?;
        Ok(verdict(found, asked))
    }
}

/// The diff as the reviewers are given it, cut if it is long, with the cut
/// stated. A review that silently saw part of a change would pass what it never
/// read.
pub(crate) fn diff_text(patch: &str) -> String {
    let (shown, cut) = cut_on_boundary(patch, MAX_DIFF);
    let mut text = format!("Change (a unified diff, data):\n{shown}\n");
    if cut {
        let _ = writeln!(
            text,
            "[The diff continues for {} more bytes that you were not given. Say so in a finding if it matters.]",
            patch.len() - shown.len()
        );
    }
    text
}

/// Run every reviewer that applies and collect what they report. `context`
/// builds the text one reviewer reads, given whether it may see the rules;
/// `on_event` sees each event of a run, for cost and usage. Returns the
/// findings and how many reviewers were asked.
pub(crate) fn run_reviewers<A: Agent>(
    agent: &A,
    ticket: &Ticket,
    has_rules: bool,
    context: impl Fn(bool) -> Result<String, CheckError>,
    mut on_event: impl FnMut(&AgentEvent) -> Result<(), CheckError>,
) -> Result<(Vec<ReviewFinding>, usize), CheckError> {
    // Start every reviewer before reading any: each run has a thread of its
    // own, so they work side by side while this one waits on the first.
    let mut runs = Vec::new();
    for reviewer in REVIEWERS {
        if reviewer.needs_rules && !has_rules {
            continue;
        }
        let context = context(reviewer.needs_rules)?;
        let run = agent
            .start_review(ReviewRequest {
                ticket,
                instruction: reviewer.instruction,
                context: &context,
            })
            .map_err(|error| launch(&format!("the {} reviewer: {error}", reviewer.name)))?;
        runs.push((reviewer.name, run));
    }

    let mut found: Vec<ReviewFinding> = Vec::new();
    let mut asked = 0;
    for (name, run) in runs {
        asked += 1;
        let mut report = None;
        for item in run {
            on_event(&item)?;
            match item {
                AgentEvent::ReviewResult(result) => report = Some(result),
                AgentEvent::Failed(error) => {
                    return Err(launch(&format!("the {name} reviewer: {error}")));
                }
                AgentEvent::Stopped(reason) => {
                    return Err(launch(&format!("the {name} reviewer stopped: {reason}")));
                }
                _ => {}
            }
        }
        let report =
            report.ok_or_else(|| launch(&format!("the {name} reviewer gave no answer")))?;
        report
            .validate()
            .map_err(|reason| launch(&format!("the {name} reviewer: {reason}")))?;
        found.extend(report.findings);
    }
    Ok((found, asked))
}

/// The findings at or above the threshold, one per place, most sure first.
pub(crate) fn confident(found: Vec<ReviewFinding>) -> Vec<ReviewFinding> {
    let mut best: HashMap<(String, u32), ReviewFinding> = HashMap::new();
    for finding in found.into_iter().filter(|f| f.confidence >= THRESHOLD) {
        let key = (finding.path.clone(), finding.line);
        match best.get(&key) {
            Some(held) if held.confidence >= finding.confidence => {}
            _ => {
                best.insert(key, finding);
            }
        }
    }
    let mut sure: Vec<ReviewFinding> = best.into_values().collect();
    sure.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    sure
}

/// Turn what the reviewers found into the check's result: the confident
/// findings, one per place, most sure first.
fn verdict(found: Vec<ReviewFinding>, asked: usize) -> Report {
    let total = found.len();
    let sure = confident(found);
    let doubtful = total - sure.len().min(total);
    if sure.is_empty() {
        let mut note =
            format!("{asked} reviewers read the change and found nothing at {THRESHOLD}% or above");
        if doubtful > 0 {
            let _ = write!(note, " ({doubtful} lower-confidence remarks not acted on)");
        }
        return Report::pass(note);
    }
    let mut note = format!(
        "{} finding(s) at {THRESHOLD}% or above from {asked} reviewers:",
        sure.len()
    );
    for finding in sure.iter().take(SHOWN) {
        let _ = write!(
            note,
            "\n{}:{} [{}, {}%] {}",
            finding.path,
            finding.line,
            kind_name(finding.kind),
            finding.confidence,
            finding.issue.trim()
        );
    }
    if sure.len() > SHOWN {
        let _ = write!(note, "\nand {} more", sure.len() - SHOWN);
    }
    Report::fail(note)
}

pub(crate) fn kind_name(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Bug => "bug",
        FindingKind::Rules => "rules",
        FindingKind::Tests => "tests",
        FindingKind::ErrorHandling => "error handling",
        FindingKind::Other => "other",
    }
}

pub(crate) fn launch(reason: &dyn std::fmt::Display) -> CheckError {
    CheckError::Launch {
        check: "review".to_owned(),
        reason: reason.to_string(),
    }
}

/// The start of `text` up to `max` bytes, cut on a character boundary, and
/// whether anything was left out.
pub(crate) fn cut_on_boundary(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

#[cfg(test)]
mod tests {
    use autopilot_core::{FindingKind, ReviewFinding};

    use super::{THRESHOLD, cut_on_boundary, verdict};

    fn finding(path: &str, line: u32, confidence: u8, issue: &str) -> ReviewFinding {
        ReviewFinding {
            path: path.into(),
            line,
            issue: issue.into(),
            kind: FindingKind::Bug,
            confidence,
        }
    }

    // Invariant: a long diff is cut on a character boundary and the cut is
    // reported, so a reviewer is never handed half a character or a partial
    // diff it believes is whole. A change that breaks it: slicing at the byte.
    #[test]
    fn a_long_diff_is_cut_cleanly_and_the_cut_is_reported() {
        assert_eq!(cut_on_boundary("short", 10), ("short", false));
        assert_eq!(cut_on_boundary("aé", 2), ("a", true));
        assert_eq!(cut_on_boundary("abcdef", 3), ("abc", true));
    }

    // Invariant: only findings at the threshold fail the check, the same place
    // reported by two reviewers counts once at the higher confidence, and the
    // note names each as `path:line`. A change that breaks it: failing on any
    // finding (every doubtful remark sends the agent round again), or listing a
    // place twice.
    #[test]
    fn only_confident_findings_fail_and_a_place_counts_once() {
        let report = verdict(
            vec![
                finding("src/a.rs", 3, 85, "first reading"),
                finding("src/a.rs", 3, 95, "second reading"),
                finding("src/b.rs", 9, THRESHOLD, "at the line"),
                finding("src/c.rs", 1, THRESHOLD - 1, "just under"),
            ],
            4,
        );
        assert!(!report.passed());
        assert!(report.note.starts_with("2 finding(s)"), "{}", report.note);
        let a = report
            .note
            .find("src/a.rs:3 [bug, 95%] second reading")
            .unwrap();
        let b = report
            .note
            .find("src/b.rs:9 [bug, 80%] at the line")
            .unwrap();
        assert!(a < b, "most sure first: {}", report.note);
        assert!(!report.note.contains("src/c.rs"), "{}", report.note);
    }

    // Invariant: remarks under the threshold pass the check and are counted, so
    // the owner can see they were heard and not acted on.
    #[test]
    fn doubtful_remarks_pass_and_are_counted() {
        let report = verdict(vec![finding("src/a.rs", 3, 40, "might be slow")], 4);
        assert!(report.passed());
        assert!(report.note.contains("4 reviewers"), "{}", report.note);
        assert!(
            report.note.contains("1 lower-confidence"),
            "{}",
            report.note
        );
        assert!(verdict(vec![], 3).passed());
    }

    // Invariant: the note is bounded however many findings there are.
    #[test]
    fn a_long_list_is_cut_and_counted() {
        let many: Vec<ReviewFinding> = (1..=15)
            .map(|n| finding("src/a.rs", n, 90, "problem"))
            .collect();
        let report = verdict(many, 4);
        assert!(report.note.contains("15 finding(s)"));
        assert!(report.note.contains("and 5 more"), "{}", report.note);
        assert_eq!(report.note.matches("src/a.rs:").count(), 10);
    }
}
