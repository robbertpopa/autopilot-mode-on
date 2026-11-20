#![cfg(windows)]

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use autopilot_core::TicketId;
use autopilot_git::{Error, Repository, WorktreeManager};

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

fn mark_pending(marker: &Path) -> io::Result<()> {
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(marker)?)?;
    record["ready"] = false.into();
    fs::write(marker, serde_json::to_vec(&record)?)
}

#[test]
fn an_interrupted_creation_can_only_reopen_a_clean_checkout() {
    let fixture = tempfile::tempdir().unwrap();
    let repository = fixture.path().join("repository");
    let root = fixture.path().join("managed worktrees");
    fs::create_dir(&repository).unwrap();
    fs::create_dir(&root).unwrap();
    let program = git_program().unwrap();
    git(&program, &repository, &["init", "--initial-branch=base"]).unwrap();
    fs::write(repository.join("item.txt"), b"tracked sample\n").unwrap();
    git(&program, &repository, &["add", "--", "item.txt"]).unwrap();
    git(&program, &repository, &["commit", "-m", "base"]).unwrap();

    let reopen = || {
        let repository =
            Repository::new(program.clone(), PathBuf::new(), repository.clone()).unwrap();
        WorktreeManager::new(repository, &root).unwrap()
    };
    let ticket = TicketId::from("42");
    let checkout = reopen().create(&ticket, "base").unwrap();
    let marker = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    mark_pending(&marker).unwrap();

    let recovered = reopen().create(&ticket, "base").unwrap();
    assert_eq!(recovered.checkout.path, checkout.checkout.path);
    assert_eq!(recovered.base, checkout.base);

    mark_pending(&marker).unwrap();
    let tracked = checkout.checkout.path.join("item.txt");
    fs::write(&tracked, b"someone edited the content\n").unwrap();
    assert!(matches!(
        reopen().create(&ticket, "base"),
        Err(Error::RecoveryRequired(_))
    ));
    assert_eq!(fs::read(&tracked).unwrap(), b"someone edited the content\n");

    let native_checkout = root.join(checkout.checkout.path.file_name().unwrap());
    git(
        &program,
        &native_checkout,
        &["update-index", "--assume-unchanged", "--", "item.txt"],
    )
    .unwrap();
    assert!(matches!(
        reopen().create(&ticket, "base"),
        Err(Error::RecoveryRequired(_))
    ));
}
