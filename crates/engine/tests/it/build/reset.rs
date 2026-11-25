//! Resetting a Build removes its copy of the project and puts the step back.

use autopilot_core::Step;
use autopilot_engine::{ResetSetup, reset_build};
use autopilot_git::{Removal, WorktreeManager};
use autopilot_pipeline::{StepState, fold};

use super::*;

fn setup(world: &World) -> ResetSetup<'_> {
    ResetSetup {
        repository: world.repository.path(),
        worktrees: world.worktrees.path(),
        git: PathBuf::from("git"),
        guardian: PathBuf::new(),
    }
}

fn build_state(world: &World) -> StepState {
    let history = watch(world.data.path(), &world.ticket.id, 0).unwrap();
    fold(&history).unwrap().step(Step::BuildCheck).clone()
}

fn checkout(world: &World) -> PathBuf {
    world
        .worktrees
        .path()
        .canonicalize()
        .unwrap()
        .join(WorktreeManager::checkout_folder(&world.ticket.id).unwrap())
}

fn branch_exists(world: &World) -> bool {
    let branch = WorktreeManager::ticket_branch(&world.ticket.id).unwrap();
    Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &branch])
        .current_dir(world.repository.path())
        .output()
        .unwrap()
        .status
        .success()
}

// Invariant: a reset leaves neither a copy without a reset step nor a reset
// step without its copy, and the next Build starts from the base branch, not
// from the old work. It fails if the removal is skipped (the old file is still
// there for the next Build), if the step is not reset, or if the branch stays
// (the next Build could not create its own).
#[test]
fn a_reset_build_loses_its_work_and_the_next_build_starts_clean() {
    let world = world("b-reset", "plain", false);
    assert_eq!(build(&world, ALWAYS_PASS).unwrap(), BuildOutcome::Ready);
    let folder = checkout(&world);
    assert!(folder.is_dir() && branch_exists(&world));
    std::fs::write(folder.join("unsaved.txt"), "not committed").unwrap();

    let removal = reset_build(world.data.path(), &world.ticket.id, &setup(&world)).unwrap();

    assert_eq!(removal, Removal::Removed);
    assert!(!folder.exists() && !branch_exists(&world));
    assert_eq!(build_state(&world), StepState::Idle);
    assert_eq!(build(&world, ALWAYS_PASS).unwrap(), BuildOutcome::Ready);
    assert!(folder.is_dir());
    assert!(!folder.join("unsaved.txt").exists());
}

// Invariant: a Build that never ran has nothing to reset, and the refusal
// touches nothing. It fails if the check moves after the removal, so that a
// refused reset still deleted a copy it was not asked about.
#[test]
fn a_build_that_never_ran_is_refused_and_nothing_is_removed() {
    let world = world("b-idle", "plain", false);
    let error = reset_build(world.data.path(), &world.ticket.id, &setup(&world)).unwrap_err();
    assert!(error.to_string().contains("has not run"), "{error}");
    assert_eq!(build_state(&world), StepState::Idle);
}
