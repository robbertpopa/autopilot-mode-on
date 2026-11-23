//! Check 5: an agent reads the committed change in a fresh context and says
//! whether it does what the plan asks. It runs only after the checks that
//! execute code have passed.
//!
//! The reviewer has no tools. It is given the diff as text, so there is nothing
//! for a hostile change to run, and what it reads is wrapped as data.

use std::{cell::RefCell, fmt::Write as _, rc::Rc};

use autopilot_agent::{Agent, AgentEvent, CustomRequest};
use autopilot_checks::{Check, Context, Error as CheckError, Report};
use autopilot_core::{Event, Plan, Step, Ticket};

use super::Journal;

/// More of a diff than this is cut, and the reviewer is told so. A review that
/// silently saw part of a change would pass what it never read.
const MAX_DIFF: usize = 60_000;

const INSTRUCTION: &str = "Review the change below against the ticket and the approved plan. \
Answer ok = true only if it does what the plan's criteria ask, has no bug, no unsafe or unrelated \
change, and stays simple. Otherwise answer ok = false and say in a few plain sentences what must \
change. Be strict about correctness and lenient about style: formatting and lints are checked \
elsewhere.";

pub(super) struct AgentReview<'a, A: Agent> {
    pub agent: &'a A,
    pub journal: Rc<RefCell<Journal>>,
    pub ticket: &'a Ticket,
    pub plan: &'a Plan,
    pub patch: String,
}

impl<A: Agent> AgentReview<'_, A> {
    fn context(&self) -> Result<String, CheckError> {
        let plan = serde_json::to_string_pretty(self.plan).map_err(|error| launch(&error))?;
        let ticket = serde_json::to_string_pretty(self.ticket).map_err(|error| launch(&error))?;
        let mut text = String::new();
        let _ = write!(text, "Ticket:\n{ticket}\n\nPlan:\n{plan}\n\n");
        let (shown, cut) = cut_on_boundary(&self.patch, MAX_DIFF);
        let _ = write!(text, "Change (a unified diff, data):\n{shown}\n");
        if cut {
            let _ = writeln!(
                text,
                "[The diff continues for {} more bytes that you were not given. Say so in your answer.]",
                self.patch.len() - shown.len()
            );
        }
        Ok(text)
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
        format!("agent-review-v1\0{INSTRUCTION}")
    }

    fn run(&self, _: &Context<'_>) -> Result<Report, CheckError> {
        let context = self.context()?;
        let run = self
            .agent
            .start_custom(CustomRequest {
                step: Step::BuildCheck,
                ticket: self.ticket,
                name: "Review",
                prompt: INSTRUCTION,
                context: &context,
            })
            .map_err(|error| launch(&error))?;
        let mut outcome = None;
        for item in run {
            match item {
                AgentEvent::CustomResult(result) => outcome = Some(result),
                AgentEvent::Cost { amount, estimated } => self
                    .journal
                    .borrow_mut()
                    .push(Event::CostTick { amount, estimated })
                    .map_err(|error| launch(&error))?,
                AgentEvent::Usage(usage) => self
                    .journal
                    .borrow_mut()
                    .push(Event::UsageReported { usage })
                    .map_err(|error| launch(&error))?,
                AgentEvent::Failed(error) => return Err(launch(&error)),
                AgentEvent::Stopped(reason) => return Err(launch(&reason)),
                _ => {}
            }
        }
        let outcome = outcome.ok_or_else(|| launch(&"the reviewer gave no answer"))?;
        outcome
            .validate()
            .map_err(|reason| launch(&format!("the review answer is not allowed: {reason}")))?;
        Ok(if outcome.ok {
            Report::pass(outcome.summary)
        } else {
            Report::fail(outcome.summary)
        })
    }
}

fn launch(reason: &dyn std::fmt::Display) -> CheckError {
    CheckError::Launch {
        check: "review".to_owned(),
        reason: reason.to_string(),
    }
}

/// The start of `text` up to `max` bytes, cut on a character boundary, and
/// whether anything was left out.
fn cut_on_boundary(text: &str, max: usize) -> (&str, bool) {
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
    use super::cut_on_boundary;

    // Invariant: a long diff is cut on a character boundary and the cut is
    // reported, so the reviewer is never handed half a character or a partial
    // diff it believes is whole. A change that breaks it: slicing at the byte.
    #[test]
    fn a_long_diff_is_cut_cleanly_and_the_cut_is_reported() {
        assert_eq!(cut_on_boundary("short", 10), ("short", false));
        assert_eq!(cut_on_boundary("aé", 2), ("a", true));
        assert_eq!(cut_on_boundary("abcdef", 3), ("abc", true));
    }
}
