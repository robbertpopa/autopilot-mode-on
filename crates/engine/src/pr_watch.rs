//! One pass over a published pull request: read it, name the next action.
//! It changes nothing; running the action is a separate step.

use std::path::Path;

use autopilot_checks::resolve_executable;
use autopilot_core::{Step, TicketId};
use autopilot_git::{ObjectId, Repository};
use autopilot_pipeline::{StepState, fold};
use autopilot_store::{SqliteStore, Store};
use autopilot_tracker::{Ci, Decision, Gh, Mergeable, PullState};

use crate::{
    ChecksState, DATABASE, Error, MergeState, PrAction, PrState, PrSummary, ReviewDecision,
    next_action, publish::last_offer,
};

#[derive(Debug, Eq, PartialEq)]
pub struct PrWatch {
    pub number: u64,
    pub url: String,
    pub summary: PrSummary,
    /// Comments from trusted reviewers that the action would be built from.
    pub feedback: usize,
    pub action: Option<PrAction>,
}

/// # Errors
/// Returns an error if the ticket has no published pull request, or `gh` or
/// Git fails.
pub fn watch_pull_request(
    directory: &Path,
    ticket: &TicketId,
    repository: &Path,
    guardian: &Path,
    gh: &Gh,
) -> Result<PrWatch, Error> {
    let store = SqliteStore::open_read_only(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let folded = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    if folded.step(Step::PullRequest) != &StepState::Passed {
        return Err(Error::NotReady("no pull request has been published".into()));
    }
    let offer = last_offer(&history).ok_or_else(|| Error::History("no publish offer".into()))?;
    let branch = &offer.approval.branch;
    let pull = gh
        .pull_request_for(branch)?
        .ok_or_else(|| Error::NotReady(format!("no pull request found for {branch}")))?;
    let remote = gh.pr_state(pull.number)?;

    let path = std::env::var_os("PATH").unwrap_or_default();
    let git = resolve_executable("git", &path)
        .ok_or_else(|| Error::NotReady("git was not found".into()))?;
    let local =
        Repository::new(git, guardian.into(), repository.into())?.resolve_local_branch(branch)?;

    let summary = summarize(&remote, &local);
    Ok(PrWatch {
        number: pull.number,
        url: pull.url,
        feedback: remote.feedback.len(),
        action: next_action(&summary),
        summary,
    })
}

/// The facts the rules read, from what GitHub says and the local branch's head.
pub(crate) fn summarize(remote: &autopilot_tracker::PrState, local: &ObjectId) -> PrSummary {
    PrSummary {
        state: match remote.state {
            PullState::Open => PrState::Open,
            PullState::Merged => PrState::Merged,
            PullState::Closed => PrState::Closed,
        },
        merge_state: match remote.mergeable {
            Mergeable::Clean => MergeState::Clean,
            Mergeable::Conflicting => MergeState::Conflicting,
            Mergeable::Unknown => MergeState::Unknown,
        },
        unresolved_threads: remote.unresolved_threads,
        review_decision: match remote.decision {
            Decision::None => ReviewDecision::None,
            Decision::Approved => ReviewDecision::Approved,
            Decision::ChangesRequested => ReviewDecision::ChangesRequested,
        },
        checks: match remote.ci {
            Ci::NoChecks => ChecksState::None,
            Ci::Pending => ChecksState::Running,
            Ci::Passed => ChecksState::Passing,
            Ci::Failed(_) => ChecksState::Failing,
        },
        // The worktree's own changes are not read yet: only a branch that
        // differs from the pull request's head counts as unsynced.
        local_dirty: false,
        local_ahead: !local.as_str().eq_ignore_ascii_case(&remote.head),
    }
}
