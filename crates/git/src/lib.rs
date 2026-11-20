//! Host-side, bounded access to a local Git repository.

mod conflicts;
mod diff;
mod managed;

pub use conflicts::{PathOverlap, overlapping_paths};
pub use diff::{DiffStats, FileStats, LineStats};
pub use managed::{OwnedWorktree, Removal, TreeFingerprint, WorktreeManager};

use std::{
    env,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::atomic::AtomicBool,
    time::Duration,
};

use autopilot_process::{CommandSpec, Limits, Outcome, Runner};

const GIT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(15),
    bytes_per_stream: 1024 * 1024,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the repository directory must be absolute")]
    RelativeRepository,
    #[error("Git {operation} failed ({status}): {diagnostic}")]
    Git {
        operation: &'static str,
        status: ExitStatus,
        diagnostic: String,
    },
    #[error("Git {0} timed out")]
    TimedOut(&'static str),
    #[error("Git {0} exceeded the output limit")]
    OutputLimit(&'static str),
    #[error("Git returned an incomplete or invalid worktree inventory")]
    Inventory,
    #[error("Git did not return a full object ID")]
    ObjectId,
    #[error("Git returned invalid or incomplete diff statistics")]
    InvalidDiff,
    #[error("planned path is not a valid repository-relative path: {0}")]
    InvalidPlannedPath(PathBuf),
    #[error("the worktree changed during fingerprinting at {0}")]
    ChangedDuringFingerprint(PathBuf),
    #[error("the worktree contains an unsupported entry at {0}")]
    UnsupportedFingerprintEntry(PathBuf),
    #[error("local branch {0:?} was not found")]
    LocalBranch(String),
    #[error("the managed worktree directory must be outside the Git repository and its checkouts")]
    ManagedRoot,
    #[error("the configured Git repository changed")]
    RepositoryChanged,
    #[error("the ticket ID cannot be used as a worktree branch")]
    InvalidTicket,
    #[error("another operation owns this worktree directory")]
    Busy,
    #[error("the worktree or its ownership conflicts with an existing entry at {0}")]
    WorktreeConflict(PathBuf),
    #[error("an unfinished worktree reservation at {0} requires recovery")]
    RecoveryRequired(PathBuf),
    #[error("Git checkout has an external filter or config include")]
    ExternalCheckoutFilter,
    #[error("invalid worktree ownership record: {0}")]
    Ownership(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectId(String);

impl ObjectId {
    fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if !matches!(bytes.len(), 40 | 64) || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(Error::ObjectId);
        }
        String::from_utf8(bytes.to_vec())
            .map(Self)
            .map_err(|_| Error::ObjectId)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    /// The full object ID, or all zeroes for an unborn branch.
    pub head: Option<ObjectId>,
    pub branch: Option<OsString>,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
}

pub struct Repository {
    program: PathBuf,
    directory: PathBuf,
    runner: Runner,
}

impl Repository {
    /// Bind a trusted Git executable and guardian to an absolute local directory.
    ///
    /// # Errors
    /// Returns an error when the directory is relative.
    pub fn new(program: PathBuf, guardian: PathBuf, directory: PathBuf) -> Result<Self, Error> {
        if !directory.is_absolute() {
            return Err(Error::RelativeRepository);
        }
        Ok(Self {
            program,
            directory,
            runner: Runner::new(guardian),
        })
    }

    /// Resolve the exact local branch to its full object ID.
    ///
    /// # Errors
    /// Returns an error if Git fails or the branch or object ID is unavailable.
    pub fn resolve_local_branch(&self, name: &str) -> Result<ObjectId, Error> {
        let reference = format!("refs/heads/{name}");
        let output = self.read_git(
            "for-each-ref",
            &["--format=%(objectname) %(refname)", "--", &reference],
        )?;
        // Loose refs on case-insensitive filesystems can resolve with the wrong spelling.
        for line in output.split(|&byte| byte == b'\n') {
            let mut fields = line.trim_ascii_end().splitn(2, |&byte| byte == b' ');
            let object = fields.next().unwrap_or_default();
            if fields.next() == Some(reference.as_bytes()) {
                return ObjectId::parse(object);
            }
        }
        Err(Error::LocalBranch(name.into()))
    }

    /// Read Git's registered worktrees, including locked and prunable entries.
    ///
    /// # Errors
    /// Returns an error if Git fails or the inventory is incomplete or malformed.
    pub fn worktrees(&self) -> Result<Vec<Worktree>, Error> {
        let output = self.read_git("worktree", &["list", "--porcelain", "-z"])?;
        parse_worktrees(&output)
    }

    fn read_git(&self, operation: &'static str, arguments: &[&str]) -> Result<Vec<u8>, Error> {
        self.git(
            &self.directory,
            operation,
            arguments.iter().copied(),
            GIT_LIMITS,
        )
    }

    fn git(
        &self,
        directory: &Path,
        operation: &'static str,
        arguments: impl IntoIterator<Item = impl Into<OsString>>,
        limits: Limits,
    ) -> Result<Vec<u8>, Error> {
        self.git_with_index(directory, operation, arguments, limits, None)
    }

    fn git_with_index(
        &self,
        directory: &Path,
        operation: &'static str,
        arguments: impl IntoIterator<Item = impl Into<OsString>>,
        limits: Limits,
        index: Option<&Path>,
    ) -> Result<Vec<u8>, Error> {
        let mut args = vec![
            "--no-pager".into(),
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.trustctime=true".into(),
            "-c".into(),
            "core.checkStat=default".into(),
            "-c".into(),
            "submodule.recurse=false".into(),
            "-c".into(),
            "core.longpaths=true".into(),
            "-C".into(),
            git_path(directory),
            operation.into(),
        ];
        args.extend(arguments.into_iter().map(Into::into));
        let mut environment = vec![
            // A missing object must not turn a local Git operation into a fetch.
            ("GIT_ALLOW_PROTOCOL".into(), "".into()),
            ("GIT_NO_LAZY_FETCH".into(), "1".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
            ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ];
        if let Some(index) = index {
            environment.push(("GIT_INDEX_FILE".into(), git_path(index)));
        }
        if cfg!(windows)
            && let Some(root) = env::var_os("SystemRoot")
        {
            environment.push(("SystemRoot".into(), root));
        }
        let output = self.runner.run(
            &CommandSpec {
                program: self.program.clone(),
                args,
                env: environment,
            },
            b"",
            limits,
            &AtomicBool::new(false),
        )?;
        match output.outcome {
            Outcome::Exited(status) if status.success() => Ok(output.stdout),
            Outcome::Exited(status) => Err(Error::Git {
                operation,
                status,
                diagnostic: String::from_utf8_lossy(&output.stderr).trim_end().into(),
            }),
            Outcome::TimedOut => Err(Error::TimedOut(operation)),
            Outcome::OutputLimit => Err(Error::OutputLimit(operation)),
            Outcome::Cancelled => {
                Err(io::Error::new(io::ErrorKind::Interrupted, "Git was cancelled").into())
            }
        }
    }
}

fn parse_worktrees(bytes: &[u8]) -> Result<Vec<Worktree>, Error> {
    let inventory = bytes.strip_suffix(b"\0").ok_or(Error::Inventory)?;
    if !inventory.ends_with(b"\0") {
        return Err(Error::Inventory);
    }
    let mut fields = Vec::new();
    let mut trees = Vec::new();
    for field in inventory.split(|&byte| byte == 0) {
        if field.is_empty() {
            trees.push(parse_worktree(&fields)?);
            fields.clear();
        } else {
            fields.push(field);
        }
    }
    Ok(trees)
}

fn parse_worktree(fields: &[&[u8]]) -> Result<Worktree, Error> {
    let (first, attributes) = fields.split_first().ok_or(Error::Inventory)?;
    let path = first.strip_prefix(b"worktree ").ok_or(Error::Inventory)?;
    let path = PathBuf::from(git_os_value(path)?);
    if !path.is_absolute() {
        return Err(Error::Inventory);
    }
    let mut tree = Worktree {
        path,
        head: None,
        branch: None,
        bare: false,
        locked: false,
        prunable: false,
    };
    let mut detached = false;
    for field in attributes {
        let mut parts = field.splitn(2, |&byte| byte == b' ');
        match parts.next().unwrap_or_default() {
            b"HEAD" if tree.head.is_none() => {
                tree.head = Some(ObjectId::parse(parts.next().ok_or(Error::Inventory)?)?);
            }
            b"branch" if tree.branch.is_none() => {
                tree.branch = Some(git_os_value(parts.next().ok_or(Error::Inventory)?)?);
            }
            b"bare" if !tree.bare && parts.next().is_none() => tree.bare = true,
            b"detached" if !detached && parts.next().is_none() => detached = true,
            b"locked" if !tree.locked => tree.locked = true,
            b"prunable" if !tree.prunable => tree.prunable = true,
            b"HEAD" | b"branch" | b"bare" | b"detached" | b"locked" | b"prunable" | b"worktree" => {
                return Err(Error::Inventory);
            }
            _ => {}
        }
    }
    let attached = tree.branch.is_some();
    let valid = if tree.bare {
        tree.head.is_none() && !attached && !detached
    } else {
        tree.head.is_some() && attached != detached
    };
    if !valid {
        return Err(Error::Inventory);
    }
    Ok(tree)
}

fn git_os_value(bytes: &[u8]) -> Result<OsString, Error> {
    if bytes.is_empty() {
        return Err(Error::Inventory);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(std::ffi::OsStr::from_bytes(bytes).into())
    }
    #[cfg(windows)]
    {
        std::str::from_utf8(bytes)
            .map(OsString::from)
            .map_err(|_| Error::Inventory)
    }
}

fn git_path(path: &Path) -> OsString {
    #[cfg(windows)]
    {
        use std::{
            os::windows::ffi::{OsStrExt, OsStringExt},
            path::{Component, Prefix},
        };
        // Git for Windows interprets a verbatim prefix as an invalid UNC path.
        // It handles long paths itself when given a native drive or share path.
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(_) => {
                    let native = path.as_os_str().encode_wide().skip(4).collect::<Vec<_>>();
                    return OsString::from_wide(&native);
                }
                Prefix::VerbatimUNC(_, _) => {
                    let native = [u16::from(b'\\'); 2]
                        .into_iter()
                        .chain(path.as_os_str().encode_wide().skip(8))
                        .collect::<Vec<_>>();
                    return OsString::from_wide(&native);
                }
                _ => {}
            }
        }
    }
    path.as_os_str().to_os_string()
}

#[cfg(test)]
mod tests;
