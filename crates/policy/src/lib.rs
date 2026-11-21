//! Rules that make a step ask the owner, whatever its checkbox says.
//!
//! The rules are in code because they must hold even when the model or the
//! ticket text argues otherwise. The checkbox can only add a question: the
//! decision is `forced || checkbox`, so nothing here lets it remove one.

use autopilot_core::Money;
use autopilot_profile::{Profile, WhenUnsure};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

/// Lockfiles and manifests of common ecosystems. A change here swaps what the
/// build downloads, so it is never a quiet edit.
const LOCKFILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "bun.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "go.sum",
    "composer.lock",
    "gradle.lockfile",
    "pubspec.lock",
];

const SCRIPT_PATTERNS: &[&str] = &[
    "**/*.sh",
    "**/*.bash",
    "**/*.ps1",
    "**/*.bat",
    "**/*.cmd",
    "**/Makefile",
    "**/justfile",
    "scripts/**",
    "script/**",
    "**/.husky/**",
];

const WORKFLOW_PATTERN: &str = ".github/**";

/// Ticket labels that mean the change is security work.
const SECURITY_LABELS: &[&str] = &["security", "vulnerability", "cve"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Risk {
    Trivial,
    Standard,
    Risky,
}

/// What a diff scan found. The scan itself is another crate's job; policy only
/// decides what a finding means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffSignals {
    pub new_network_calls: bool,
    pub new_dependencies: bool,
    pub changed_signing: bool,
}

#[derive(Clone, Debug)]
pub struct Facts<'a> {
    /// Paths the step will change or has changed, relative to the repository.
    pub touched: &'a [String],
    pub risk: Risk,
    pub labels: &'a [String],
    pub ticket_spent: Money,
    pub day_spent: Money,
    pub signals: DiffSignals,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    ProtectedPath(String),
    WorkflowFile(String),
    Lockfile(String),
    Script(String),
    /// The path leaves the repository or cannot be read as a relative path.
    OutsideRepository(String),
    RiskyTier,
    SecurityLabel(String),
    TicketBudgetExceeded,
    DayBudgetExceeded,
    NewNetworkCalls,
    NewDependencies,
    ChangedSigning,
}

impl Reason {
    /// The phrase shown as "asks anyway: ...".
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::ProtectedPath(path) => format!("protected path {path}"),
            Self::WorkflowFile(path) => format!("workflow file {path}"),
            Self::Lockfile(path) => format!("lockfile {path}"),
            Self::Script(path) => format!("script {path}"),
            Self::OutsideRepository(path) => format!("path outside the repository {path}"),
            Self::RiskyTier => "risky ticket".into(),
            Self::SecurityLabel(label) => format!("security label {label}"),
            Self::TicketBudgetExceeded => "ticket budget exceeded".into(),
            Self::DayBudgetExceeded => "daily budget exceeded".into(),
            Self::NewNetworkCalls => "new network calls".into(),
            Self::NewDependencies => "new dependencies".into(),
            Self::ChangedSigning => "changed signing".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Why the step must ask no matter what. Empty means nothing is forced.
    pub forced: Vec<Reason>,
    /// The step's own "Ask me after this step" box.
    pub checkbox: bool,
}

impl Decision {
    #[must_use]
    pub fn must_ask(&self) -> bool {
        !self.forced.is_empty() || self.checkbox
    }

    /// "asks anyway: ..." text, or `None` when the checkbox alone decides.
    #[must_use]
    pub fn asks_anyway(&self) -> Option<String> {
        if self.forced.is_empty() {
            return None;
        }
        let parts: Vec<String> = self.forced.iter().map(Reason::describe).collect();
        Some(format!("asks anyway: {}", parts.join(", ")))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("protected path pattern {pattern:?} is not valid: {source}")]
pub struct PolicyError {
    pattern: String,
    #[source]
    source: globset::Error,
}

pub struct Policy {
    protected: GlobSet,
    protected_patterns: Vec<String>,
    workflow: GlobSet,
    scripts: GlobSet,
    per_ticket: Option<Money>,
    per_day: Option<Money>,
}

impl Policy {
    /// Compile the project's rules once.
    ///
    /// # Errors
    /// Returns an error naming a protected-path pattern that is not a valid glob.
    pub fn new(profile: &Profile) -> Result<Self, PolicyError> {
        let protected_patterns: Vec<String> = profile.protected_paths.clone();
        Ok(Self {
            protected: set(&protected_patterns)?,
            protected_patterns,
            workflow: set(&[WORKFLOW_PATTERN.to_owned()])?,
            scripts: set(&SCRIPT_PATTERNS
                .iter()
                .map(|p| (*p).to_owned())
                .collect::<Vec<_>>())?,
            per_ticket: profile
                .budgets
                .per_ticket_microusd
                .map(|v| Money::from_microusd(v.get())),
            per_day: profile
                .budgets
                .daily_microusd
                .map(|v| Money::from_microusd(v.get())),
        })
    }

    /// Decide whether a step must ask.
    ///
    /// `ask_after` is the step's checkbox. It never changes `forced`.
    #[must_use]
    pub fn evaluate(&self, ask_after: bool, facts: &Facts<'_>) -> Decision {
        let mut forced = Vec::new();
        for original in facts.touched {
            self.check_path(original, &mut forced);
        }
        if facts.risk == Risk::Risky {
            forced.push(Reason::RiskyTier);
        }
        for label in facts.labels {
            if is_security_label(label) {
                forced.push(Reason::SecurityLabel(label.clone()));
            }
        }
        if self
            .per_ticket
            .is_some_and(|limit| facts.ticket_spent > limit)
        {
            forced.push(Reason::TicketBudgetExceeded);
        }
        if self.per_day.is_some_and(|limit| facts.day_spent > limit) {
            forced.push(Reason::DayBudgetExceeded);
        }
        for (found, reason) in [
            (facts.signals.new_network_calls, Reason::NewNetworkCalls),
            (facts.signals.new_dependencies, Reason::NewDependencies),
            (facts.signals.changed_signing, Reason::ChangedSigning),
        ] {
            if found {
                forced.push(reason);
            }
        }
        Decision {
            forced,
            checkbox: ask_after,
        }
    }

    fn check_path(&self, original: &str, forced: &mut Vec<Reason>) {
        let Some(path) = normalize(original) else {
            forced.push(Reason::OutsideRepository(original.to_owned()));
            return;
        };
        if self.protected.is_match(&path) {
            forced.push(Reason::ProtectedPath(path.clone()));
        }
        if self.workflow.is_match(&path) {
            forced.push(Reason::WorkflowFile(path.clone()));
        }
        let name = path.rsplit('/').next().unwrap_or(&path);
        if LOCKFILES.iter().any(|lock| lock.eq_ignore_ascii_case(name)) {
            forced.push(Reason::Lockfile(path.clone()));
        }
        if self.scripts.is_match(&path) {
            forced.push(Reason::Script(path));
        }
    }

    /// The patterns this policy was built from, for display.
    #[must_use]
    pub fn protected_patterns(&self) -> &[String] {
        &self.protected_patterns
    }
}

/// What to do when a step is unsure, as the project asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsure {
    Ask,
    /// Carry on, and record this text as an assumption so nothing is hidden.
    Decide {
        assumption: String,
    },
}

/// Resolve an unsure moment. This is separate from [`Policy::evaluate`] on
/// purpose: "decide and tell me" never silences a forced confirmation.
#[must_use]
pub fn when_unsure(setting: WhenUnsure, assumption: &str) -> Unsure {
    match setting {
        WhenUnsure::Ask => Unsure::Ask,
        WhenUnsure::DecideAndTell => Unsure::Decide {
            assumption: assumption.to_owned(),
        },
    }
}

fn set(patterns: &[String]) -> Result<GlobSet, PolicyError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        // Case-insensitive because Windows treats two spellings as one file.
        // `*` crosses directories, which only ever matches more paths.
        let glob: Glob = GlobBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .map_err(|source| PolicyError {
                pattern: pattern.clone(),
                source,
            })?;
        builder.add(glob);
    }
    builder.build().map_err(|source| PolicyError {
        pattern: patterns.join(", "),
        source,
    })
}

/// A repository-relative path with forward slashes, or `None` if it escapes.
fn normalize(path: &str) -> Option<String> {
    let unified = path.replace('\\', "/");
    let absolute = unified.starts_with('/')
        || unified.as_bytes().get(1) == Some(&b':')
        || unified.starts_with("//");
    if absolute || unified.contains('\0') {
        return None;
    }
    let mut parts = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn is_security_label(label: &str) -> bool {
    let lower = label.trim().to_ascii_lowercase();
    SECURITY_LABELS.iter().any(|known| {
        lower == *known
            || lower
                .strip_prefix(known)
                .is_some_and(|rest| rest.starts_with([':', '/', '-', ' ']))
    })
}

#[cfg(test)]
mod tests;
