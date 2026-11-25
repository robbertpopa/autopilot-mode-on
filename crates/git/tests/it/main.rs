#![cfg(windows)]

use std::{
    env, fs, io,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

use autopilot_core::TicketId;
use autopilot_git::{Error, LineStats, Removal, Repository, WorktreeManager};

const ABANDONED: &str = "abandoned";

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    program: PathBuf,
    repository: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> io::Result<Self> {
        let directory = tempfile::tempdir()?;
        let repository = directory.path().join("repository");
        let root = directory.path().join("managed worktrees");
        fs::create_dir(&repository)?;
        fs::create_dir(&root)?;
        let program = git_program()?;
        git(&program, &repository, &["init", "--initial-branch=base"])?;
        fs::write(repository.join("item.txt"), b"tracked sample\n")?;
        git(&program, &repository, &["add", "--", "item.txt"])?;
        git(&program, &repository, &["commit", "-m", "base"])?;
        Ok(Self {
            _directory: directory,
            program,
            repository,
            root,
        })
    }

    fn repository(&self) -> Result<Repository, Error> {
        Repository::new(
            self.program.clone(),
            PathBuf::new(),
            self.repository.clone(),
        )
    }

    fn manager(&self) -> Result<WorktreeManager, Error> {
        WorktreeManager::new(self.repository()?, &self.root)
    }
}

fn git_program() -> io::Result<PathBuf> {
    let path = env::var_os("PATH").ok_or_else(|| io::Error::other("PATH is missing"))?;
    env::split_paths(&path)
        .map(|path| path.join("git.exe"))
        .find(|path| path.is_absolute() && path.is_file())
        .ok_or_else(|| io::Error::other("the Windows integration test requires Git"))
}

fn git(program: &Path, repository: &Path, arguments: &[&str]) -> io::Result<()> {
    let output = Command::new(program)
        .arg("-C")
        .arg(repository)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(arguments)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

fn set_ownership_flag(marker: &Path, flag: &str, value: bool) -> io::Result<()> {
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(marker)?)?;
    record[flag] = value.into();
    fs::write(marker, serde_json::to_vec(&record)?)
}

#[test]
fn an_interrupted_creation_can_only_reopen_a_clean_checkout() {
    let fixture = Fixture::new().unwrap();
    let reopen = || fixture.manager().unwrap();
    let ticket = TicketId::from("42");
    let checkout = reopen().create(&ticket, "base").unwrap();
    let marker = fs::read_dir(&fixture.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    set_ownership_flag(&marker, "ready", false).unwrap();

    let recovered = reopen().create(&ticket, "base").unwrap();
    assert_eq!(recovered.checkout.path, checkout.checkout.path);
    assert_eq!(recovered.base, checkout.base);

    set_ownership_flag(&marker, "ready", false).unwrap();
    let tracked = checkout.checkout.path.join("item.txt");
    fs::write(&tracked, b"someone edited the content\n").unwrap();
    assert!(matches!(
        reopen().create(&ticket, "base"),
        Err(Error::RecoveryRequired(_))
    ));
    assert_eq!(fs::read(&tracked).unwrap(), b"someone edited the content\n");

    let native_checkout = fixture
        .root
        .join(checkout.checkout.path.file_name().unwrap());
    git(
        &fixture.program,
        &native_checkout,
        &["update-index", "--assume-unchanged", "--", "item.txt"],
    )
    .unwrap();
    assert!(matches!(
        reopen().create(&ticket, "base"),
        Err(Error::RecoveryRequired(_))
    ));
}

#[test]
fn three_tickets_of_one_project_create_their_worktrees_side_by_side() {
    let fixture = Fixture::new().unwrap();
    let handles: Vec<_> = ["a1", "b2", "c3"]
        .into_iter()
        .map(|id| {
            let manager = fixture.manager().unwrap();
            thread::spawn(move || {
                manager
                    .create(&TicketId::from(id), "base")
                    .map(|run| run.base)
            })
        })
        .collect();
    // The lock is per project folder; a ticket that arrives while another holds
    // it must wait its turn instead of ending with Error::Busy.
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
}

#[test]
fn removing_a_checkout_keeps_unsaved_changes_and_commits() {
    let fixture = Fixture::new().unwrap();
    let manager = fixture.manager().unwrap();
    let ticket = TicketId::from("42");
    let run = manager.create(&ticket, "base").unwrap();
    let tracked = run.checkout.path.join("item.txt");
    fs::write(&tracked, b"keep these changes\n").unwrap();

    assert!(matches!(
        manager.remove(&ticket),
        Err(Error::RecoveryRequired(_))
    ));
    assert_eq!(fs::read(&tracked).unwrap(), b"keep these changes\n");

    let native = fixture.root.join(run.checkout.path.file_name().unwrap());
    git(&fixture.program, &native, &["add", "--", "item.txt"]).unwrap();
    git(&fixture.program, &native, &["commit", "-m", "preserve it"]).unwrap();
    assert_eq!(manager.remove(&ticket).unwrap(), Removal::BranchPreserved);
    assert!(!run.checkout.path.exists());
    let repo = fixture.repository().unwrap();
    assert_ne!(repo.resolve_local_branch("ticket/42").unwrap(), run.base);
    assert_eq!(manager.remove(&ticket).unwrap(), Removal::AlreadyAbsent);
}

#[test]
fn removing_a_worktree_can_resume_after_the_files_are_moved_aside() {
    let fixture = Fixture::new().unwrap();
    let manager = fixture.manager().unwrap();
    let ticket = TicketId::from("42");
    let run = manager.create(&ticket, "base").unwrap();
    let marker = run.checkout.path.with_extension("owner.json");
    let retired = run.checkout.path.with_extension("removing");
    set_ownership_flag(&marker, "removing", true).unwrap();
    fs::rename(&run.checkout.path, &retired).unwrap();
    drop(manager);

    let unsaved = retired.join("unsaved.txt");
    fs::write(&unsaved, b"keep me").unwrap();
    assert!(matches!(
        fixture.manager().unwrap().remove(&ticket),
        Err(Error::RecoveryRequired(_))
    ));
    assert_eq!(fs::read(&unsaved).unwrap(), b"keep me");
    fs::remove_file(unsaved).unwrap();

    assert_eq!(
        fixture.manager().unwrap().remove(&ticket).unwrap(),
        Removal::Removed
    );
    assert!(!retired.exists());
    assert!(!marker.exists());
    assert!(matches!(
        fixture
            .repository()
            .unwrap()
            .resolve_local_branch("ticket/42"),
        Err(Error::LocalBranch(_))
    ));
}

#[test]
fn a_windows_file_lock_keeps_removal_recoverable() {
    let fixture = Fixture::new().unwrap();
    let manager = fixture.manager().unwrap();
    let ticket = TicketId::from("42");
    let checkout = manager.create(&ticket, "base").unwrap();
    let file = checkout.checkout.path.join("item.txt");
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&file)
        .unwrap();

    assert!(matches!(manager.remove(&ticket), Err(Error::Io(_))));
    assert!(
        fixture
            .repository()
            .unwrap()
            .resolve_local_branch("ticket/42")
            .is_ok()
    );
    assert_eq!(fixture.repository().unwrap().worktrees().unwrap().len(), 2);
    drop(handle);

    assert_eq!(manager.remove(&ticket).unwrap(), Removal::Removed);
    assert!(!checkout.checkout.path.exists());
    assert_eq!(fixture.repository().unwrap().worktrees().unwrap().len(), 1);
}

#[test]
fn removing_a_worktree_detects_edits_with_an_unchanged_size_and_mtime() {
    let fixture = Fixture::new().unwrap();
    let manager = fixture.manager().unwrap();
    let ticket = TicketId::from("42");
    let created = manager.create(&ticket, "base").unwrap();
    let native = fixture
        .root
        .join(created.checkout.path.file_name().unwrap());
    let tracked = native.join("item.txt");
    let times = fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_600_000_000));
    fs::File::options()
        .write(true)
        .open(&tracked)
        .unwrap()
        .set_times(times)
        .unwrap();
    git(&fixture.program, &native, &["update-index", "--refresh"]).unwrap();
    let mut contents = fs::read(&tracked).unwrap();
    contents[0] = b'X';
    fs::write(&tracked, &contents).unwrap();
    fs::File::options()
        .write(true)
        .open(&tracked)
        .unwrap()
        .set_times(times)
        .unwrap();

    assert!(matches!(
        manager.remove(&ticket),
        Err(Error::RecoveryRequired(_))
    ));
    assert_eq!(fs::read(tracked).unwrap(), contents);
}

#[test]
fn removal_does_not_delete_a_case_alias_used_by_another_worktree() {
    let fixture = Fixture::new().unwrap();
    let manager = fixture.manager().unwrap();
    let ticket = TicketId::from("ABC");
    let created = manager.create(&ticket, "base").unwrap();
    let alternative = fixture.repository.parent().unwrap().join("other checkout");
    git(
        &fixture.program,
        &fixture.repository,
        &[
            "worktree",
            "add",
            "--detach",
            alternative.to_str().unwrap(),
            "base",
        ],
    )
    .unwrap();
    git(
        &fixture.program,
        &alternative,
        &["symbolic-ref", "HEAD", "refs/heads/ticket/abc"],
    )
    .unwrap();

    assert!(matches!(
        manager.remove(&ticket),
        Err(Error::RecoveryRequired(_))
    ));
    assert!(created.checkout.path.join("item.txt").is_file());
    assert!(alternative.join("item.txt").is_file());
    assert_eq!(
        fixture
            .repository()
            .unwrap()
            .resolve_local_branch("ticket/ABC")
            .unwrap(),
        created.base
    );
}

#[test]
fn a_commit_summary_keeps_both_paths_of_a_rename_from_a_subdirectory() {
    let fixture = Fixture::new().unwrap();
    let subdirectory = fixture.repository.join("inside");
    fs::create_dir(&subdirectory).unwrap();
    let repository = Repository::new(
        fixture.program.clone(),
        PathBuf::new(),
        subdirectory.clone(),
    )
    .unwrap();
    let base = repository.resolve_local_branch("base").unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["switch", "-c", "edited"],
    )
    .unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["mv", "item.txt", "moved.txt"],
    )
    .unwrap();
    fs::write(subdirectory.join("binary.dat"), [0, 1, 0]).unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["add", "--", "inside/binary.dat"],
    )
    .unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["commit", "-m", "changes"],
    )
    .unwrap();
    let head = repository.resolve_local_branch("edited").unwrap();

    let summary = repository.diff_stats(&base, &head).unwrap();
    assert_eq!(
        summary.lines,
        LineStats {
            added: 1,
            removed: 1
        }
    );
    assert_eq!(summary.files.len(), 3);
    assert!(
        summary
            .files
            .iter()
            .any(|file| file.path == Path::new("item.txt"))
    );
    assert!(
        summary
            .files
            .iter()
            .any(|file| file.path == Path::new("moved.txt"))
    );
    assert!(
        summary
            .files
            .iter()
            .any(|file| file.path == Path::new("inside/binary.dat") && file.lines.is_none())
    );
}

// Invariant: the scan sees the lines the agent added, in new and changed text
// files, and nothing for a binary file.
#[test]
fn added_lines_come_from_the_commit_and_skip_binary_files() {
    let fixture = Fixture::new().unwrap();
    let repository = fixture.repository().unwrap();
    let base = repository.resolve_local_branch("base").unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["switch", "-c", "scan"],
    )
    .unwrap();
    fs::write(
        fixture.repository.join("Cargo.toml"),
        "[dependencies]
serde = \"1\"
",
    )
    .unwrap();
    fs::write(fixture.repository.join("blob.dat"), [0, 1, 0]).unwrap();
    git(&fixture.program, &fixture.repository, &["add", "."]).unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["commit", "-m", "scan"],
    )
    .unwrap();
    let head = repository.resolve_local_branch("scan").unwrap();

    let added = repository.added_lines(&base, &head).unwrap();
    let cargo = added.iter().find(|file| file.path == "Cargo.toml").unwrap();
    assert_eq!(cargo.lines, ["[dependencies]", "serde = \"1\""]);
    assert!(
        added
            .iter()
            .all(|file| file.path != "blob.dat" || file.lines.is_empty())
    );
}

// Invariant: the host pushes one commit to one named branch and never
// forces: a diverged branch is refused by the remote and a name that could
// change the refspec is refused before Git runs.
#[test]
fn a_push_sends_one_commit_and_never_forces() {
    let fixture = Fixture::new().unwrap();
    let repository = fixture.repository().unwrap();
    let remote = tempfile::tempdir().unwrap();
    git(&fixture.program, remote.path(), &["init", "-q", "--bare"]).unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        repository.remote_url("origin").unwrap(),
        remote.path().to_str().unwrap()
    );
    assert!(repository.remote_url("--evil").is_err());

    let base = repository.resolve_local_branch("base").unwrap();
    repository
        .push_commit("origin", &base, "ticket/one")
        .unwrap();
    let pushed = Command::new(&fixture.program)
        .args([
            "-C",
            remote.path().to_str().unwrap(),
            "rev-parse",
            "refs/heads/ticket/one",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        base.as_str()
    );

    git(
        &fixture.program,
        &fixture.repository,
        &["switch", "-c", "other", "base~0"],
    )
    .unwrap();
    fs::write(fixture.repository.join("other.txt"), "x").unwrap();
    git(&fixture.program, &fixture.repository, &["add", "."]).unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["commit", "-m", "other"],
    )
    .unwrap();
    let other = repository.resolve_local_branch("other").unwrap();
    // `other` descends from base, so it fast-forwards; a sibling does not.
    repository
        .push_commit("origin", &other, "ticket/one")
        .unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["switch", "-c", "sibling", "base"],
    )
    .unwrap();
    fs::write(fixture.repository.join("sibling.txt"), "y").unwrap();
    git(&fixture.program, &fixture.repository, &["add", "."]).unwrap();
    git(
        &fixture.program,
        &fixture.repository,
        &["commit", "-m", "sibling"],
    )
    .unwrap();
    let sibling = repository.resolve_local_branch("sibling").unwrap();
    assert!(
        repository
            .push_commit("origin", &sibling, "ticket/one")
            .is_err()
    );

    for bad in ["+ticket/x", "a:b", "-x", "ticket/../main", "/abs"] {
        assert!(
            repository.push_commit("origin", &base, bad).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn a_worktree_fingerprint_changes_for_untracked_files_and_staged_content() {
    let fixture = Fixture::new().unwrap();
    let manager = fixture.manager().unwrap();
    let ticket = TicketId::from("42");
    let created = manager.create(&ticket, "base").unwrap();
    let native = fixture
        .root
        .join(created.checkout.path.file_name().unwrap());
    let initial = manager.fingerprint(&ticket).unwrap();

    let untracked = native.join("untracked.bin");
    fs::write(&untracked, [1, 2, 3]).unwrap();
    assert_ne!(initial, manager.fingerprint(&ticket).unwrap());
    fs::remove_file(untracked).unwrap();
    assert_eq!(initial, manager.fingerprint(&ticket).unwrap());

    let tracked = native.join("item.txt");
    let original = fs::read(&tracked).unwrap();
    fs::write(&tracked, b"stage these different bytes\n").unwrap();
    git(&fixture.program, &native, &["add", "--", "item.txt"]).unwrap();
    fs::write(&tracked, original).unwrap();
    assert_ne!(initial, manager.fingerprint(&ticket).unwrap());

    fs::write(native.join("empty"), b"").unwrap();
    git(&fixture.program, &native, &["add", "-N", "--", "empty"]).unwrap();
    let intent = manager.fingerprint(&ticket).unwrap();
    git(&fixture.program, &native, &["add", "--", "empty"]).unwrap();
    assert_ne!(intent, manager.fingerprint(&ticket).unwrap());
}

#[test]
fn a_fresh_manager_removes_the_checkout_and_branch_of_a_killed_worker() {
    let fixture = Fixture::new().unwrap();
    let ready = fixture.root.join("worker-ready");
    let mut worker = KillOnDrop(
        Command::new(env::current_exe().unwrap())
            .args(["--ignored", "--exact", "synthetic_worktree_owner"])
            .env("AUTOPILOT_GIT_FIXTURE_PROGRAM", &fixture.program)
            .env("AUTOPILOT_GIT_FIXTURE_REPOSITORY", &fixture.repository)
            .env("AUTOPILOT_GIT_FIXTURE_ROOT", &fixture.root)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready.is_file() {
        assert!(
            Instant::now() < deadline && worker.0.try_wait().unwrap().is_none(),
            "the synthetic owner failed to create its checkout"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let repository = fixture.repository().unwrap();
    let checkout = repository
        .worktrees()
        .unwrap()
        .into_iter()
        .find(|worktree| {
            worktree.branch.as_deref() == Some(std::ffi::OsStr::new("refs/heads/ticket/abandoned"))
        })
        .unwrap();
    assert!(checkout.path.join("item.txt").is_file());
    worker.0.kill().unwrap();
    assert!(!worker.0.wait().unwrap().success());

    let ticket = TicketId::from(ABANDONED);
    assert_eq!(
        fixture.manager().unwrap().remove(&ticket).unwrap(),
        Removal::Removed
    );
    assert!(!checkout.path.exists());
    assert!(!checkout.path.with_extension("owner.json").exists());
    assert!(!checkout.path.with_extension("removing").exists());
    assert!(matches!(
        repository.resolve_local_branch("ticket/abandoned"),
        Err(Error::LocalBranch(_))
    ));
    assert_eq!(repository.worktrees().unwrap().len(), 1);
    assert_eq!(
        fs::read(fixture.repository.join("item.txt")).unwrap(),
        b"tracked sample\n"
    );
}

#[test]
#[ignore = "invoked by the killed-worker recovery test"]
fn synthetic_worktree_owner() {
    let Some(root) = env::var_os("AUTOPILOT_GIT_FIXTURE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let repository = Repository::new(
        env::var_os("AUTOPILOT_GIT_FIXTURE_PROGRAM").unwrap().into(),
        PathBuf::new(),
        env::var_os("AUTOPILOT_GIT_FIXTURE_REPOSITORY")
            .unwrap()
            .into(),
    )
    .unwrap();
    let manager = WorktreeManager::new(repository, &root).unwrap();
    manager.create(&TicketId::from(ABANDONED), "base").unwrap();
    fs::write(root.join("worker-ready"), b"").unwrap();
    loop {
        thread::park();
    }
}
