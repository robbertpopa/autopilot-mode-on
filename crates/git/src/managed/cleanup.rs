use std::{ffi::OsStr, fs, io, path::Path, thread, time::Duration};

use autopilot_core::TicketId;

use super::{CHECKOUT_LIMITS, WorktreeManager, read_ownership, ticket_worktree, write_ownership};
use crate::{Error, ObjectId, git_path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Removal {
    AlreadyAbsent,
    Removed,
    BranchPreserved,
}

impl WorktreeManager {
    /// Remove a clean managed checkout. Preserve a branch with commits beyond its original base.
    /// An interrupted removal resumes from the ownership record.
    ///
    /// # Errors
    /// Returns an error when Git identity, ownership or cleanliness cannot be verified,
    /// or when a filesystem operation fails.
    pub fn remove(&self, ticket: &TicketId) -> Result<Removal, Error> {
        let (branch, key) = ticket_worktree(ticket)?;
        let path = self.root.join(&key);
        let marker = path.with_extension("owner.json");
        let retired = path.with_extension("removing");
        let reference = format!("refs/heads/{branch}");
        let _lock = self.lock()?;
        let Some(mut owner) = read_ownership(&marker)? else {
            return Ok(Removal::AlreadyAbsent);
        };
        if owner.repository != self.common_directory.as_os_str().as_encoded_bytes()
            || owner.branch != branch
        {
            return Err(Error::WorktreeConflict(marker));
        }
        let base = ObjectId::parse(owner.base.as_bytes())?;

        let checkout_exists = managed_directory_exists(&path)?;
        let retired_exists = managed_directory_exists(&retired)?;
        if retired_exists && (!owner.removing || checkout_exists) {
            return Err(Error::WorktreeConflict(retired));
        }
        if checkout_exists {
            if self.repository.directory.canonicalize()?.starts_with(&path) {
                return Err(Error::WorktreeConflict(path));
            }
            let checkout = self
                .exact_checkout(&path, &reference)?
                .ok_or_else(|| Error::WorktreeConflict(path.clone()))?;
            self.require_clean_checkout(&checkout)?;
        } else if !owner.ready && !owner.removing {
            // Creation may have ended before Git registered a checkout. Never
            // delete a branch on the strength of a pending reservation alone.
            self.require_free(&path, &branch)?;
            fs::remove_file(marker)?;
            return Ok(Removal::Removed);
        }

        let registered = self.cleanup_registration(&path, &reference)?;
        if retired_exists && !registered {
            return Err(Error::RecoveryRequired(retired));
        }
        if !registered && !checkout_exists && !owner.removing {
            let result = match self.repository.resolve_local_branch(&branch) {
                Ok(_) => Removal::BranchPreserved,
                Err(Error::LocalBranch(_)) => Removal::Removed,
                Err(error) => return Err(error),
            };
            fs::remove_file(marker)?;
            return Ok(result);
        }
        if !owner.removing {
            owner.removing = true;
            write_ownership(&marker, &owner)?;
        }
        if checkout_exists {
            // Git can remove its registration after a partial file deletion.
            // Stage the files atomically so a lock leaves the original intact.
            retry_file_lock(|| fs::rename(&path, &retired))?;
        }
        if managed_directory_exists(&retired)? {
            self.remove_retired_files(&retired, &path, &reference)?;
        }
        if managed_directory_exists(&path)? {
            return Err(Error::RecoveryRequired(path));
        }
        if self.cleanup_registration(&path, &reference)? {
            self.repository.git(
                &self.repository.directory,
                "worktree",
                ["remove".into(), "--".into(), git_path(&path)],
                CHECKOUT_LIMITS,
            )?;
        }
        if self.cleanup_registration(&path, &reference)? {
            return Err(Error::RecoveryRequired(path));
        }
        let result = self.remove_branch(&branch, &reference, &base)?;
        fs::remove_file(marker)?;
        Ok(result)
    }

    fn cleanup_registration(&self, path: &Path, reference: &str) -> Result<bool, Error> {
        let mut found = false;
        for tree in self.repository.worktrees()? {
            let branch_matches = tree.branch.as_deref() == Some(OsStr::new(reference));
            if !branch_matches
                && tree.branch.as_ref().is_some_and(|branch| {
                    branch
                        .as_encoded_bytes()
                        .eq_ignore_ascii_case(reference.as_bytes())
                })
            {
                return Err(Error::RecoveryRequired(path.into()));
            }
            let path_matches = tree.path.file_name() == path.file_name()
                && tree.path.parent().is_some_and(|parent| {
                    parent
                        .canonicalize()
                        .is_ok_and(|parent| Some(parent.as_path()) == path.parent())
                });
            if branch_matches || path_matches {
                if !branch_matches || !path_matches || tree.locked || found {
                    return Err(Error::RecoveryRequired(path.into()));
                }
                found = true;
            }
        }
        Ok(found)
    }

    fn remove_retired_files(
        &self,
        retired: &Path,
        original: &Path,
        reference: &str,
    ) -> Result<(), Error> {
        let gitfile = retired.join(".git");
        match gitfile.symlink_metadata() {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(Error::RecoveryRequired(retired.into())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // An interrupted final directory removal is safe only if it is empty.
                return Ok(retry_file_lock(|| fs::remove_dir(retired))?);
            }
            Err(error) => return Err(error.into()),
        }
        self.verify_checkout_identity(retired, original, reference)?;
        self.verify_checkout_files(retired, true)?;
        for entry in fs::read_dir(retired)? {
            let entry = entry?;
            if entry.file_name() == ".git" {
                continue;
            }
            let child = entry.path();
            if entry.file_type()?.is_dir() {
                if managed_directory_exists(&child)? {
                    retry_file_lock(|| fs::remove_dir_all(&child))?;
                }
            } else {
                retry_file_lock(|| fs::remove_file(&child))?;
            }
        }
        // Keep the link until the data is gone so a retry can still inspect Git status.
        retry_file_lock(|| fs::remove_file(&gitfile))?;
        retry_file_lock(|| fs::remove_dir(retired))?;
        Ok(())
    }

    fn remove_branch(
        &self,
        branch: &str,
        reference: &str,
        base: &ObjectId,
    ) -> Result<Removal, Error> {
        match self.repository.resolve_local_branch(branch) {
            Ok(tip) if tip == *base => {
                self.repository.read_git(
                    "update-ref",
                    &["--no-deref", "-d", reference, base.as_str()],
                )?;
                Ok(Removal::Removed)
            }
            Ok(_) => Ok(Removal::BranchPreserved),
            Err(Error::LocalBranch(_)) => Ok(Removal::Removed),
            Err(error) => Err(error),
        }
    }
}

fn managed_directory_exists(path: &Path) -> Result<bool, Error> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && path.canonicalize()? == path => Ok(true),
        Ok(_) => Err(Error::WorktreeConflict(path.into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn retry_file_lock(mut operation: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    for delay_ms in [25, 50, 100, 200, 400] {
        match operation() {
            Err(error) if cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                thread::sleep(Duration::from_millis(delay_ms));
            }
            result => return result,
        }
    }
    operation()
}
