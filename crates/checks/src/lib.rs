//! Checks are code, not an agent. Each one answers "does this tree pass?" with
//! evidence, and the runner stops at the first one that does not.

mod command;
mod criteria;
mod diff;
mod review;
mod runner;

#[cfg(test)]
mod criteria_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod runner_tests;

pub use command::{CommandCheck, Environment, Toolbox, plain_directory, resolve_executable};
pub use criteria::CriteriaCheck;
pub use diff::DiffCheck;
pub use review::{ReviewCheck, Reviewer};
pub use runner::{CheckRun, Kept, Key, run_all, standard_checks};

use std::{path::Path, sync::atomic::AtomicBool};

use autopilot_core::Plan;

/// A path the agent changed, relative to the repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Change {
    pub path: String,
    /// Lines added and removed. `None` for a binary file, where Git gives no counts.
    pub lines: Option<(u64, u64)>,
}

/// What a check may look at. The caller builds it once per round.
pub struct Context<'a> {
    pub worktree: &'a Path,
    pub plan: &'a Plan,
    pub changes: &'a [Change],
    pub cancel: &'a AtomicBool,
}

/// The outcome of one check run.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    ok: bool,
    pub note: String,
    pub artifacts: Vec<String>,
    /// What the check could not verify. A report with any of these never passes.
    pub unknowns: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn pass(note: impl Into<String>) -> Self {
        Self {
            ok: true,
            note: note.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn fail(note: impl Into<String>) -> Self {
        Self {
            note: note.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_unknowns(mut self, unknowns: Vec<String>) -> Self {
        self.unknowns = unknowns;
        self
    }

    /// Passing is decided here and nowhere else, so an unknown can never slip
    /// through by a check forgetting to look at its own list.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.ok && self.unknowns.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("check {check:?} could not run: {reason}")]
    Launch { check: String, reason: String },
    #[error("check {check:?} was cancelled")]
    Cancelled { check: String },
}

pub trait Check {
    fn id(&self) -> &str;
    /// A check that does not apply is left out of the run, not counted as passed.
    fn applies(&self, context: &Context<'_>) -> bool;
    /// Whether the agent can fix what this check reports. A failure that
    /// cannot be refined goes to the owner instead of another build turn.
    fn can_refine(&self) -> bool;
    /// Everything besides the tree that changes the answer, such as the
    /// resolved command line. It is part of the key for a kept result.
    fn configuration(&self) -> String;
    /// # Errors
    /// Returns an error when the check could not run at all. A check that ran
    /// and found a problem returns a failing [`Report`] instead.
    fn run(&self, context: &Context<'_>) -> Result<Report, Error>;
}

/// The tail of a long text, cut on a character boundary, for a failure note.
pub(crate) fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
