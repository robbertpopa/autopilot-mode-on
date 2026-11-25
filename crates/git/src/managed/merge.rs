use std::ffi::OsString;

use autopilot_core::TicketId;

use super::{CHECKOUT_LIMITS, WorktreeManager, read_ownership, ticket_worktree, write_ownership};
use crate::{Error, ObjectId};

/// How merging a newer base into a ticket's branch ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BaseMerge {
    /// The branch already contains the base, or it merged without conflicts and
    /// the host committed the merge.
    Merged,
    /// Git stopped with conflict markers in these files and left the merge
    /// open. Whoever resolves them leaves the commit to the host: its next
    /// `commit_all` makes the merge commit.
    Conflicts(Vec<String>),
}

impl WorktreeManager {
    /// Merge `base` into the ticket's branch inside its checkout, and record it
    /// as the ticket's base from now on.
    ///
    /// The recorded base is what the change list, the patch the reviewers read
    /// and the publish signals diff against. Left at the old base, they would
    /// judge the upstream commits just merged as the ticket's own work.
    ///
    /// # Errors
    /// Returns an error for an unverified checkout, uncommitted changes in it,
    /// or a Git failure other than conflicts.
    pub fn merge_base(&self, ticket: &TicketId, base: &ObjectId) -> Result<BaseMerge, Error> {
        let path = self.verified_checkout(ticket)?;
        let (_, key) = ticket_worktree(ticket)?;
        let marker = self.root.join(format!("{key}.owner.json"));
        let _lock = self.lock()?;
        let mut owner =
            read_ownership(&marker)?.ok_or_else(|| Error::WorktreeConflict(path.clone()))?;

        // A merge over uncommitted work would mix it into the merge commit.
        let dirty = self.repository.git(
            &path,
            "status",
            ["--porcelain=v1", "-z", "--untracked-files=no"],
            CHECKOUT_LIMITS,
        )?;
        if !dirty.is_empty() {
            return Err(Error::WorktreeConflict(path));
        }

        let merged = self.repository.git(
            &path,
            "merge",
            ["--no-ff", "--no-commit", "--no-edit", base.as_str()],
            CHECKOUT_LIMITS,
        );
        let unmerged = self.repository.git(
            &path,
            "diff",
            ["--name-only", "--diff-filter=U", "-z"],
            CHECKOUT_LIMITS,
        )?;
        let conflicted: Vec<String> = unmerged
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect();
        // A merge that fails without conflicts (a missing commit, a lock) must
        // not be taken for one that worked.
        if conflicted.is_empty()
            && let Err(error) = merged
        {
            return Err(error);
        }
        owner.base = base.as_str().into();
        write_ownership(&marker, &owner)?;
        if !conflicted.is_empty() {
            return Ok(BaseMerge::Conflicts(conflicted));
        }
        self.commit_merge(&path)?;
        Ok(BaseMerge::Merged)
    }

    fn commit_merge(&self, path: &std::path::Path) -> Result<(), Error> {
        let merging = self.repository.git(
            path,
            "rev-parse",
            ["--quiet", "--verify", "MERGE_HEAD"],
            CHECKOUT_LIMITS,
        );
        if merging.is_err() {
            return Ok(());
        }
        let environment: Vec<(OsString, OsString)> = super::commit::IDENTITY
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect();
        self.repository.git_full(
            path,
            "commit",
            [
                "--quiet",
                "--no-verify",
                "--no-gpg-sign",
                "-m",
                "autopilot: merge the base branch",
            ],
            CHECKOUT_LIMITS,
            None,
            &environment,
        )?;
        Ok(())
    }
}
