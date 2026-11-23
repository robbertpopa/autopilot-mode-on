//! Pipelines and projects. A pipeline says which steps a ticket runs; a
//! project says what is true of one workspace folder. Built-in pipelines live
//! in code and cannot be edited; custom ones are read from `profile.toml`.
//!
//! A pipeline can switch steps on or off. It cannot change what a step asks:
//! `ask_after`, protected paths and budgets stay with the profile and the
//! policy, so no pipeline definition can loosen them.

use std::collections::BTreeMap;

use autopilot_core::Step;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Agent, ProjectBudgets};

/// Whether a step runs in a pipeline.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepMode {
    On,
    /// Runs only when the ticket calls for it (Clarify after open questions,
    /// Design after a screen change).
    IfNeeded,
    Off,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Pipeline {
    pub name: String,
    /// Built-in pipelines are read-only.
    pub builtin: bool,
    modes: BTreeMap<Step, StepMode>,
    /// A lower cap on build rounds than the step setting, if any.
    pub rounds: Option<u32>,
}

impl Pipeline {
    #[must_use]
    pub fn mode(&self, step: Step) -> StepMode {
        // A custom step runs only where a pipeline names it.
        let unlisted = if step.is_custom() {
            StepMode::Off
        } else {
            StepMode::On
        };
        self.modes.get(&step).copied().unwrap_or(unlisted)
    }

    /// The custom steps this pipeline runs, in pipeline order.
    #[must_use]
    pub fn custom_steps(&self) -> Vec<Step> {
        self.modes
            .iter()
            .filter(|(step, mode)| step.is_custom() && **mode != StepMode::Off)
            .map(|(step, _)| *step)
            .collect()
    }

    /// The steps this pipeline never runs, in pipeline order.
    #[must_use]
    pub fn off(&self) -> Vec<Step> {
        Step::ALL
            .into_iter()
            .filter(|step| self.mode(*step) == StepMode::Off)
            .collect()
    }
}

fn make(name: &str, overrides: &[(Step, StepMode)], rounds: Option<u32>) -> Pipeline {
    let mut modes: BTreeMap<Step, StepMode> = Step::ALL
        .into_iter()
        .map(|step| (step, StepMode::On))
        .collect();
    modes.insert(Step::Clarify, StepMode::IfNeeded);
    modes.insert(Step::Design, StepMode::IfNeeded);
    for (step, mode) in overrides {
        modes.insert(*step, *mode);
    }
    Pipeline {
        name: name.to_owned(),
        builtin: true,
        modes,
        rounds,
    }
}

/// The pipelines every profile has. `full` is what a ticket ran before
/// pipelines could be chosen, so a ticket with no choice recorded is `full`.
#[must_use]
pub fn builtin_pipelines() -> Vec<Pipeline> {
    use Step::{BuildCheck, Clarify, Design, Plan, PullRequest};
    use StepMode::{IfNeeded, Off, On};
    vec![
        make("full", &[], None),
        make("bug-fix", &[(Design, Off)], None),
        make("quick-change", &[(Clarify, Off), (Design, Off)], Some(2)),
        make("ui-change", &[(Design, On)], None),
        make(
            "ask-only",
            &[
                (Design, Off),
                (Plan, Off),
                (BuildCheck, Off),
                (PullRequest, Off),
                (Clarify, IfNeeded),
            ],
            None,
        ),
    ]
}

/// The name a ticket without a project and the one a ticket with no choice
/// recorded fall back to.
pub const DEFAULT_PIPELINE: &str = "full";
pub const ASK_ONLY: &str = "ask-only";

/// Reject a mode table the engine could not run in order.
///
/// # Errors
/// Returns a sentence naming the step that cannot be switched this way.
pub fn validate_modes(modes: &BTreeMap<Step, StepMode>) -> Result<(), String> {
    let mode = |step: Step| {
        modes.get(&step).copied().unwrap_or(if step.is_custom() {
            StepMode::Off
        } else {
            StepMode::On
        })
    };
    let runs = |step| mode(step) != StepMode::Off;
    if mode(Step::Understand) != StepMode::On {
        return Err("understand always runs first".into());
    }
    if runs(Step::BuildCheck) && mode(Step::Plan) != StepMode::On {
        return Err("build needs a plan: keep plan on".into());
    }
    if runs(Step::PullRequest) && !runs(Step::BuildCheck) {
        return Err("a pull request needs build: keep build_check on".into());
    }
    if runs(Step::Plan) && mode(Step::Plan) != StepMode::On {
        return Err("plan is either on or off".into());
    }
    for (step, each) in modes {
        if !step.is_custom() || *each == StepMode::Off {
            continue;
        }
        if *each == StepMode::IfNeeded {
            return Err(format!("{} is either on or off", step.as_str()));
        }
        if !runs(Step::BuildCheck) {
            return Err(format!(
                "{} runs after the build: keep build_check on",
                step.as_str()
            ));
        }
    }
    Ok(())
}

/// A custom pipeline as written in `profile.toml`.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineFile {
    /// The built-in or custom pipeline this one starts from. Default `full`.
    pub based_on: Option<String>,
    pub on: Vec<Step>,
    pub if_needed: Vec<Step>,
    pub off: Vec<Step>,
    pub rounds: Option<u32>,
}

/// What is true of one workspace folder, as written in `profile.toml`.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectFile {
    pub name: Option<String>,
    pub default_pipeline: Option<String>,
    pub agent: Option<Agent>,
    pub github_repo: Option<String>,
    /// Added to the profile's own list; it can never replace or empty it.
    pub protected_paths: Vec<String>,
    /// A cap lower than the profile's, never higher.
    pub budgets: ProjectBudgets,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Project {
    /// The workspace folder, normalised: forward slashes, no trailing slash.
    pub path: String,
    pub name: String,
    pub default_pipeline: Option<String>,
    pub agent: Option<Agent>,
    pub github_repo: Option<String>,
    pub protected_paths: Vec<String>,
    pub budgets: ProjectBudgets,
}

/// One spelling for a folder, so `D:\work\app\` and `D:/work/app` are the same
/// project. Case is kept: Linux paths differ by case.
#[must_use]
pub fn normalize_path(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let trimmed = slashed.trim_end_matches('/');
    if trimmed.is_empty() && !slashed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The folder name, for display only. Two projects can share it, so it is
/// never used to tell them apart.
#[must_use]
pub fn leaf(path: &str) -> String {
    normalize_path(path)
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Resolve a custom pipeline against the ones before it.
///
/// # Errors
/// Returns a sentence when the base is unknown or the steps cannot run.
pub fn resolve(
    name: &str,
    file: &PipelineFile,
    known: &BTreeMap<String, Pipeline>,
) -> Result<Pipeline, String> {
    let base_name = file.based_on.as_deref().unwrap_or(DEFAULT_PIPELINE);
    let base = known
        .get(base_name)
        .ok_or_else(|| format!("based_on {base_name:?} is not a known pipeline"))?;
    let mut modes = base.modes.clone();
    for (steps, mode) in [
        (&file.on, StepMode::On),
        (&file.if_needed, StepMode::IfNeeded),
        (&file.off, StepMode::Off),
    ] {
        for step in steps {
            modes.insert(*step, mode);
        }
    }
    validate_modes(&modes).map_err(|why| format!("pipeline {name:?}: {why}"))?;
    if file.rounds == Some(0) {
        return Err(format!("pipeline {name:?}: rounds must be at least 1"));
    }
    Ok(Pipeline {
        name: name.to_owned(),
        builtin: false,
        modes,
        rounds: file.rounds.or(base.rounds),
    })
}

/// A pipeline name is short and plain, so it is safe in a file, a log line
/// and a window title.
///
/// # Errors
/// Returns a sentence when the name is empty, long or has odd characters.
pub fn check_name(name: &str) -> Result<(), String> {
    let plain = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if name.is_empty() || name.len() > 40 || !plain {
        return Err(format!(
            "pipeline name {name:?} must be 1 to 40 characters of a-z, 0-9 and '-'"
        ));
    }
    Ok(())
}

/// A custom step as written in `profile.toml`: what it is called and what the
/// agent is told. Its model, effort, limits and `ask_after` live in the
/// matching `[steps.custom_N]` table, like any other step.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CustomStepFile {
    pub name: String,
    pub prompt: String,
}

/// A custom step the owner defined.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CustomStep {
    pub name: String,
    pub prompt: String,
}

impl CustomStep {
    /// The longest prompt a step may hold. The prompt is part of the agent's
    /// instructions, so it is bounded like the other owner-written text.
    pub const MAX_PROMPT: usize = 4000;

    /// # Errors
    /// Returns a sentence when the name or the prompt cannot be used.
    pub fn check(name: &str, prompt: &str) -> Result<(), String> {
        let plain = |text: &str| !text.chars().any(char::is_control);
        if name.trim().is_empty() || name.chars().count() > 40 || !plain(name) {
            return Err("a step name is 1 to 40 plain characters".into());
        }
        if prompt.trim().is_empty() {
            return Err("a step needs a prompt: say what the agent should do".into());
        }
        if prompt.chars().count() > Self::MAX_PROMPT {
            return Err(format!(
                "a step prompt is at most {} characters",
                Self::MAX_PROMPT
            ));
        }
        if prompt
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err("a step prompt has a control character".into());
        }
        Ok(())
    }
}
