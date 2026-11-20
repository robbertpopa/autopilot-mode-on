//! TOML project profiles with step-specific defaults and source diagnostics.

use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
};

use autopilot_core::{Money, Step};
use schemars::{JsonSchema, Schema, schema_for};
use serde::{Deserialize, Serialize};
use toml::Spanned;

#[derive(Debug, thiserror::Error)]
#[error("{}:{line}:{column}: {key}: {message}", path.display())]
pub struct ProfileDiagnostic {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub key: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    #[default]
    Claude,
    Codex,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Auth {
    #[default]
    Subscription,
    Api,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenUnsure {
    #[default]
    Ask,
    DecideAndTell,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckCommand {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BranchRules {
    pub protected: Vec<String>,
}

impl Default for BranchRules {
    fn default() -> Self {
        Self {
            protected: ["main", "master", "develop"].map(String::from).to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectBudgets {
    pub per_ticket_microusd: Option<NonZeroU64>,
    pub daily_microusd: Option<NonZeroU64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Limits {
    pub turns: u32,
    pub rounds: u32,
    pub wall_seconds: u32,
    pub idle_seconds: u32,
    pub budget_microusd: Option<Money>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StepSettings {
    pub agent: Agent,
    pub auth: Auth,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub limits: Limits,
    pub ask_after: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Profile {
    steps: BTreeMap<Step, StepSettings>,
    pub checks: BTreeMap<String, CheckCommand>,
    pub protected_paths: Vec<String>,
    pub branch_rules: BranchRules,
    pub budgets: ProjectBudgets,
    pub when_unsure: WhenUnsure,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ProfileFile {
    steps: BTreeMap<Step, StepOverrides>,
    checks: BTreeMap<String, CheckCommand>,
    protected_paths: Vec<String>,
    branch_rules: BranchRules,
    budgets: ProjectBudgets,
    when_unsure: WhenUnsure,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(extend(
    "if" = {
        "required": ["limits"],
        "properties": {"limits": {
            "required": ["budget_microusd"],
            "properties": {"budget_microusd": {"type": "integer"}}
        }}
    },
    "then" = {"required": ["auth"], "properties": {"auth": {"const": "api"}}}
))]
struct StepOverrides {
    agent: Agent,
    auth: Auth,
    model: Option<String>,
    effort: Option<String>,
    limits: LimitOverrides,
    ask_after: Option<bool>,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct LimitOverrides {
    turns: Option<NonZeroU32>,
    rounds: Option<NonZeroU32>,
    wall_seconds: Option<NonZeroU32>,
    idle_seconds: Option<NonZeroU32>,
    #[schemars(with = "Option<NonZeroU64>")]
    budget_microusd: Option<Spanned<NonZeroU64>>,
}

impl Profile {
    /// Parse a project profile. Missing step fields use their individual defaults.
    ///
    /// # Errors
    /// Returns a diagnostic with the profile path, config key and source location.
    pub fn parse(path: impl AsRef<Path>, source: &str) -> Result<Self, ProfileDiagnostic> {
        let path = path.as_ref();
        let deserializer = toml::de::Deserializer::parse(source)
            .map_err(|error| toml_diagnostic(path, source, "profile".into(), &error))?;
        let mut file: ProfileFile =
            serde_path_to_error::deserialize(deserializer).map_err(|error| {
                let key = error.path().to_string();
                let key = if key == "." { "profile".into() } else { key };
                toml_diagnostic(path, source, key, error.inner())
            })?;

        let mut steps = BTreeMap::new();
        for step in Step::ALL {
            let options = file.steps.remove(&step).unwrap_or_default();
            steps.insert(step, resolve_step(step, options, path, source)?);
        }

        Ok(Self {
            steps,
            checks: file.checks,
            protected_paths: file.protected_paths,
            branch_rules: file.branch_rules,
            budgets: file.budgets,
            when_unsure: file.when_unsure,
        })
    }

    #[must_use]
    pub fn step(&self, step: Step) -> Option<&StepSettings> {
        self.steps.get(&step)
    }

    #[must_use]
    pub fn schema() -> Schema {
        schema_for!(ProfileFile)
    }
}

fn resolve_step(
    step: Step,
    options: StepOverrides,
    path: &Path,
    source: &str,
) -> Result<StepSettings, ProfileDiagnostic> {
    let limits = options.limits;
    if let (Auth::Subscription, Some(budget)) = (options.auth, &limits.budget_microusd) {
        return Err(diagnostic(
            path,
            source,
            budget.span().start,
            format!("steps.{}.limits.budget_microusd", step.as_str()),
            "dollar budgets require auth = \"api\"; use turn and time limits with a subscription"
                .into(),
        ));
    }

    Ok(StepSettings {
        agent: options.agent,
        auth: options.auth,
        model: options.model,
        effort: options.effort,
        limits: Limits {
            turns: limits.turns.map_or(20, NonZeroU32::get),
            rounds: limits.rounds.map_or(3, NonZeroU32::get),
            wall_seconds: limits.wall_seconds.map_or(900, NonZeroU32::get),
            idle_seconds: limits.idle_seconds.map_or(120, NonZeroU32::get),
            budget_microusd: limits
                .budget_microusd
                .map(|budget| Money::from_microusd(budget.into_inner().get())),
        },
        ask_after: options.ask_after.unwrap_or(matches!(
            step,
            Step::Design | Step::Plan | Step::PullRequest
        )),
    })
}

fn toml_diagnostic(
    path: &Path,
    source: &str,
    key: String,
    error: &toml::de::Error,
) -> ProfileDiagnostic {
    diagnostic(
        path,
        source,
        error.span().map_or(0, |span| span.start),
        key,
        error.message().into(),
    )
}

fn diagnostic(
    path: &Path,
    source: &str,
    offset: usize,
    key: String,
    message: String,
) -> ProfileDiagnostic {
    let prefix = &source[..source.floor_char_boundary(offset.min(source.len()))];
    ProfileDiagnostic {
        path: path.to_owned(),
        line: prefix.split('\n').count(),
        column: prefix
            .rsplit('\n')
            .next()
            .map_or(1, |line| line.chars().count() + 1),
        key,
        message,
    }
}
