use std::{collections::BTreeMap, fmt::Write, sync::Arc};

use autopilot_policy::Policy;
use sha2::{Digest, Sha256};

use crate::{
    Check, Context, CriteriaCheck, DiffCheck, Error, Report, ReviewCheck, Reviewer, Toolbox,
};

/// What a green result is valid for. A kept result is reused only when every
/// part matches, so any edit to the tree, the plan or the tooling re-runs it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Key {
    /// The commit the worktree was branched from.
    pub base: String,
    /// The complete worktree fingerprint.
    pub tree: String,
    /// The hash of the approved plan.
    pub plan: String,
    configuration: String,
}

impl Key {
    #[must_use]
    pub fn new(base: &str, tree: &str, plan: &str) -> Self {
        Self {
            base: base.into(),
            tree: tree.into(),
            plan: plan.into(),
            configuration: String::new(),
        }
    }

    fn for_check(&self, check: &dyn Check) -> Self {
        // Hashed so a long command line does not bloat the key.
        let digest = Sha256::digest(check.configuration().as_bytes());
        let mut configuration = String::with_capacity(64);
        for byte in digest {
            let _ = write!(configuration, "{byte:02x}");
        }
        Self {
            configuration,
            ..self.clone()
        }
    }
}

/// Green results only. A failing result is never stored, so a failure is
/// always re-run rather than remembered.
#[derive(Debug, Default)]
pub struct Kept(BTreeMap<String, (Key, Report)>);

impl Kept {
    fn get(&self, id: &str, key: &Key) -> Option<&Report> {
        self.0
            .get(id)
            .filter(|(kept, _)| kept == key)
            .map(|(_, report)| report)
    }
}

#[derive(Debug)]
pub struct CheckRun {
    pub id: String,
    pub report: Report,
    pub reused: bool,
    pub can_refine: bool,
}

/// Run the checks that apply, in order, and stop after the first that does
/// not pass. The last entry of the result is that failure, if there was one.
///
/// # Errors
/// Returns an error when a check could not run at all; the checks before it
/// keep their kept results.
pub fn run_all(
    checks: &[Box<dyn Check + '_>],
    context: &Context<'_>,
    key: &Key,
    kept: &mut Kept,
) -> Result<Vec<CheckRun>, Error> {
    let mut runs = Vec::new();
    for check in checks.iter().filter(|check| check.applies(context)) {
        let key = key.for_check(check.as_ref());
        let id = check.id().to_owned();
        let (report, reused) = if let Some(report) = kept.get(&id, &key) {
            (report.clone(), true)
        } else {
            let report = check.run(context)?;
            if report.passed() {
                kept.0.insert(id.clone(), (key, report.clone()));
            } else {
                kept.0.remove(&id);
            }
            (report, false)
        };
        let passed = report.passed();
        runs.push(CheckRun {
            id,
            report,
            reused,
            can_refine: check.can_refine(),
        });
        if !passed {
            break;
        }
    }
    Ok(runs)
}

/// The agreed order, cheapest first: build, fmt, clippy, tests, protected
/// paths and size, plan criteria, then review. Commands the profile does not
/// name are left out. Review is added only when a reviewer is given.
#[must_use]
pub fn standard_checks(
    toolbox: &Arc<Toolbox>,
    policy: Arc<Policy>,
    reviewer: Option<Box<dyn Reviewer>>,
) -> Vec<Box<dyn Check>> {
    let mut checks: Vec<Box<dyn Check>> = Vec::new();
    for name in ["build", "fmt", "clippy", "test"] {
        if let Some(check) = toolbox.check(name) {
            checks.push(Box::new(check));
        }
    }
    checks.push(Box::new(DiffCheck::new(policy)));
    checks.push(Box::new(CriteriaCheck::new(Arc::clone(toolbox))));
    if let Some(reviewer) = reviewer {
        checks.push(Box::new(ReviewCheck::new(reviewer)));
    }
    checks
}
