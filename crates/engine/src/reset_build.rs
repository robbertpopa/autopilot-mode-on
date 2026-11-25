//! Put a Build back to the start: its copy of the project and its branch are
//! removed, then the step goes back to idle so the next Build makes a fresh
//! copy from the base branch.

use std::{env, path::Path, path::PathBuf};

use autopilot_core::{Step, TicketId};
use autopilot_git::{Removal, Repository, WorktreeManager};
use autopilot_pipeline::{StepState, fold};
use autopilot_store::{SqliteStore, Store};

use crate::{DATABASE, Error, reset_step};

/// Where the ticket's copy of the project is made from and kept.
pub struct ResetSetup<'a> {
    pub repository: &'a Path,
    pub worktrees: &'a Path,
    /// A program name is looked up on `PATH`; the Git crate wants an absolute one.
    pub git: PathBuf,
    pub guardian: PathBuf,
}

/// Remove the ticket's worktree and branch, then reset Build and every step
/// after it. The files and commits in the copy are deleted and cannot come
/// back, so the caller asks the owner first. Nothing is removed when the step
/// cannot be reset, and the step is reset only after the removal worked, so
/// the copy and the step never disagree.
///
/// # Errors
/// Returns an error if a step is active, Build has not run, a pull request was
/// already asked for, Git refuses the removal (a locked or unrecognised copy),
/// or the log cannot be written.
pub fn reset_build(
    directory: &Path,
    ticket: &TicketId,
    setup: &ResetSetup<'_>,
) -> Result<Removal, Error> {
    refuse_unless_resettable(directory, ticket)?;
    let git = autopilot_checks::resolve_executable(
        &setup.git.to_string_lossy(),
        &env::var_os("PATH").unwrap_or_default(),
    )
    .ok_or_else(|| Error::NotReady(format!("git was not found: {}", setup.git.display())))?;
    let repository = Repository::new(git, setup.guardian.clone(), setup.repository.to_path_buf())?;
    let removal =
        WorktreeManager::new(repository, setup.worktrees)?.remove_discarding_work(ticket)?;
    reset_step(directory, ticket, Step::BuildCheck, true)?;
    Ok(removal)
}

fn refuse_unless_resettable(directory: &Path, ticket: &TicketId) -> Result<(), Error> {
    let store = SqliteStore::open_read_only(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::BuildCheck) == &StepState::Idle {
        return Err(Error::NotReady("the build has not run".into()));
    }
    if state.active().is_some() {
        return Err(Error::NotReady("a step is still running".into()));
    }
    // Resetting would drop the approved commit, and the pull request step would
    // then run again over a branch that was already pushed.
    if !matches!(
        state.step(Step::PullRequest),
        StepState::Idle | StepState::Skipped(_)
    ) {
        return Err(Error::NotReady(
            "a pull request was already asked for from this build".into(),
        ));
    }
    Ok(())
}
