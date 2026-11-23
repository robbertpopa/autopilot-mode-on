//! TOML project profiles with step-specific defaults and source diagnostics.

use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
};

use autopilot_core::{Money, Step};

mod pipeline;
pub use pipeline::{
    ASK_ONLY, CustomStep, DEFAULT_PIPELINE, Pipeline, Project, StepMode, builtin_pipelines,
    check_name, leaf, normalize_path,
};
use pipeline::{CustomStepFile, PipelineFile, ProjectFile};
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

/// What the pipeline review does. Off is the default: nothing is computed,
/// written or notified until the owner turns it on.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    #[default]
    Off,
    NotifyOnly,
    /// Notify, and apply tier 1 tuning to custom pipelines.
    NotifyAndTune,
}

/// How the review decides something is worth a look. Whole numbers only:
/// shares are thousandths of the context window, and the cost multiple is a
/// percent (200 means twice the median). Starting defaults, not measured
/// facts; the first real runs should tune them.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewSettings {
    pub mode: ReviewMode,
    /// How many of the latest runs of a step are looked at.
    pub window_runs: u32,
    pub peak_context_permille: u32,
    /// How many runs in the window must cross the line to open a finding.
    pub min_hits: u32,
    pub cost_multiple_percent: u32,
}

impl Default for ReviewSettings {
    fn default() -> Self {
        Self {
            mode: ReviewMode::Off,
            window_runs: 5,
            peak_context_permille: 800,
            min_hits: 3,
            cost_multiple_percent: 200,
        }
    }
}

impl ReviewSettings {
    /// # Errors
    /// Returns a sentence for a value that cannot be a sensible threshold.
    pub fn check(&self) -> Result<(), String> {
        if self.window_runs == 0 || self.window_runs > 50 {
            return Err("window_runs must be between 1 and 50".into());
        }
        if self.min_hits == 0 || self.min_hits > self.window_runs {
            return Err("min_hits must be between 1 and window_runs".into());
        }
        if self.peak_context_permille == 0 || self.peak_context_permille > 1000 {
            return Err("peak_context_permille must be between 1 and 1000".into());
        }
        if self.cost_multiple_percent < 100 || self.cost_multiple_percent > 10_000 {
            return Err("cost_multiple_percent must be between 100 and 10000".into());
        }
        Ok(())
    }
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
    custom_steps: BTreeMap<Step, CustomStep>,
    pub checks: BTreeMap<String, CheckCommand>,
    pub protected_paths: Vec<String>,
    pub branch_rules: BranchRules,
    pub budgets: ProjectBudgets,
    pub when_unsure: WhenUnsure,
    pub review: ReviewSettings,
    /// Projects by normalised workspace path.
    pub projects: BTreeMap<String, Project>,
    custom_pipelines: BTreeMap<String, Pipeline>,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ProfileFile {
    steps: BTreeMap<Step, StepOverrides>,
    custom_steps: BTreeMap<Step, CustomStepFile>,
    checks: BTreeMap<String, CheckCommand>,
    protected_paths: Vec<String>,
    branch_rules: BranchRules,
    budgets: ProjectBudgets,
    when_unsure: WhenUnsure,
    review: ReviewSettings,
    projects: BTreeMap<String, ProjectFile>,
    pipelines: BTreeMap<String, PipelineFile>,
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
        let custom_steps = resolve_custom_steps(&file.custom_steps, path, source)?;
        for step in custom_steps.keys() {
            let options = file.steps.remove(step).unwrap_or_default();
            steps.insert(*step, resolve_step(*step, options, path, source)?);
        }
        // A settings table for a custom step that was never defined would
        // otherwise be dropped without a word.
        if let Some(step) = file.steps.keys().find(|step| step.is_custom()) {
            let key = format!("steps.{}", step.as_str());
            let offset = source.find(&key).unwrap_or(0);
            return Err(diagnostic(
                path,
                source,
                offset,
                key,
                "this custom step is not defined under [custom_steps]".into(),
            ));
        }

        if let Err(why) = file.review.check() {
            let offset = source.find("[review]").unwrap_or(0);
            return Err(diagnostic(path, source, offset, "review".into(), why));
        }
        let custom_pipelines = resolve_pipelines(&file.pipelines, &custom_steps, path, source)?;
        let projects = resolve_projects(&file.projects, &custom_pipelines, path, source)?;

        Ok(Self {
            steps,
            custom_steps,
            checks: file.checks,
            protected_paths: file.protected_paths,
            branch_rules: file.branch_rules,
            budgets: file.budgets,
            when_unsure: file.when_unsure,
            review: file.review,
            projects,
            custom_pipelines,
        })
    }

    /// Built-in pipelines first, then the custom ones by name.
    #[must_use]
    pub fn pipelines(&self) -> Vec<Pipeline> {
        let mut all = builtin_pipelines();
        all.extend(self.custom_pipelines.values().cloned());
        all
    }

    #[must_use]
    pub fn pipeline(&self, name: &str) -> Option<Pipeline> {
        self.pipelines().into_iter().find(|each| each.name == name)
    }

    /// A pipeline as it would be if `name` were added: the named base with the
    /// given steps switched, checked by the same rules as a saved one.
    ///
    /// # Errors
    /// Returns a sentence when the base is unknown or the steps cannot run in order.
    pub fn build_pipeline(
        &self,
        name: &str,
        based_on: &str,
        on: &[Step],
        if_needed: &[Step],
        off: &[Step],
        rounds: Option<u32>,
    ) -> Result<Pipeline, String> {
        let known: BTreeMap<String, Pipeline> = self
            .pipelines()
            .into_iter()
            .map(|pipeline| (pipeline.name.clone(), pipeline))
            .collect();
        let file = PipelineFile {
            based_on: Some(based_on.to_owned()),
            on: on.to_vec(),
            if_needed: if_needed.to_vec(),
            off: off.to_vec(),
            rounds,
        };
        let made = pipeline::resolve(name, &file, &known)?;
        check_defined(&made, &self.custom_steps)?;
        Ok(made)
    }

    /// The project for a workspace folder, however its path is spelled.
    #[must_use]
    pub fn project(&self, workspace: &str) -> Option<&Project> {
        self.projects.get(&normalize_path(workspace))
    }

    /// The profile a ticket in `workspace` runs under. A project only adds to
    /// the profile's protected paths and only lowers its budgets, so a project
    /// entry can never make a rule weaker, whatever it says.
    #[must_use]
    pub fn for_project(&self, workspace: Option<&str>) -> Self {
        let mut merged = self.clone();
        let Some(project) = workspace.and_then(|path| self.project(path)) else {
            return merged;
        };
        for pattern in &project.protected_paths {
            if !merged.protected_paths.contains(pattern) {
                merged.protected_paths.push(pattern.clone());
            }
        }
        merged.budgets = ProjectBudgets {
            per_ticket_microusd: lower(
                self.budgets.per_ticket_microusd,
                project.budgets.per_ticket_microusd,
            ),
            daily_microusd: lower(self.budgets.daily_microusd, project.budgets.daily_microusd),
        };
        merged
    }

    /// The custom steps the owner defined, by slot.
    #[must_use]
    pub const fn custom_steps(&self) -> &BTreeMap<Step, CustomStep> {
        &self.custom_steps
    }

    #[must_use]
    pub fn custom_step(&self, step: Step) -> Option<&CustomStep> {
        self.custom_steps.get(&step)
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

/// The smaller of two optional caps; `None` means no cap, so a cap beats none.
fn lower(global: Option<NonZeroU64>, project: Option<NonZeroU64>) -> Option<NonZeroU64> {
    match (global, project) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Custom pipelines, each resolved against the built-ins and the ones before
/// it. A pass per level lets one custom pipeline build on another.
/// A pipeline may run only the custom steps the profile defines.
fn check_defined(pipeline: &Pipeline, defined: &BTreeMap<Step, CustomStep>) -> Result<(), String> {
    for step in pipeline.custom_steps() {
        if !defined.contains_key(&step) {
            return Err(format!(
                "pipeline {:?} runs {}, which is not defined under [custom_steps]",
                pipeline.name,
                step.as_str()
            ));
        }
    }
    Ok(())
}

fn resolve_custom_steps(
    files: &BTreeMap<Step, CustomStepFile>,
    path: &Path,
    source: &str,
) -> Result<BTreeMap<Step, CustomStep>, ProfileDiagnostic> {
    let mut steps = BTreeMap::new();
    for (step, file) in files {
        let key = format!("custom_steps.{}", step.as_str());
        let at = |why: String| {
            let offset = source.find(&key).unwrap_or(0);
            diagnostic(path, source, offset, key.clone(), why)
        };
        if !step.is_custom() {
            return Err(at("only custom_1 to custom_8 can be defined here".into()));
        }
        CustomStep::check(&file.name, &file.prompt).map_err(at)?;
        steps.insert(
            *step,
            CustomStep {
                name: file.name.trim().to_owned(),
                prompt: file.prompt.clone(),
            },
        );
    }
    Ok(steps)
}

fn resolve_pipelines(
    files: &BTreeMap<String, PipelineFile>,
    defined: &BTreeMap<Step, CustomStep>,
    path: &Path,
    source: &str,
) -> Result<BTreeMap<String, Pipeline>, ProfileDiagnostic> {
    let mut known: BTreeMap<String, Pipeline> = builtin_pipelines()
        .into_iter()
        .map(|each| (each.name.clone(), each))
        .collect();
    let mut custom = BTreeMap::new();
    let mut waiting: Vec<(&String, &PipelineFile)> = files.iter().collect();
    while !waiting.is_empty() {
        let before = waiting.len();
        let mut later = Vec::new();
        for (name, file) in waiting {
            let at = |why: String| {
                let offset = source.find(&format!("pipelines.{name}")).unwrap_or(0);
                diagnostic(path, source, offset, format!("pipelines.{name}"), why)
            };
            pipeline::check_name(name).map_err(at)?;
            if known.get(name).is_some_and(|each| each.builtin) {
                return Err(at(format!(
                    "{name:?} is a built-in pipeline and is read-only"
                )));
            }
            let base = file.based_on.as_deref().unwrap_or(DEFAULT_PIPELINE);
            if !known.contains_key(base) && files.contains_key(base) && base != name {
                later.push((name, file));
                continue;
            }
            let made = pipeline::resolve(name, file, &known).map_err(at)?;
            check_defined(&made, defined).map_err(at)?;
            known.insert(name.clone(), made.clone());
            custom.insert(name.clone(), made);
        }
        if later.len() == before {
            let name = later[0].0;
            let offset = source.find(&format!("pipelines.{name}")).unwrap_or(0);
            return Err(diagnostic(
                path,
                source,
                offset,
                format!("pipelines.{name}"),
                "based_on forms a loop".into(),
            ));
        }
        waiting = later;
    }
    Ok(custom)
}

fn resolve_projects(
    files: &BTreeMap<String, ProjectFile>,
    custom: &BTreeMap<String, Pipeline>,
    path: &Path,
    source: &str,
) -> Result<BTreeMap<String, Project>, ProfileDiagnostic> {
    let mut projects = BTreeMap::new();
    for (key, file) in files {
        let at = |why: String| {
            let offset = source.find(key.as_str()).unwrap_or(0);
            diagnostic(path, source, offset, format!("projects.{key}"), why)
        };
        let normal = normalize_path(key);
        if normal.is_empty() {
            return Err(at("a project needs a workspace path as its key".into()));
        }
        if let Some(name) = &file.default_pipeline
            && !custom.contains_key(name)
            && !builtin_pipelines().iter().any(|each| &each.name == name)
        {
            return Err(at(format!(
                "default_pipeline {name:?} is not a known pipeline"
            )));
        }
        if let Some(repo) = &file.github_repo
            && (repo.len() > 100 || repo.chars().any(char::is_whitespace))
        {
            return Err(at("github_repo must be a short name like owner/repo".into()));
        }
        let project = Project {
            name: file.name.clone().unwrap_or_else(|| leaf(&normal)),
            path: normal.clone(),
            default_pipeline: file.default_pipeline.clone(),
            agent: file.agent,
            github_repo: file.github_repo.clone(),
            protected_paths: file.protected_paths.clone(),
            budgets: file.budgets,
        };
        if projects.insert(normal, project).is_some() {
            return Err(at("two projects name the same folder".into()));
        }
    }
    Ok(projects)
}
