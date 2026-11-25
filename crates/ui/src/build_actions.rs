//! Build and the pull request, started from the window.
//!
//! These go through the same paths as the command line: a real agent builds
//! only inside the validated Linux boundary (`sandbox_boundary`), the review
//! runs after the checks, and a pull request is a forced approval of one commit
//! before anything leaves the machine. The window adds nothing of its own to
//! that: where a refusal has a reason, the reason is the text shown.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use autopilot_agent_claude::{BuildConfig, ClaudeAgent, ClaudeConfig, Sandbox};
use autopilot_core::{Step, TicketId};
use autopilot_engine::{BuildOutcome, BuildSetup, Isolation, PublishSetup};
use autopilot_profile::Auth;

use crate::backend::{AgentChoice, Options, beside, claude_config, guardian, ticket_folder};

/// The remote the window publishes to.
const REMOTE: &str = "origin";

/// What the window needs to know about the ticket's project.
struct Project {
    repository: PathBuf,
    places: autopilot_engine::Places,
}

fn project_of(options: &Options, ticket: &TicketId) -> Result<Project, String> {
    let repository = ticket_folder(options, ticket).ok_or_else(|| {
        "Build needs the project folder, and this ticket has none. Start a ticket with a project \
         folder chosen: the agent builds in a copy of it."
            .to_owned()
    })?;
    let places = autopilot_engine::places(&repository, &guardian(), REMOTE).map_err(|error| {
        format!(
            "The project folder is not ready for Build: {error}. It must be a Git repository \
             with a branch checked out."
        )
    })?;
    Ok(Project { repository, places })
}

/// Where every ticket's copy of the project lives. It is made here: the
/// worktree manager opens an existing folder and will not guess one.
fn worktrees(options: &Options) -> Result<PathBuf, String> {
    let folder = options.data_dir.join("worktrees");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Could not make {}: {error}", folder.display()))?;
    Ok(folder)
}

/// Run the Build step for the approved plan.
///
/// # Errors
/// Returns the reason in plain words: no project folder, the sandbox or the API
/// key not being ready, or the build failing to run.
pub(crate) fn make_build(options: &Options, ticket: &TicketId) -> Result<String, String> {
    let directory = options.data_dir.as_path();
    let profile =
        autopilot_engine::load_profile(&options.profile).map_err(|error| error.to_string())?;
    let project = project_of(options, ticket)?;
    let history =
        autopilot_engine::watch(directory, ticket, 0).map_err(|error| error.to_string())?;
    let words =
        autopilot_engine::ticket_words(ticket, &history).map_err(|error| error.to_string())?;
    let guardian = guardian();
    let cli = beside("autopilot")?;
    let hook = BuildConfig {
        hook_command: format!("\"{}\" hook", cli.display()),
        data_dir: directory.to_path_buf(),
    };

    let (agent, isolation, review) = match AgentChoice::read(directory) {
        AgentChoice::Practice => (
            practice_agent(guardian.clone(), hook)?,
            Isolation::Fixture,
            false,
        ),
        AgentChoice::Claude => claude_build_agent(options, ticket, guardian.clone(), hook)?,
    };
    let setup = BuildSetup {
        repository: &project.repository,
        base_branch: &project.places.base,
        worktrees: &worktrees(options)?,
        git: PathBuf::from("git"),
        guardian,
        profile: &profile,
        review,
    };
    match autopilot_engine::run_build(directory, &words, &agent, &setup, isolation)
        .map_err(|error| error.to_string())?
    {
        BuildOutcome::Ready => Ok("Build passed: every check is green".to_owned()),
        BuildOutcome::NeedsYou { reason, .. } => {
            Ok(format!("Build stopped and needs you: {reason}"))
        }
    }
}

/// The practice agent builds with the fixture program, in a boundary that
/// holds nothing real: it exists to try the window, not to change code.
fn practice_agent(guardian: PathBuf, hook: BuildConfig) -> Result<ClaudeAgent, String> {
    let config = ClaudeConfig {
        auth: Auth::Subscription,
        model: "fake".into(),
        effort: None,
        max_turns: 8,
        max_budget_usd: None,
        executable: Some(beside("autopilot-fake-agent")?),
        timeout: Duration::from_secs(120),
    };
    Ok(ClaudeAgent::new(config, guardian)
        .map_err(|error| error.to_string())?
        .with_build(hook))
}

/// Discover the Linux side and run the escape probe, the same way for Build and
/// for the Sandbox page. The probe takes about five seconds, so callers run it
/// off the window's thread. The error is the raw reason, row by row.
pub(crate) fn probe_sandbox(
    options: &Options,
) -> Result<autopilot_agent_claude::Validated, String> {
    let distro = std::env::var("AUTOPILOT_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu".to_owned());
    let folder = options.data_dir.join("sandbox");
    autopilot_agent_claude::sandbox_boundary(&distro, &folder, &guardian())
        .map_err(|error| error.to_string())
}

/// The probe Build uses: shared by Builds that start together, so three
/// tickets do not each spend five seconds on the same check. The Sandbox page
/// calls `probe_sandbox` itself and always runs it fresh.
fn shared_boundary(options: &Options) -> Result<autopilot_agent_claude::Validated, String> {
    static PROBE: crate::shared_probe::SharedProbe<autopilot_agent_claude::Validated> =
        crate::shared_probe::SharedProbe::new();
    PROBE.get(|| probe_sandbox(options))
}

/// Why the Build cannot be reset now, in words for a greyed button; `None`
/// when it can. The engine checks the same rules again before it removes
/// anything.
pub(crate) fn reset_refusal(view: &autopilot_engine::TicketView) -> Option<&'static str> {
    use autopilot_pipeline::StepState;
    let state = |step| view.step(step).map(|card| &card.state);
    if view.steps.iter().any(|card| card.state.is_active()) {
        return Some("a step is running or waiting for you: stop it or answer it first");
    }
    if matches!(state(Step::BuildCheck), None | Some(StepState::Idle)) {
        return Some("the build has not run yet");
    }
    if !matches!(
        state(Step::PullRequest),
        None | Some(StepState::Idle | StepState::Skipped(_))
    ) {
        return Some("a pull request was already asked for from this build");
    }
    None
}

/// Remove the Build's copy of the project and its branch, then put the step
/// back. The files in the copy are gone for good: the window asks first.
///
/// # Errors
/// Returns the reason in plain words: no project folder, a step running, a
/// pull request already asked for, or Git refusing the removal.
pub(crate) fn reset_build(options: &Options, ticket: &TicketId) -> Result<String, String> {
    let repository = ticket_folder(options, ticket).ok_or_else(|| {
        "The project folder this ticket was started in is gone, so its copy cannot be removed          safely."
            .to_owned()
    })?;
    let setup = autopilot_engine::ResetSetup {
        repository: &repository,
        worktrees: &worktrees(options)?,
        git: PathBuf::from("git"),
        guardian: guardian(),
    };
    autopilot_engine::reset_build(&options.data_dir, ticket, &setup)
        .map(|_| {
            "Build reset: its copy of the project and its branch were removed. Build starts again from the base branch."
                .to_owned()
        })
        .map_err(|error| error.to_string())
}

/// The real agent, inside the Linux boundary. Every way this can be refused has
/// its own sentence, because the owner reads it where they pressed the button.
fn claude_build_agent(
    options: &Options,
    ticket: &TicketId,
    guardian: PathBuf,
    hook: BuildConfig,
) -> Result<(ClaudeAgent, Isolation, bool), String> {
    let config = claude_config(options, Step::BuildCheck, Some(ticket))?;
    if config.auth != Auth::Api {
        return Err(
            "Build with Claude runs in API-key mode only: a subscription sign-in cannot be kept \
             out of the agent's reach. Set Build to use the API key in Settings, Pipeline steps, \
             or switch the agent to Practice."
                .to_owned(),
        );
    }
    if std::env::var_os(autopilot_agent_claude::API_KEY_SOURCE).is_none_or(|key| key.is_empty()) {
        return Err(format!(
            "Build with Claude needs the dedicated API key in {}. Save it from the welcome \
             screen or Settings, then restart the window.",
            autopilot_agent_claude::API_KEY_SOURCE
        ));
    }
    let boundary = shared_boundary(options).map_err(|error| {
        let (place, setup) = if cfg!(target_os = "macos") {
            ("macOS", "tools/macos/setup.sh")
        } else {
            ("Linux", "tools/wsl")
        };
        format!(
            "The {place} sandbox is not ready, so Build stays off: {error}. Set it up with \
             {setup} (docs/spikes/S6-containment.md)."
        )
    })?;
    let sandbox = Sandbox {
        boundary: boundary.clone(),
        config_root: options.data_dir.join("claude-config"),
    };
    let agent = ClaudeAgent::sandboxed(config, guardian, sandbox)
        .map_err(|error| error.to_string())?
        .with_build(hook);
    Ok((agent, Isolation::Wsl(boundary), true))
}

fn publish_setup<'a>(
    options: &Options,
    project: &'a Project,
    profile: &'a autopilot_profile::Profile,
    worktrees: &'a Path,
    repo: &'a autopilot_engine::Repo,
) -> PublishSetup<'a> {
    let _ = options;
    PublishSetup {
        repository: &project.repository,
        base_branch: &project.places.base,
        worktrees,
        git: PathBuf::from("git"),
        guardian: guardian(),
        profile,
        remote: REMOTE,
        repo,
    }
}

/// Ask for the owner's approval of one commit. Nothing leaves the machine.
///
/// # Errors
/// Returns the reason in plain words.
pub(crate) fn prepare_pull_request(options: &Options, ticket: &TicketId) -> Result<String, String> {
    let directory = options.data_dir.as_path();
    let profile =
        autopilot_engine::load_profile(&options.profile).map_err(|error| error.to_string())?;
    let project = project_of(options, ticket)?;
    let repo = project.places.github.clone().ok_or_else(|| {
        format!(
            "The project's {REMOTE} remote does not point at a GitHub repository, so there is \
             nowhere to open a pull request."
        )
    })?;
    let history =
        autopilot_engine::watch(directory, ticket, 0).map_err(|error| error.to_string())?;
    let words =
        autopilot_engine::ticket_words(ticket, &history).map_err(|error| error.to_string())?;
    let trees = worktrees(options)?;
    let setup = publish_setup(options, &project, &profile, &trees, &repo);
    autopilot_engine::prepare_publish(directory, &words, &setup)
        .map(|_| format!("Waiting for your approval to open a pull request on {repo}"))
        .map_err(|error| error.to_string())
}

/// Push the approved commit and open the draft pull request.
///
/// # Errors
/// Returns the reason in plain words.
pub(crate) fn open_pull_request(options: &Options, ticket: &TicketId) -> Result<String, String> {
    let directory = options.data_dir.as_path();
    let profile =
        autopilot_engine::load_profile(&options.profile).map_err(|error| error.to_string())?;
    let project = project_of(options, ticket)?;
    let repo = project.places.github.clone().ok_or_else(|| {
        format!("The project's {REMOTE} remote does not point at a GitHub repository.")
    })?;
    let history =
        autopilot_engine::watch(directory, ticket, 0).map_err(|error| error.to_string())?;
    let words =
        autopilot_engine::ticket_words(ticket, &history).map_err(|error| error.to_string())?;
    let trees = worktrees(options)?;
    let setup = publish_setup(options, &project, &profile, &trees, &repo);
    let gh =
        autopilot_engine::gh_for(guardian(), repo.clone()).map_err(|error| error.to_string())?;
    let published = autopilot_engine::run_publish(directory, &words, &setup, &gh)
        .map_err(|error| error.to_string())?;
    let pr = published.pull_request();
    Ok(format!(
        "Draft pull request #{} opened: {}",
        pr.number, pr.url
    ))
}
