#![cfg(windows)]

use std::{
    env, fs, io,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, UNIX_EPOCH},
};

use autopilot_core::TicketId;
use autopilot_git::{Error, Removal, Repository, WorktreeManager};

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
