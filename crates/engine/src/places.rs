//! What Git already knows about a project, so the window can start a Build and
//! a pull request without asking the owner for it: the branch the work is based
//! on and the GitHub repository the `origin` remote points at.

use std::path::{Path, PathBuf};

use autopilot_git::Repository;
use autopilot_tracker::{Gh, Repo};

use crate::Error;

/// The branch a ticket's work starts from and the repository it is published
/// to, when the project has a GitHub remote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Places {
    /// The branch checked out in the project now. A ticket branches from it.
    pub base: String,
    /// The GitHub repository the remote points at, if it points at one.
    pub github: Option<Repo>,
}

/// Read the project's base branch and its GitHub repository.
///
/// # Errors
/// Returns an error if the folder is not a Git repository, or its HEAD is not
/// on a branch (a detached HEAD has nothing to branch from by name).
pub fn places(repository: &Path, guardian: &Path, remote: &str) -> Result<Places, Error> {
    let git =
        autopilot_checks::resolve_executable("git", &std::env::var_os("PATH").unwrap_or_default())
            .ok_or_else(|| Error::NotReady("git was not found on PATH".into()))?;
    let repo = Repository::new(git, guardian.to_path_buf(), repository.to_path_buf())?;
    let base = repo.current_branch()?;
    // A project without that remote simply has nowhere to publish yet.
    let github = repo
        .remote_url(remote)
        .ok()
        .and_then(|url| Repo::from_remote(&url));
    Ok(Places { base, github })
}

/// Where a ticket's copy of the project is, once a Build has made it. The
/// window and the command line keep every ticket's checkout under
/// `<data>/worktrees`.
#[must_use]
pub fn worktree_of(directory: &Path, ticket: &autopilot_core::TicketId) -> Option<PathBuf> {
    let folder = autopilot_git::WorktreeManager::checkout_folder(ticket).ok()?;
    let path = directory.join("worktrees").join(folder);
    path.is_dir().then_some(path)
}

/// A `gh` handle for publishing. `gh` is found on `PATH`; the token it uses is
/// its own sign-in, which the tracker hands to nothing else.
///
/// # Errors
/// Returns an error if `gh` is not installed or cannot be set up.
pub fn gh_for(guardian: PathBuf, repo: Repo) -> Result<Gh, Error> {
    let program =
        autopilot_checks::resolve_executable("gh", &std::env::var_os("PATH").unwrap_or_default())
            .ok_or_else(|| {
            Error::NotReady("gh (the GitHub command line) was not found on PATH".into())
        })?;
    Gh::new(program, guardian, repo, std::env::vars_os())
        .map_err(|error| Error::NotReady(error.to_string()))
}
