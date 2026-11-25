//! Fix a published pull request: one Build round loop per action, through the
//! same path as Build, so the same sandbox, budgets, protected paths and review
//! apply. The result is a new commit on the ticket branch. Nothing is pushed
//! here: the commit goes out only through a new publish the owner approves.

use std::{env, fmt::Write as _, path::Path};

use autopilot_agent::Agent;
use autopilot_checks::resolve_executable;
use autopilot_core::{Step, Ticket};
use autopilot_git::{BaseMerge, Repository, WorktreeManager};
use autopilot_pipeline::{StepState, fold};
use autopilot_store::{SqliteStore, Store};
use autopilot_tracker::{Gh, ci_block, feedback_block};

use crate::{
    BuildOutcome, BuildSetup, DATABASE, Error, Isolation, PrAction, build::run_build_with,
    next_action, pr_watch::summarize, publish::last_offer, reset_step,
};

pub struct RepairSetup<'a> {
    pub build: BuildSetup<'a>,
    /// The Git remote the pull request's base branch lives on, such as `origin`.
    pub remote: &'a str,
    pub gh: &'a Gh,
}

/// Fix what the pull request needs, as the one action the rules name now.
///
/// The pull request is read again here and the action must still be the one
/// asked for: acting on an old reading would fix something that has changed.
/// A merge conflict is merged by the host from the remote base branch, and the
/// agent resolves the markers inside the sandbox.
///
/// Afterwards the ticket's Build step has passed with a new commit, and the
/// pull request step is back to idle: publishing that commit is a new request
/// that the owner approves on its own.
///
/// # Errors
/// Returns an error if the ticket has no published pull request, the action is
/// not the one the pull request needs now, there is nothing a trusted person
/// wrote to act on, or the merge, the build or `gh` fails.
pub fn repair_pull_request(
    directory: &Path,
    ticket: &Ticket,
    agent: &impl Agent,
    setup: &RepairSetup<'_>,
    isolation: Isolation,
    action: PrAction,
) -> Result<BuildOutcome, Error> {
    if !matches!(
        action,
        PrAction::ResolveConflicts | PrAction::FixReviewFeedback | PrAction::FixFailingChecks
    ) {
        return Err(Error::NotReady(
            "that action is not a fix to build; publish the commit instead".into(),
        ));
    }
    let build = &setup.build;
    let branch = published_branch(directory, ticket)?;
    let git = resolve_executable(
        &build.git.to_string_lossy(),
        &env::var_os("PATH").unwrap_or_default(),
    )
    .ok_or_else(|| Error::NotReady(format!("git was not found: {}", build.git.display())))?;
    let repository = Repository::new(git.clone(), build.guardian.clone(), build.repository.into())?;
    let pull = setup
        .gh
        .pull_request_for(&branch)?
        .ok_or_else(|| Error::NotReady(format!("no pull request found for {branch}")))?;
    let remote = setup.gh.pr_state(pull.number)?;
    let local = repository.resolve_local_branch(&branch)?;
    let needed = next_action(&summarize(&remote, &local));
    if needed != Some(action) {
        return Err(Error::NotReady(format!(
            "the pull request needs {needed:?} now, not {action:?}"
        )));
    }

    let text = match action {
        PrAction::FixReviewFeedback => {
            if remote.feedback.is_empty() {
                return Err(Error::NotReady(
                    "no comment from a trusted reviewer to act on".into(),
                ));
            }
            feedback_block(&remote.feedback)
        }
        PrAction::FixFailingChecks => {
            let failed = setup.gh.failed_checks(&remote.head)?;
            if failed.is_empty() {
                return Err(Error::NotReady("no failing check to act on".into()));
            }
            ci_block(&failed)
        }
        _ => merge_base(setup, &git, &repository, ticket)?,
    };
    reset_step(directory, &ticket.id, Step::BuildCheck, true)?;
    run_build_with(directory, ticket, agent, build, isolation, Some(text))
}

fn published_branch(directory: &Path, ticket: &Ticket) -> Result<String, Error> {
    let store = SqliteStore::open_read_only(directory.join(DATABASE))?;
    let history = store.events(&ticket.id, 0)?;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::PullRequest) != &StepState::Passed {
        return Err(Error::NotReady("no pull request has been published".into()));
    }
    let offer = last_offer(&history).ok_or_else(|| Error::History("no publish offer".into()))?;
    Ok(offer.approval.branch)
}

/// Merge the remote base branch into the ticket branch and say what the agent
/// is to do about the result.
fn merge_base(
    setup: &RepairSetup<'_>,
    git: &Path,
    repository: &Repository,
    ticket: &Ticket,
) -> Result<String, Error> {
    let build = &setup.build;
    // The remote's branch, not a local one that may be behind: it is what the
    // pull request conflicts against.
    let base = repository.fetch_branch(setup.remote, build.base_branch)?;
    let manager = WorktreeManager::new(
        Repository::new(git.into(), build.guardian.clone(), build.repository.into())?,
        build.worktrees,
    )?;
    manager.create(&ticket.id, build.base_branch)?;
    Ok(match manager.merge_base(&ticket.id, &base)? {
        BaseMerge::Merged => "The base branch was merged without conflicts. Check that the \
                              result still builds and fix anything the merge broke.\n"
            .to_owned(),
        BaseMerge::Conflicts(files) => {
            let mut text = String::from(
                "The base branch was merged into this branch and left conflict markers \
                 (<<<<<<<, =======, >>>>>>>) in these files. Resolve every one of them so \
                 the change still does what the plan says and the base branch's own changes \
                 are kept. Do not commit.\n",
            );
            for file in files {
                let _ = writeln!(text, "- {file:?}");
            }
            text
        }
    })
}
