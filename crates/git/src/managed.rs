use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions, TryLockError},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use autopilot_core::TicketId;
use autopilot_process::Limits;
use serde::{Deserialize, Serialize};

use crate::{Error, GIT_LIMITS, ObjectId, Repository, Worktree, git_os_value, git_path};

mod cleanup;
mod commit;
mod fingerprint;
pub use cleanup::Removal;
pub use fingerprint::TreeFingerprint;

const OWNERSHIP_LIMIT: u64 = 256 * 1024;
const CHECKOUT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    ..GIT_LIMITS
};
const STATUS_ARGUMENTS: [&str; 5] = [
    "--porcelain=v1",
    "-z",
    "--untracked-files=all",
    "--ignored=matching",
    "--ignore-submodules=all",
];

pub struct OwnedWorktree {
    pub checkout: Worktree,
    pub base: ObjectId,
}

pub struct WorktreeManager {
    repository: Repository,
    common_directory: PathBuf,
    root: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct Ownership {
    repository: Vec<u8>,
    branch: String,
    base_branch: String,
    base: String,
    ready: bool,
    #[serde(default)]
    removing: bool,
}

impl WorktreeManager {
    /// Use an existing host-managed directory for ownership records and ticket checkouts.
    ///
    /// # Errors
    /// Returns an error if the directory is unusable or overlaps a Git checkout or admin directory.
    pub fn new(repository: Repository, root: &Path) -> Result<Self, Error> {
        if !root.is_absolute() {
            return Err(Error::ManagedRoot);
        }
        let root = root.canonicalize()?;
        let common_directory =
            repository_path(&repository, &repository.directory, "--git-common-dir")?;
        verify_root(&repository, &root, &common_directory)?;
        Ok(Self {
            repository,
            common_directory,
            root,
        })
    }

    /// The folder name under the worktree root that holds a ticket's checkout.
    ///
    /// # Errors
    /// Returns an error if the ticket id cannot name a folder.
    pub fn checkout_folder(ticket: &TicketId) -> Result<String, Error> {
        ticket_worktree(ticket).map(|(_, key)| key)
    }

    /// The branch name the manager uses for a ticket.
    ///
    /// # Errors
    /// Returns an error if the ticket id cannot name a branch.
    pub fn ticket_branch(ticket: &TicketId) -> Result<String, Error> {
        ticket_worktree(ticket).map(|(branch, _)| branch)
    }

    /// Create a ticket checkout, or reopen an exact checkout from a reservation.
    ///
    /// # Errors
    /// Returns an error if the ticket, repository, ownership or worktree cannot be verified.
    /// An incomplete or changed checkout requires recovery.
    pub fn create(&self, ticket: &TicketId, base_branch: &str) -> Result<OwnedWorktree, Error> {
        let (branch, key) = ticket_worktree(ticket)?;
        let reference = format!("refs/heads/{branch}");
        let destination = self.root.join(&key);
        let marker = self.root.join(format!("{key}.owner.json"));
        let _lock = self.lock()?;

        let repository_identity = self.common_directory.as_os_str().as_encoded_bytes();
        let existing = read_ownership(&marker)?;
        let initial = existing.is_none();
        let mut owner = if let Some(mut owner) = existing {
            if owner.repository != repository_identity
                || owner.branch != branch
                || owner.base_branch != base_branch
            {
                return Err(Error::WorktreeConflict(marker));
            }
            if owner.removing {
                return Err(Error::RecoveryRequired(destination));
            }
            if let Some(checkout) = self.exact_checkout(&destination, &reference)? {
                let base = ObjectId::parse(owner.base.as_bytes())?;
                if !owner.ready {
                    self.verify_pending_checkout(&checkout, &base)?;
                    owner.ready = true;
                    write_ownership(&marker, &owner)?;
                }
                return Ok(OwnedWorktree { checkout, base });
            }
            if owner.ready {
                return Err(Error::WorktreeConflict(destination));
            }
            owner
        } else {
            let base = self.repository.resolve_local_branch(base_branch)?;
            Ownership {
                repository: repository_identity.to_vec(),
                branch: branch.clone(),
                base_branch: base_branch.into(),
                base: base.as_str().into(),
                ready: false,
                removing: false,
            }
        };
        match self.require_free(&destination, &branch) {
            Err(Error::WorktreeConflict(path)) if !initial => {
                return Err(Error::RecoveryRequired(path));
            }
            result => result?,
        }
        self.verify_checkout_config(&self.repository.directory)?;
        let base = ObjectId::parse(owner.base.as_bytes())?;
        if initial {
            write_ownership(&marker, &owner)?;
        }
        let checkout = self.add_checkout(&destination, &branch, &reference, base)?;
        owner.ready = true;
        write_ownership(&marker, &owner)?;
        Ok(checkout)
    }

    fn lock(&self) -> Result<File, Error> {
        if self.root.canonicalize()? != self.root {
            return Err(Error::ManagedRoot);
        }
        let lock = root_lock(&self.root)?;
        if repository_path(
            &self.repository,
            &self.repository.directory,
            "--git-common-dir",
        )? != self.common_directory
        {
            return Err(Error::RepositoryChanged);
        }
        verify_root(&self.repository, &self.root, &self.common_directory)?;
        Ok(lock)
    }

    fn exact_checkout(&self, path: &Path, reference: &str) -> Result<Option<Worktree>, Error> {
        let mut checkout = None;
        for mut tree in self.repository.worktrees()? {
            if tree.branch.as_deref() != Some(OsStr::new(reference)) {
                continue;
            }
            if checkout.is_some()
                || tree.prunable
                || tree.path.canonicalize()? != path
                || path.canonicalize()? != path
            {
                return Err(Error::WorktreeConflict(path.into()));
            }
            self.verify_checkout_identity(path, path, reference)?;
            tree.path = path.into();
            checkout = Some(tree);
        }
        Ok(checkout)
    }

    fn verify_checkout_identity(
        &self,
        path: &Path,
        registered_path: &Path,
        reference: &str,
    ) -> Result<(), Error> {
        let administrative = repository_path(&self.repository, path, "--absolute-git-dir")?;
        let branch = self
            .repository
            .git(path, "symbolic-ref", ["--quiet", "HEAD"], GIT_LIMITS)?;
        if repository_path(&self.repository, path, "--show-toplevel")? != path
            || repository_path(&self.repository, path, "--git-common-dir")? != self.common_directory
            || administrative.parent() != Some(self.common_directory.join("worktrees").as_path())
            || branch.trim_ascii_end() != reference.as_bytes()
            || !path.join(".git").symlink_metadata()?.is_file()
            || !registered_checkout(&administrative, registered_path)?
        {
            return Err(Error::WorktreeConflict(path.into()));
        }
        Ok(())
    }

    fn require_free(&self, path: &Path, branch: &str) -> Result<(), Error> {
        match path.symlink_metadata() {
            Ok(_) => return Err(Error::WorktreeConflict(path.into())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let existing = self
            .repository
            .read_git("for-each-ref", &["--format=%(refname)", "--", "refs/heads"])?;
        let target = format!("refs/heads/{branch}").to_ascii_lowercase();
        let child_prefix = format!("{target}/");
        for line in existing.split(|&byte| byte == b'\n') {
            let reference = line.trim_ascii_end().to_ascii_lowercase();
            if reference == b"refs/heads/ticket"
                || reference == target.as_bytes()
                || reference.starts_with(child_prefix.as_bytes())
            {
                return Err(Error::WorktreeConflict(path.into()));
            }
        }
        Ok(())
    }

    fn verify_pending_checkout(&self, checkout: &Worktree, base: &ObjectId) -> Result<(), Error> {
        if checkout.head.as_ref() != Some(base) {
            return Err(Error::RecoveryRequired(checkout.path.clone()));
        }
        self.require_clean_checkout(checkout)
    }

    fn require_clean_checkout(&self, checkout: &Worktree) -> Result<(), Error> {
        let path = &checkout.path;
        if checkout.locked {
            return Err(Error::RecoveryRequired(path.clone()));
        }
        self.verify_checkout_files(path, false)
    }

    fn verify_checkout_files(&self, path: &Path, allow_deleted: bool) -> Result<(), Error> {
        self.verify_checkout_config(path)?;
        let administrative = repository_path(&self.repository, path, "--absolute-git-dir")?;
        match administrative.join("index.lock").symlink_metadata() {
            Ok(_) => return Err(Error::RecoveryRequired(path.into())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let entries = self
            .repository
            .git(path, "ls-files", ["--stage", "-v", "-z"], GIT_LIMITS)?;
        // Skip-worktree, assume-unchanged and gitlink entries can hide files from status.
        if entries.split(|&byte| byte == 0).any(|record| {
            record.starts_with(b"S ")
                || record.first().is_some_and(u8::is_ascii_lowercase)
                || record
                    .get(2..)
                    .is_some_and(|entry| entry.starts_with(b"160000 "))
        }) {
            return Err(Error::RecoveryRequired(path.into()));
        }
        let status = self
            .repository
            .git(path, "status", STATUS_ARGUMENTS, GIT_LIMITS)?;
        require_safe_status(path, &status, allow_deleted)?;

        // A fresh index avoids cached stat data that can hide timestamp-preserving edits.
        let temporary = tempfile::tempdir_in(&self.root)?;
        let index = temporary.path().join("index");
        self.repository
            .git_with_index(path, "read-tree", ["HEAD"], GIT_LIMITS, Some(&index))?;
        let status = self.repository.git_with_index(
            path,
            "status",
            STATUS_ARGUMENTS,
            GIT_LIMITS,
            Some(&index),
        )?;
        require_safe_status(path, &status, allow_deleted)
    }

    fn verify_checkout_config(&self, path: &Path) -> Result<(), Error> {
        // A checkout can invoke a configured filter before an agent sandbox exists.
        match self.repository.git(
            path,
            "config",
            [
                "--no-includes",
                "--name-only",
                "--get-regexp",
                r"^(filter\..*\.(clean|process|smudge)|include(if)?\..*)$",
            ],
            GIT_LIMITS,
        ) {
            Err(Error::Git { status, .. }) if status.code() == Some(1) => Ok(()),
            Ok(_) => Err(Error::ExternalCheckoutFilter),
            Err(error) => Err(error),
        }
    }

    fn add_checkout(
        &self,
        path: &Path,
        branch: &str,
        reference: &str,
        base: ObjectId,
    ) -> Result<OwnedWorktree, Error> {
        self.repository.git(
            &self.repository.directory,
            "worktree",
            [
                OsString::from("add"),
                "--no-track".into(),
                "-b".into(),
                branch.into(),
                "--".into(),
                git_path(path),
                base.as_str().into(),
            ],
            CHECKOUT_LIMITS,
        )?;
        let checkout = self
            .exact_checkout(path, reference)?
            .filter(|tree| tree.head.as_ref() == Some(&base))
            .ok_or_else(|| Error::WorktreeConflict(path.into()))?;
        Ok(OwnedWorktree { checkout, base })
    }
}

fn repository_path(
    repository: &Repository,
    directory: &Path,
    flag: &str,
) -> Result<PathBuf, Error> {
    let output = repository.git(
        directory,
        "rev-parse",
        ["--path-format=absolute", flag],
        GIT_LIMITS,
    )?;
    Ok(git_path_line(&output)?.canonicalize()?)
}

fn git_path_line(bytes: &[u8]) -> Result<PathBuf, Error> {
    let path = bytes.strip_suffix(b"\n").ok_or(Error::Inventory)?;
    #[cfg(windows)]
    let path = path.strip_suffix(b"\r").unwrap_or(path);
    Ok(PathBuf::from(git_os_value(path)?))
}

fn registered_checkout(administrative: &Path, checkout: &Path) -> Result<bool, Error> {
    let backlink = administrative.join("gitdir");
    if !backlink.symlink_metadata()?.is_file() {
        return Ok(false);
    }
    let mut contents = Vec::new();
    File::open(backlink)?
        .take(OWNERSHIP_LIMIT + 1)
        .read_to_end(&mut contents)?;
    if contents.len() as u64 > OWNERSHIP_LIMIT {
        return Ok(false);
    }
    // Git may store this path relative to the administrative directory.
    let registered = administrative.join(git_path_line(&contents)?);
    if registered.file_name() != Some(OsStr::new(".git")) {
        return Ok(false);
    }
    let directory = registered.parent().ok_or(Error::Inventory)?;
    let parent = directory.parent().ok_or(Error::Inventory)?.canonicalize()?;
    let name = directory.file_name().ok_or(Error::Inventory)?;
    Ok(parent.join(name) == checkout)
}

fn require_safe_status(path: &Path, status: &[u8], allow_deleted: bool) -> Result<(), Error> {
    let safe = status
        .split(|&byte| byte == 0)
        .all(|record| record.is_empty() || (allow_deleted && record.starts_with(b" D ")));
    if !safe {
        return Err(Error::RecoveryRequired(path.into()));
    }
    Ok(())
}

fn verify_root(repository: &Repository, root: &Path, common: &Path) -> Result<(), Error> {
    if !root.is_dir() || root.starts_with(common) || root.canonicalize()? != root {
        return Err(Error::ManagedRoot);
    }
    let bare = repository.read_git("rev-parse", &["--is-bare-repository"])?;
    if bare.trim_ascii_end() == b"false"
        && root.starts_with(repository_path(
            repository,
            &repository.directory,
            "--show-toplevel",
        )?)
    {
        return Err(Error::ManagedRoot);
    }
    for tree in repository.worktrees()? {
        match tree.path.canonicalize() {
            Ok(path) if root.starts_with(&path) => return Err(Error::ManagedRoot),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// How long a ticket waits for another ticket's short operation on the same
/// project. Tickets of one project run side by side, so a held lock is the
/// normal case and not a conflict.
const LOCK_WAIT: Duration = Duration::from_secs(60);

fn root_lock(root: &Path) -> Result<File, Error> {
    root_lock_within(root, LOCK_WAIT)
}

fn root_lock_within(root: &Path, wait: Duration) -> Result<File, Error> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".autopilot-worktrees.lock"))?;
    let start = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(lock),
            Err(TryLockError::WouldBlock) if start.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(TryLockError::WouldBlock) => return Err(Error::Busy),
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
    }
}

fn ticket_worktree(ticket: &TicketId) -> Result<(String, String), Error> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let id = ticket.as_str();
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(Error::InvalidTicket);
    }
    let upper = id.to_ascii_uppercase();
    if matches!(upper.as_str(), "AUX" | "CON" | "NUL" | "PRN")
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|suffix| matches!(suffix.as_bytes(), [b'0'..=b'9']))
    {
        return Err(Error::InvalidTicket);
    }
    // A lowercase encoding distinguishes differently cased IDs on Windows.
    let mut key = String::from("ticket-");
    for byte in id.bytes() {
        key.push(char::from(HEX[usize::from(byte >> 4)]));
        key.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
    Ok((format!("ticket/{id}"), key))
}

fn read_ownership(path: &Path) -> Result<Option<Ownership>, Error> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_file() && metadata.len() <= OWNERSHIP_LIMIT => {}
        Ok(_) => return Err(Error::WorktreeConflict(path.into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut contents = Vec::new();
    File::open(path)?
        .take(OWNERSHIP_LIMIT + 1)
        .read_to_end(&mut contents)?;
    if contents.len() as u64 > OWNERSHIP_LIMIT {
        return Err(Error::WorktreeConflict(path.into()));
    }
    Ok(Some(serde_json::from_slice(&contents)?))
}

fn write_ownership(path: &Path, owner: &Ownership) -> Result<(), Error> {
    let directory = path.parent().ok_or(Error::ManagedRoot)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(temporary.as_file_mut(), owner)?;
    temporary.as_file().sync_all()?;
    // Git may start only after the reservation is visible under its final name.
    if owner.ready || owner.removing {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    }
    .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
