use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::Path,
    rc::Rc,
    sync::atomic::AtomicBool,
};

use autopilot_core::{Criterion, Plan, Verify};

use crate::{Check, Context, Error, Kept, Key, Report, run_all};

struct Scripted {
    id: &'static str,
    applies: bool,
    config: Rc<RefCell<String>>,
    results: RefCell<VecDeque<Report>>,
    runs: Rc<Cell<u32>>,
}

impl Scripted {
    fn boxed(id: &'static str, results: Vec<Report>) -> (Box<dyn Check>, Rc<Cell<u32>>) {
        let runs = Rc::new(Cell::new(0));
        let check = Self {
            id,
            applies: true,
            config: Rc::new(RefCell::new("v1".into())),
            results: RefCell::new(results.into()),
            runs: Rc::clone(&runs),
        };
        (Box::new(check), runs)
    }
}

impl Check for Scripted {
    fn id(&self) -> &str {
        self.id
    }
    fn applies(&self, _: &Context<'_>) -> bool {
        self.applies
    }
    fn can_refine(&self) -> bool {
        true
    }
    fn configuration(&self) -> String {
        self.config.borrow().clone()
    }
    fn run(&self, _: &Context<'_>) -> Result<Report, Error> {
        self.runs.set(self.runs.get() + 1);
        Ok(self
            .results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Report::pass("again")))
    }
}

fn plan() -> Plan {
    Plan {
        criteria: vec![Criterion {
            id: "a".into(),
            text: "t".into(),
            verify: Verify::Test,
            verification_id: "t".into(),
        }],
        test_plan: vec![],
        files: vec![],
    }
}

/// `(id, passed, reused)` per check that ran.
fn run(checks: &[Box<dyn Check>], key: &Key, kept: &mut Kept) -> Vec<(String, bool, bool)> {
    let plan = plan();
    let cancel = AtomicBool::new(false);
    let context = Context {
        worktree: Path::new("."),
        plan: &plan,
        changes: &[],
        cancel: &cancel,
    };
    run_all(checks, &context, key, kept)
        .unwrap()
        .into_iter()
        .map(|r| (r.id, r.report.passed(), r.reused))
        .collect()
}

fn row(id: &str, passed: bool, reused: bool) -> (String, bool, bool) {
    (id.into(), passed, reused)
}

// Invariant: cheap checks gate expensive ones. A change that ran every check
// would pay for a review of a tree that does not even build.
#[test]
fn the_runner_stops_at_the_first_failure() {
    let (build, _) = Scripted::boxed("build", vec![Report::pass("ok")]);
    let (test, _) = Scripted::boxed("test", vec![Report::fail("1 failed")]);
    let (review, review_runs) = Scripted::boxed("review", vec![]);
    let result = run(
        &[build, test, review],
        &Key::new("b", "t", "p"),
        &mut Kept::default(),
    );
    assert_eq!(
        result,
        vec![row("build", true, false), row("test", false, false)]
    );
    assert_eq!(review_runs.get(), 0);
}

// Invariant: a report with an unknown is not a pass, even if the check says
// ok. A change that trusted the flag alone would let "could not verify" through.
#[test]
fn an_unknown_can_never_pass() {
    let report = Report::pass("looks fine").with_unknowns(vec!["no way to verify c1".into()]);
    assert!(!report.passed());
    let (check, _) = Scripted::boxed("criteria", vec![report]);
    let (after, after_runs) = Scripted::boxed("review", vec![]);
    let result = run(
        &[check, after],
        &Key::new("b", "t", "p"),
        &mut Kept::default(),
    );
    assert_eq!(result, vec![row("criteria", false, false)]);
    assert_eq!(after_runs.get(), 0);
}

// Invariant: a green result is reused only for the same base, tree, plan and
// tooling. Each field is changed in turn, because dropping any one from the
// key would reuse a stale pass and no other test would notice.
#[test]
fn a_kept_result_needs_every_part_of_the_key_to_match() {
    let (check, runs) = Scripted::boxed("test", vec![]);
    let checks = [check];
    let mut kept = Kept::default();
    let key = Key::new("base", "tree", "plan");
    assert_eq!(
        run(&checks, &key, &mut kept),
        vec![row("test", true, false)]
    );
    assert_eq!(run(&checks, &key, &mut kept), vec![row("test", true, true)]);
    assert_eq!(runs.get(), 1, "an unchanged tree reuses the result");
    for changed in [
        Key::new("other", "tree", "plan"),
        Key::new("base", "edited", "plan"),
        Key::new("base", "tree", "other plan"),
    ] {
        let before = runs.get();
        run(&checks, &changed, &mut kept);
        assert_eq!(runs.get(), before + 1, "{changed:?} must re-run");
    }
}

#[test]
fn changed_tooling_invalidates_a_kept_result() {
    let runs = Rc::new(Cell::new(0));
    let config = Rc::new(RefCell::new("cargo test".to_owned()));
    let check: Box<dyn Check> = Box::new(Scripted {
        id: "test",
        applies: true,
        config: Rc::clone(&config),
        results: RefCell::new(VecDeque::new()),
        runs: Rc::clone(&runs),
    });
    let checks = [check];
    let mut kept = Kept::default();
    let key = Key::new("b", "t", "p");
    run(&checks, &key, &mut kept);
    run(&checks, &key, &mut kept);
    assert_eq!(runs.get(), 1);
    *config.borrow_mut() = "cargo test --release".into();
    run(&checks, &key, &mut kept);
    assert_eq!(runs.get(), 2);
}

// Invariant: a failure is never remembered. Without this, a flaky red result
// would be replayed forever for an unchanged tree.
#[test]
fn a_failing_round_is_never_kept() {
    let (check, runs) = Scripted::boxed(
        "test",
        vec![
            Report::pass("ok"),
            Report::fail("broke"),
            Report::pass("ok"),
        ],
    );
    let checks = [check];
    let mut kept = Kept::default();
    let first = Key::new("b", "t", "p");
    run(&checks, &first, &mut kept);
    let second = Key::new("b", "t2", "p");
    assert_eq!(
        run(&checks, &second, &mut kept),
        vec![row("test", false, false)]
    );
    assert_eq!(
        run(&checks, &second, &mut kept),
        vec![row("test", true, false)]
    );
    assert_eq!(runs.get(), 3, "the red result was re-run, not replayed");
    assert_eq!(
        run(&checks, &second, &mut kept),
        vec![row("test", true, true)]
    );
}

#[test]
fn a_check_that_does_not_apply_is_left_out() {
    let runs = Rc::new(Cell::new(0));
    let skipped: Box<dyn Check> = Box::new(Scripted {
        id: "fmt",
        applies: false,
        config: Rc::new(RefCell::new(String::new())),
        results: RefCell::new(VecDeque::new()),
        runs: Rc::clone(&runs),
    });
    let (next, _) = Scripted::boxed("test", vec![]);
    let result = run(
        &[skipped, next],
        &Key::new("b", "t", "p"),
        &mut Kept::default(),
    );
    assert_eq!(result, vec![row("test", true, false)]);
    assert_eq!(runs.get(), 0);
}
