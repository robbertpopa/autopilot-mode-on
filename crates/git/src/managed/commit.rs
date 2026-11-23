use std::ffi::OsString;

use autopilot_core::TicketId;

use super::{CHECKOUT_LIMITS, WorktreeManager, read_ownership, ticket_worktree};
use crate::{DiffStats, Error, ObjectId};

/// The identity the host commits under. Fixed, so a commit never depends on
/// whatever name the machine's Git config holds.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "autopilot"),
    ("GIT_AUTHOR_EMAIL", "autopilot@localhost"),
    ("GIT_COMMITTER_NAME", "autopilot"),
    ("GIT_COMMITTER_EMAIL", "autopilot@localhost"),
];

impl WorktreeManager {
    /// Commit everything in the ticket's checkout, including new and deleted
    /// files, as the host. Returns `None` when there was nothing to commit.
    ///
    /// The agent never commits for itself, and if it did, hooks and signing
    /// are off here either way. Committing after each turn is what lets a diff
    /// against the base see untracked files and deletions.
    ///
    /// # Errors
    /// Returns an error for an unverified checkout or a Git failure.
    pub fn commit_all(&self, ticket: &TicketId, message: &str) -> Result<Option<ObjectId>, Error> {
        let path = self.verified_checkout(ticket)?;
        let _lock = self.lock()?;
        self.repository
            .git(&path, "add", ["-A", "--"], CHECKOUT_LIMITS)?;
        let staged = self.repository.git(
            &path,
            "status",
            ["--porcelain=v1", "-z", "--untracked-files=no"],
            CHECKOUT_LIMITS,
        )?;
        if staged.is_empty() {
            return Ok(None);
        }
        let environment: Vec<(OsString, OsString)> = IDENTITY
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect();
        self.repository.git_full(
            &path,
            "commit",
            ["--quiet", "--no-verify", "--no-gpg-sign", "-m", message],
            CHECKOUT_LIMITS,
            None,
            &environment,
        )?;
        let head =
            self.repository
                .git(&path, "rev-parse", ["--verify", "HEAD"], CHECKOUT_LIMITS)?;
        Ok(Some(ObjectId::parse(head.trim_ascii_end())?))
    }

    /// Everything the ticket's branch changed since its base, as of its last commit.
    ///
    /// # Errors
    /// Returns an error for an unverified checkout or a Git failure.
    pub fn committed_changes(&self, ticket: &TicketId) -> Result<DiffStats, Error> {
        let path = self.verified_checkout(ticket)?;
        let (_, key) = ticket_worktree(ticket)?;
        let owner = read_ownership(&self.root.join(format!("{key}.owner.json")))?
            .ok_or_else(|| Error::WorktreeConflict(path.clone()))?;
        let base = ObjectId::parse(owner.base.as_bytes())?;
        let head =
            self.repository
                .git(&path, "rev-parse", ["--verify", "HEAD"], CHECKOUT_LIMITS)?;
        let head = ObjectId::parse(head.trim_ascii_end())?;
        self.repository.diff_stats(&base, &head)
    }

    /// The unified diff of everything committed in a ticket's checkout since its
    /// base.
    ///
    /// # Errors
    /// Returns an error for an unverified checkout, a Git failure or a diff
    /// larger than the bounded read.
    pub fn committed_patch(&self, ticket: &TicketId) -> Result<String, Error> {
        let path = self.verified_checkout(ticket)?;
        let (_, key) = ticket_worktree(ticket)?;
        let owner = read_ownership(&self.root.join(format!("{key}.owner.json")))?
            .ok_or_else(|| Error::WorktreeConflict(path.clone()))?;
        let base = ObjectId::parse(owner.base.as_bytes())?;
        let head =
            self.repository
                .git(&path, "rev-parse", ["--verify", "HEAD"], CHECKOUT_LIMITS)?;
        let head = ObjectId::parse(head.trim_ascii_end())?;
        self.repository.patch(&base, &head)
    }

    /// The checkout path, after confirming this manager owns it and it is ready.
    fn verified_checkout(&self, ticket: &TicketId) -> Result<std::path::PathBuf, Error> {
        let (branch, key) = ticket_worktree(ticket)?;
        let path = self.root.join(&key);
        let marker = self.root.join(format!("{key}.owner.json"));
        let owner =
            read_ownership(&marker)?.ok_or_else(|| Error::WorktreeConflict(marker.clone()))?;
        if owner.repository != self.common_directory.as_os_str().as_encoded_bytes()
            || owner.branch != branch
        {
            return Err(Error::WorktreeConflict(marker));
        }
        if !owner.ready || owner.removing {
            return Err(Error::RecoveryRequired(path));
        }
        let reference = format!("refs/heads/{branch}");
        self.exact_checkout(&path, &reference)?
            .ok_or_else(|| Error::WorktreeConflict(path.clone()))?;
        Ok(path)
    }
}
