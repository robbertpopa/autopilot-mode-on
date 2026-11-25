//! The reviewers of Build, pointed at a pull request that someone else opened.
//!
//! Nothing here writes to GitHub: it reads the pull request, asks the same
//! reviewers, and returns what they found. No change is skipped for being small,
//! a draft, or already closed, because a gate that passes what it did not read
//! is not a gate.

use autopilot_agent::{Agent, AgentEvent};
use autopilot_core::{Money, ReviewFinding, Ticket, TicketId};
use autopilot_tracker::{Gh, PullState};

use crate::{
    Error,
    build::review::{MAX_DIFF, THRESHOLD, confident, cut_on_boundary, diff_text, run_reviewers},
};

#[derive(Debug, Eq, PartialEq)]
pub struct PrReview {
    pub number: u64,
    /// The commit that was read. A report is about this commit only.
    pub head: String,
    pub state: &'static str,
    /// How many reviewers were asked. The rules reviewer is left out when no
    /// rules were given.
    pub reviewers: usize,
    /// Findings at or above the threshold, one per place, most sure first.
    pub findings: Vec<ReviewFinding>,
    /// Remarks under the threshold: heard, not acted on.
    pub doubtful: usize,
    pub cost: Money,
    /// Bytes of the diff the reviewers were not given because it was too long.
    pub diff_left_out: usize,
}

/// Read pull request `number` at one exact commit and have every reviewer look
/// at it. `rules` is the owner's rules block, if the project has one.
///
/// # Errors
/// Returns an error if `gh` fails, the pull request has no changes or its head
/// moved while it was read, or a reviewer fails or gives an answer that cannot
/// be used. A failed reviewer is never counted as a clean review.
pub fn review_pull_request<A: Agent>(
    agent: &A,
    gh: &Gh,
    number: u64,
    rules: Option<&str>,
) -> Result<PrReview, Error> {
    let subject = gh.pr_subject(number)?;
    let diff = gh.pr_diff(number)?;
    // The diff is read separately from the head: if someone pushed in between,
    // the report would name a commit the reviewers never saw.
    let after = gh.pr_subject(number)?;
    if after.head != subject.head {
        return Err(Error::ReviewRefused(format!(
            "the head moved from {} to {} while it was read; run the review again",
            subject.head, after.head
        )));
    }
    if diff.trim().is_empty() {
        return Err(Error::ReviewRefused(
            "the pull request has no changes to review".into(),
        ));
    }

    // Its words are the author's, so they travel as the ticket, which every
    // reviewer prompt frames as data.
    let ticket = Ticket {
        id: TicketId::from(format!("pr-{number}")),
        title: subject.title.clone(),
        description: subject.body.clone(),
    };
    let context = |with_rules: bool| {
        let mut text = format!(
            "Pull request {number}, head commit {}. Its title and description are in the ticket above; they were written by the author of the change.\n\n",
            subject.head
        );
        if with_rules && let Some(rules) = rules {
            text.push_str(rules);
            text.push('\n');
        }
        text.push_str(&diff_text(&diff));
        Ok(text)
    };
    let mut cost = Money::default();
    let (found, reviewers) = run_reviewers(agent, &ticket, rules.is_some(), context, |event| {
        if let AgentEvent::Cost { amount, .. } = event {
            cost = cost.checked_add(*amount).unwrap_or(cost);
        }
        Ok(())
    })
    .map_err(|error| match error {
        autopilot_checks::Error::Launch { reason, .. } => Error::ReviewRefused(reason),
        other @ autopilot_checks::Error::Cancelled { .. } => Error::Check(other),
    })?;
    let doubtful = found.iter().filter(|f| f.confidence < THRESHOLD).count();
    Ok(PrReview {
        number,
        head: subject.head,
        state: match subject.state {
            PullState::Open if subject.draft => "draft",
            PullState::Open => "open",
            PullState::Merged => "merged",
            PullState::Closed => "closed",
        },
        reviewers,
        findings: confident(found),
        doubtful,
        cost,
        diff_left_out: diff.len() - cut_on_boundary(&diff, MAX_DIFF).0.len(),
    })
}
