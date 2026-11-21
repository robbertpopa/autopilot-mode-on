use std::{
    env, fmt,
    fs::{self, File, Metadata},
    io::{self, Read},
    path::{Path, PathBuf},
};

use autopilot_core::TicketId;
use sha2::{Digest, Sha256};

use super::{WorktreeManager, read_ownership, repository_path, ticket_worktree};
use crate::{Error, GIT_LIMITS, ObjectId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TreeFingerprint([u8; 32]);

#[derive(PartialEq)]
struct GitState {
    head: ObjectId,
    index: Vec<u8>,
    staged: Vec<u8>,
}

impl fmt::Display for TreeFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl WorktreeManager {
    /// Fingerprint a completed checkout after its workers have stopped.
    ///
    /// Includes the current commit, logical index, file contents and paths,
    /// permissions, untracked and ignored entries, and in-tree symlink destinations.
    ///
    /// # Errors
    /// Returns an error for an unverified checkout, changes detected during
    /// the read, an unsupported file type, or a Git or filesystem failure.
    pub fn fingerprint(&self, ticket: &TicketId) -> Result<TreeFingerprint, Error> {
        let (branch, key) = ticket_worktree(ticket)?;
        let path = self.root.join(key);
        let marker = path.with_extension("owner.json");
        let _lock = self.lock()?;
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
        let checkout = self
            .exact_checkout(&path, &reference)?
            .ok_or_else(|| Error::WorktreeConflict(path.clone()))?;
        let git = self.git_state(&path)?;
        if checkout.head.as_ref() != Some(&git.head) {
            return Err(Error::ChangedDuringFingerprint(path));
        }

        let mut digest = Sha256::new();
        hash_field(&mut digest, b"autopilot-worktree-v1");
        hash_field(&mut digest, env::consts::OS.as_bytes());
        hash_field(&mut digest, git.head.as_str().as_bytes());
        hash_field(&mut digest, &git.index);
        hash_field(&mut digest, &git.staged);
        hash_tree(&mut digest, &path)?;

        self.verify_checkout_identity(&path, &path, &reference)?;
        if self.git_state(&path)? != git {
            return Err(Error::ChangedDuringFingerprint(path));
        }
        Ok(TreeFingerprint(digest.finalize().into()))
    }

    fn git_state(&self, path: &Path) -> Result<GitState, Error> {
        let administrative = repository_path(&self.repository, path, "--absolute-git-dir")?;
        match administrative.join("index.lock").symlink_metadata() {
            Ok(_) => return Err(Error::ChangedDuringFingerprint(path.into())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let head = self
            .repository
            .git(path, "rev-parse", ["--verify", "HEAD"], GIT_LIMITS)?;
        let head = ObjectId::parse(head.trim_ascii_end())?;
        let index = self
            .repository
            .git(path, "ls-files", ["--stage", "-v", "-z"], GIT_LIMITS)?;
        // ls-files exposes the empty blob for both a staged empty file and intent-to-add.
        let staged = self.repository.git(
            path,
            "diff",
            [
                "--cached",
                "--raw",
                "-z",
                "--no-abbrev",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--no-relative",
                "--ignore-submodules=none",
                "--ita-invisible-in-index",
                head.as_str(),
                "--",
            ],
            GIT_LIMITS,
        )?;
        Ok(GitState {
            head,
            index,
            staged,
        })
    }
}

fn hash_tree(digest: &mut Sha256, root: &Path) -> Result<(), Error> {
    let mut pending = vec![root.to_path_buf()];
    let mut directories = Vec::new();
    while let Some(path) = pending.pop() {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| Error::ChangedDuringFingerprint(path.clone()))?;
        let metadata = path.symlink_metadata()?;
        let file_type = metadata.file_type();
        hash_field(digest, relative.as_os_str().as_encoded_bytes());
        digest.update(permission_bits(&metadata).to_be_bytes());

        if file_type.is_symlink() {
            hash_link(digest, root, &path)?;
        } else {
            if unsupported_attributes(&metadata) || path.canonicalize()? != path {
                return Err(Error::UnsupportedFingerprintEntry(path));
            }
            if file_type.is_dir() {
                digest.update(b"D");
                let mut children = child_paths(&path, relative.as_os_str().is_empty())?;
                children.sort_unstable();
                pending.extend(children.into_iter().rev());
                directories.push((path, metadata));
            } else if file_type.is_file() {
                digest.update(b"F");
                hash_file(digest, &path, &metadata)?;
            } else {
                return Err(Error::UnsupportedFingerprintEntry(path));
            }
        }
    }
    for (path, before) in directories {
        verify_stamp(&path, &before, &path.symlink_metadata()?)?;
    }
    Ok(())
}

fn hash_link(digest: &mut Sha256, root: &Path, path: &Path) -> Result<(), Error> {
    let target = fs::read_link(path)?;
    let resolved = path.canonicalize()?;
    let relative = resolved
        .strip_prefix(root)
        .map_err(|_| Error::UnsupportedFingerprintEntry(path.into()))?;
    if relative.components().next().is_some_and(|component| {
        component
            .as_os_str()
            .as_encoded_bytes()
            .eq_ignore_ascii_case(b".git")
    }) {
        return Err(Error::UnsupportedFingerprintEntry(path.into()));
    }
    digest.update(b"L");
    hash_field(digest, target.as_os_str().as_encoded_bytes());
    hash_field(digest, relative.as_os_str().as_encoded_bytes());
    if fs::read_link(path)? != target || path.canonicalize()? != resolved {
        return Err(Error::ChangedDuringFingerprint(path.into()));
    }
    Ok(())
}

fn child_paths(path: &Path, at_root: bool) -> io::Result<Vec<PathBuf>> {
    let mut children = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let administrative = if cfg!(windows) {
            name.as_encoded_bytes().eq_ignore_ascii_case(b".git")
        } else {
            name == ".git"
        };
        if !(at_root && administrative) {
            children.push(entry.path());
        }
    }
    Ok(children)
}

fn hash_file(digest: &mut Sha256, path: &Path, metadata: &Metadata) -> Result<(), Error> {
    let mut file = open_regular(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || unsupported_attributes(&opened) {
        return Err(Error::UnsupportedFingerprintEntry(path.into()));
    }
    verify_stamp(path, metadata, &opened)?;
    let expected = metadata.len();
    digest.update(expected.to_be_bytes());
    let mut stream = (&mut file).take(expected.saturating_add(1));
    let mut total = 0;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = match stream.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            break;
        }
        total += read as u64;
        digest.update(&buffer[..read]);
    }
    if total != expected {
        return Err(Error::ChangedDuringFingerprint(path.into()));
    }
    verify_stamp(path, &opened, &file.metadata()?)?;
    verify_stamp(path, &opened, &path.symlink_metadata()?)
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn verify_stamp(path: &Path, before: &Metadata, after: &Metadata) -> Result<(), Error> {
    let modified = before.modified()?;
    if before.file_type() != after.file_type()
        || before.len() != after.len()
        || permission_bits(before) != permission_bits(after)
        || modified != after.modified()?
        || !same_file(before, after)
    {
        return Err(Error::ChangedDuringFingerprint(path.into()));
    }
    Ok(())
}

#[cfg(unix)]
fn open_regular(path: &Path) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags, open};

    open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(Into::into)
}

#[cfg(windows)]
fn open_regular(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

    fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(unix)]
fn permission_bits(metadata: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode()
}

#[cfg(windows)]
fn permission_bits(metadata: &Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
fn same_file(before: &Metadata, after: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
}

#[cfg(windows)]
fn same_file(before: &Metadata, after: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    before.creation_time() == after.creation_time()
        && before.file_attributes() == after.file_attributes()
}

#[cfg(unix)]
fn unsupported_attributes(_: &Metadata) -> bool {
    false
}

#[cfg(windows)]
fn unsupported_attributes(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use sha2::{Digest, Sha256};

    use super::{TreeFingerprint, hash_tree};

    fn snapshot(path: &Path) -> TreeFingerprint {
        let mut digest = Sha256::new();
        hash_tree(&mut digest, path).unwrap();
        TreeFingerprint(digest.finalize().into())
    }

    #[test]
    fn file_order_does_not_change_the_digest_and_untracked_contents_do() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first = first_dir.path().canonicalize().unwrap();
        let second = second_dir.path().canonicalize().unwrap();
        fs::write(first.join("tracked"), b"a").unwrap();
        fs::write(first.join("untracked"), b"b").unwrap();
        fs::write(second.join("untracked"), b"b").unwrap();
        fs::write(second.join("tracked"), b"a").unwrap();
        fs::write(second.join(".git"), b"the administrative pointer differs").unwrap();

        assert_eq!(snapshot(&first), snapshot(&second));
        fs::write(second.join("untracked"), b"c").unwrap();
        assert_ne!(snapshot(&first), snapshot(&second));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_record_in_tree_destinations_and_reject_external_targets() {
        use crate::Error;
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let worktree = directory.path().join("worktree");
        let external = directory.path().join("external");
        fs::create_dir(&worktree).unwrap();
        fs::write(&external, b"outside").unwrap();
        let worktree = worktree.canonicalize().unwrap();
        let link = worktree.join("link");
        fs::write(worktree.join("inside"), b"in tree").unwrap();
        symlink("inside", &link).unwrap();
        let first = snapshot(&worktree);
        fs::write(worktree.join("inside"), b"changed").unwrap();
        assert_ne!(first, snapshot(&worktree));
        fs::remove_file(&link).unwrap();
        symlink("../external", &link).unwrap();
        assert!(matches!(
            hash_tree(&mut Sha256::new(), &worktree),
            Err(Error::UnsupportedFingerprintEntry(_))
        ));
    }
}
