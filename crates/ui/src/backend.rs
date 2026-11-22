//! Everything that touches the data directory. The window never opens the
//! store itself: it asks `engine` and gets plain view structs back.
//!
//! One watcher thread looks for changes and asks for a repaint only when a
//! ticket really changed. Actions run on their own thread, because an engine
//! action may wait on the run lock or on an agent.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use autopilot_agent_fixture::FixtureAgent;
use autopilot_core::{Money, RequestId, Step, Ticket, TicketId};
use autopilot_engine::{TicketView, load_ticket, spent_since, tickets};
use autopilot_pipeline::clarify::Choice;
use autopilot_pipeline::inbox::{Decision, Inbox, Reply, RequestBody, check_reply};
use eframe::egui;

/// Where the window reads and writes.
#[derive(Clone, Debug)]
pub struct Options {
    pub data_dir: PathBuf,
    pub profile: PathBuf,
    /// Open on this screen: `design`, `cost`, `run` or `step`.
    pub view: Option<String>,
    /// The ticket `run` and `step` open on. Defaults to the newest.
    pub ticket: Option<String>,
}

/// Everything the window shows that comes from the store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub tickets: Vec<TicketView>,
    pub today_api: Money,
    pub today_subscription: Money,
    /// Why the data could not be read, if it could not.
    pub problem: Option<String>,
}

pub enum Message {
    Snapshot(Snapshot),
    /// What GitHub, Claude, Codex and Azure say about who is signed in.
    Accounts(Box<autopilot_accounts::Accounts>),
    /// The outcome of an action: a short confirmation, or the reason it failed.
    Done(Result<String, String>),
    /// The answer of the release check: a newer version, nothing, or why not.
    Update(Result<Option<autopilot_update::Update>, String>),
    /// The installer was downloaded and checked, and handed to the system.
    Installing(Result<(), String>),
}

/// What the owner can ask for from the window. Each one is an engine action
/// the CLI has too.
#[derive(Clone, Debug)]
pub enum Action {
    /// Create a ticket and run Understand, then decide Clarify.
    Start {
        title: String,
        description: String,
        /// Files, images and folders to attach, as absolute paths.
        attachments: Vec<PathBuf>,
        /// The project folder the agent reads, if one was chosen.
        workspace: Option<PathBuf>,
    },
    /// Attach files, images or folders to a ticket that exists.
    Attach {
        ticket: TicketId,
        paths: Vec<PathBuf>,
    },
    /// Approve, or reject, what a ticket is waiting on.
    Decide {
        ticket: TicketId,
        approve: bool,
        /// Stop asking after this kind of step in the project. Ignored for
        /// a forced confirmation and for the plan.
        always: bool,
    },
    /// Answer a waiting question with the recommended options.
    AcceptDefaults {
        ticket: TicketId,
    },
    /// Run Understand again for a ticket, with the words it started with.
    RerunUnderstand(TicketId),
    /// Run Understand again for this ticket only, with a higher turn cap.
    RerunUnderstandWithMoreTurns(TicketId),
    /// Make the plan once Understand and Clarify are settled. The plan then
    /// waits for the owner's approval.
    MakePlan(TicketId),
    /// Make the plan again, replacing the one that was made or approved.
    RerunPlan(TicketId),
    /// Ask the orchestrator to choose models and effort for the later steps.
    Orchestrate(TicketId),
    /// Make the design (static mockups) for a ticket that changes a screen,
    /// or skip the step when it needs none.
    MakeDesign(TicketId),
    /// Make the design again, replacing the one that was made.
    RerunDesign(TicketId),
    /// Show the saved mockups in the system file manager.
    OpenDesign(TicketId),
    /// Explain a step's result in plain words, once; the answer is kept.
    Explain {
        ticket: TicketId,
        step: Step,
    },
    Pause(TicketId),
    Resume(TicketId),
    Stop(TicketId),
}

#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

impl Action {
    /// What is happening, in words, while the action runs.
    #[must_use]
    pub const fn describe(&self) -> &'static str {
        match self {
            Self::Start { .. } => "Reading your request",
            Self::Orchestrate(_) => "The orchestrator is choosing models and effort",
            Self::MakePlan(_) | Self::RerunPlan(_) => "Making the plan",
            Self::MakeDesign(_) | Self::RerunDesign(_) => "Making the design",
            Self::RerunUnderstand(_) => "Reading the request again",
            Self::Explain { .. } => "Explaining in plain words",
            _ => "Working",
        }
    }
}

/// The data as it is right now.
#[must_use]
pub fn read(options: &Options) -> Snapshot {
    let mut snapshot = Snapshot::default();
    let ids = match tickets(&options.data_dir) {
        Ok(ids) => ids,
        Err(error) => {
            snapshot.problem = Some(error.to_string());
            return snapshot;
        }
    };
    for id in ids {
        match load_ticket(&options.data_dir, &id) {
            Ok(view) => snapshot.tickets.push(view),
            Err(error) => {
                snapshot.problem = Some(format!("{}: {error}", id.as_str()));
            }
        }
    }
    // Newest first, so what you just started is at the top.
    snapshot.tickets.reverse();
    let now = now_ms();
    if let Ok((api, subscription)) = spent_since(&options.data_dir, now - now % 86_400_000) {
        snapshot.today_api = api;
        snapshot.today_subscription = subscription;
    }
    snapshot
}

/// Start the thread that watches the data directory. It sends a snapshot only
/// when something changed, then wakes the window; the window is never
/// repainted on a timer.
pub fn watch(options: Options, context: egui::Context, sender: Sender<Message>) {
    thread::spawn(move || {
        let mut last: Option<Snapshot> = None;
        loop {
            let snapshot = read(&options);
            if last.as_ref() != Some(&snapshot) {
                last = Some(snapshot.clone());
                if sender.send(Message::Snapshot(snapshot)).is_err() {
                    return;
                }
                context.request_repaint();
            }
            thread::sleep(Duration::from_millis(400));
        }
    });
}

/// Ask the four tools who is signed in. Each takes a second or two, so it runs
/// on its own thread and the window shows the answer when it arrives.
pub fn fetch_accounts(context: egui::Context, sender: Sender<Message>) {
    thread::spawn(move || {
        let guardian = std::env::current_exe()
            .map(|exe| {
                exe.with_file_name(format!(
                    "autopilot-process-guardian{}",
                    std::env::consts::EXE_SUFFIX
                ))
            })
            .unwrap_or_default();
        let accounts = autopilot_accounts::collect(guardian);
        let _ = sender.send(Message::Accounts(Box::new(accounts)));
        context.request_repaint();
    });
}

fn guardian() -> std::path::PathBuf {
    std::env::current_exe()
        .map(|exe| {
            exe.with_file_name(format!(
                "autopilot-process-guardian{}",
                std::env::consts::EXE_SUFFIX
            ))
        })
        .unwrap_or_default()
}

/// Look for a newer signed release. Set `AUTOPILOT_NO_UPDATE_CHECK` to skip it.
pub fn check_for_update(context: egui::Context, sender: Sender<Message>) {
    if std::env::var_os("AUTOPILOT_NO_UPDATE_CHECK").is_some() {
        return;
    }
    thread::spawn(move || {
        let found = autopilot_update::check(&guardian(), env!("CARGO_PKG_VERSION"));
        let _ = sender.send(Message::Update(found));
        context.request_repaint();
    });
}

/// Download the installer, check it against the signed checksum, and open it.
pub fn install_update(
    context: egui::Context,
    sender: Sender<Message>,
    update: autopilot_update::Update,
) {
    thread::spawn(move || {
        let guardian = guardian();
        let directory = std::env::temp_dir().join("autopilot-mode-on-updates");
        let outcome = autopilot_update::download(&guardian, &update, &directory)
            .and_then(|file| autopilot_update::open_installer(&guardian, &file));
        let _ = sender.send(Message::Installing(outcome));
        context.request_repaint();
    });
}

/// The fixture agent sits beside this program, or one folder up when the
/// program is a test binary in `deps`.
/// Which agent the window runs a ticket's steps with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChoice {
    /// The built-in agent that answers from fixed text. Free and offline.
    /// Only for trying the app; it is never the default.
    Practice,
    /// Claude Code, billed to the signed-in subscription.
    #[default]
    Claude,
}

const AGENT_FILE: &str = "ui-agent.txt";
const NOTIFY_FILE: &str = "ui-notify.txt";

/// Whether the window may show desktop notifications. On unless turned off.
#[must_use]
pub fn notifications_on(data_dir: &Path) -> bool {
    std::fs::read_to_string(data_dir.join(NOTIFY_FILE)).map_or(true, |text| text.trim() != "off")
}

pub fn save_notifications(data_dir: &Path, on: bool) {
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(data_dir.join(NOTIFY_FILE), if on { "on" } else { "off" });
}

/// Show a desktop notification without blocking the window. A failure is
/// ignored: a missing system tool must never disturb a run.
pub fn notify(title: String, text: String) {
    let Ok(guardian) = beside("autopilot-process-guardian") else {
        return;
    };
    let _ = thread::Builder::new().name("notify".into()).spawn(move || {
        let _ = autopilot_notify::send(&guardian, &title, &text);
    });
}

impl AgentChoice {
    #[must_use]
    pub fn read(data_dir: &Path) -> Self {
        match std::fs::read_to_string(data_dir.join(AGENT_FILE)) {
            Ok(text) if text.trim() == "claude" => Self::Claude,
            Ok(text) if text.trim() == "practice" => Self::Practice,
            _ => Self::Claude,
        }
    }

    pub fn save(self, data_dir: &Path) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(
            data_dir.join(AGENT_FILE),
            if self == Self::Claude {
                "claude"
            } else {
                "practice"
            },
        );
    }
}

/// The agent a window action runs with, built from the saved choice.
enum Chosen {
    Practice(FixtureAgent),
    Claude(Box<autopilot_agent_claude::ClaudeAgent>),
}

impl autopilot_agent::Agent for Chosen {
    fn start(
        &self,
        request: autopilot_agent::StepRequest<'_>,
    ) -> Result<autopilot_agent::AgentRun, autopilot_agent::AgentError> {
        match self {
            Self::Practice(agent) => agent.start(request),
            Self::Claude(agent) => agent.start(request),
        }
    }

    fn start_plan(
        &self,
        request: autopilot_agent::StepRequest<'_>,
    ) -> Result<autopilot_agent::AgentRun, autopilot_agent::AgentError> {
        match self {
            Self::Practice(agent) => agent.start_plan(request),
            Self::Claude(agent) => agent.start_plan(request),
        }
    }

    fn start_design(
        &self,
        request: autopilot_agent::StepRequest<'_>,
    ) -> Result<autopilot_agent::AgentRun, autopilot_agent::AgentError> {
        match self {
            Self::Practice(agent) => agent.start_design(request),
            Self::Claude(agent) => agent.start_design(request),
        }
    }
}

/// A helper binary that sits next to the window, or one folder up when the
/// window runs from `target/debug/deps` in a test.
fn beside(name: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let near = exe.with_file_name(&file);
    if near.exists() {
        return Ok(near);
    }
    let up = exe
        .parent()
        .and_then(Path::parent)
        .map(|folder| folder.join(&file));
    up.filter(|path| path.exists())
        .ok_or_else(|| format!("{file} is missing next to the window"))
}

/// The Claude settings for a step: the profile's model, effort, turn cap,
/// time limit and auth for it, and the shipped defaults for what it leaves out.
/// The settings a step runs with: the profile's, then the ticket's own
/// overrides (the owner's changes and the orchestrator's choices) on top.
fn claude_config(
    options: &Options,
    step: Step,
    ticket: Option<&TicketId>,
) -> autopilot_agent_claude::ClaudeConfig {
    let mut config = profile_config(options, step);
    if let Some(ticket) = ticket
        && let Ok(chosen) = autopilot_engine::effective_overrides(&options.data_dir, ticket, step)
    {
        if let Some(model) = chosen.model {
            config.model = model;
        }
        if chosen.effort.is_some() {
            config.effort = chosen.effort;
        }
        if let Some(turns) = chosen.turns {
            // Structured output needs a turn after the answer.
            config.max_turns = turns.max(2);
        }
    }
    config
}

fn profile_config(options: &Options, step: Step) -> autopilot_agent_claude::ClaudeConfig {
    let settings = autopilot_engine::load_profile(&options.profile)
        .ok()
        .and_then(|profile| profile.step(step).cloned());
    let Some(settings) = settings else {
        return autopilot_agent_claude::ClaudeConfig {
            auth: autopilot_profile::Auth::Subscription,
            model: "haiku".to_owned(),
            effort: None,
            max_turns: 4,
            max_budget_usd: None,
            executable: None,
            timeout: Duration::from_secs(300),
        };
    };
    let api = settings.auth == autopilot_profile::Auth::Api;
    autopilot_agent_claude::ClaudeConfig {
        auth: settings.auth,
        model: settings.model.unwrap_or_else(|| "haiku".to_owned()),
        effort: settings.effort,
        // Structured output needs a turn after the answer.
        max_turns: settings.limits.turns.max(2),
        // Only an API run has a per-call dollar cap; a subscription has none.
        max_budget_usd: api.then(|| {
            let micro = settings
                .limits
                .budget_microusd
                .map_or(50_000, autopilot_core::Money::as_microusd);
            format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000)
        }),
        executable: None,
        timeout: Duration::from_secs(u64::from(settings.limits.wall_seconds.max(30))),
    }
}

/// The project folder recorded on a ticket, if it still exists.
fn ticket_folder(options: &Options, ticket: &TicketId) -> Option<PathBuf> {
    let history = autopilot_engine::watch(&options.data_dir, ticket, 0).ok()?;
    autopilot_engine::workspace_of(&history)
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

const WORKSPACE_FILE: &str = "ui-workspace.txt";

/// The project folder last chosen in the message box.
#[must_use]
pub fn saved_workspace(data_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(data_dir.join(WORKSPACE_FILE)).ok()?;
    let path = PathBuf::from(text.trim());
    path.is_dir().then_some(path)
}

pub fn save_workspace(data_dir: &Path, folder: Option<&Path>) {
    let _ = std::fs::create_dir_all(data_dir);
    match folder {
        Some(path) => {
            let _ = std::fs::write(
                data_dir.join(WORKSPACE_FILE),
                path.to_string_lossy().as_bytes(),
            );
        }
        None => {
            let _ = std::fs::remove_file(data_dir.join(WORKSPACE_FILE));
        }
    }
}

fn chosen_agent(
    options: &Options,
    step: Step,
    ticket: Option<&TicketId>,
    workspace: Option<&Path>,
) -> Result<Chosen, String> {
    match AgentChoice::read(&options.data_dir) {
        AgentChoice::Practice => fixture_agent().map(Chosen::Practice),
        AgentChoice::Claude => {
            let config = claude_config(options, step, ticket);
            claude_agent(config, workspace).map(|agent| Chosen::Claude(Box::new(agent)))
        }
    }
}

fn claude_agent(
    config: autopilot_agent_claude::ClaudeConfig,
    workspace: Option<&Path>,
) -> Result<autopilot_agent_claude::ClaudeAgent, String> {
    let agent =
        autopilot_agent_claude::ClaudeAgent::new(config, beside("autopilot-process-guardian")?)
            .map_err(|error| error.to_string())?;
    agent
        .preflight()
        .map_err(|error| format!("Claude Code is not ready: {error}"))?;
    Ok(match workspace {
        Some(folder) => agent.in_workspace(folder.to_path_buf()),
        None => agent,
    })
}

/// Whether an orchestrator chooses models and effort for later steps, and
/// which model it uses. A capable model is the point: its one call is small
/// next to what a wrong choice for a step would cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Orchestrator {
    Off,
    Sonnet,
    Opus,
}

const ORCHESTRATOR_FILE: &str = "ui-orchestrator.txt";

impl Orchestrator {
    #[must_use]
    pub fn read(data_dir: &Path) -> Self {
        match std::fs::read_to_string(data_dir.join(ORCHESTRATOR_FILE))
            .unwrap_or_default()
            .trim()
        {
            "off" => Self::Off,
            "sonnet" => Self::Sonnet,
            _ => Self::Opus,
        }
    }

    pub fn save(self, data_dir: &Path) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(
            data_dir.join(ORCHESTRATOR_FILE),
            self.model().unwrap_or("off"),
        );
    }

    #[must_use]
    pub const fn model(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Sonnet => Some("sonnet"),
            Self::Opus => Some("opus"),
        }
    }
}

/// Ask the orchestrator to choose settings for the ticket's later steps, once.
fn orchestrate(options: &Options, ticket: &TicketId) -> Result<String, String> {
    if AgentChoice::read(&options.data_dir) != AgentChoice::Claude {
        return Err("the orchestrator needs the Claude agent".to_owned());
    }
    let model = Orchestrator::read(&options.data_dir)
        .model()
        .ok_or_else(|| "the orchestrator is switched off in Settings".to_owned())?;
    let folder = ticket_folder(options, ticket);
    let mut config = profile_config(options, Step::Understand);
    model.clone_into(&mut config.model);
    let agent = claude_agent(config, folder.as_deref())?;
    autopilot_engine::triage_ticket(&options.data_dir, ticket, folder.is_some(), |prompt| {
        agent.orchestrate(prompt).map_err(|error| error.to_string())
    })
    .map(|triage| {
        let chosen: Vec<String> = triage
            .steps
            .iter()
            .map(|advice| {
                format!(
                    "{} {}/{}",
                    autopilot_engine::step_name(advice.step).to_lowercase(),
                    advice.model,
                    advice.effort
                )
            })
            .collect();
        format!(
            "Orchestrator: {:?} complexity; {}",
            triage.complexity,
            chosen.join(", ")
        )
    })
    .map_err(|error| error.to_string())
}

/// Run the orchestrator before a step when it is on and has not decided yet.
/// Its failure never stops the step: the profile's settings stay in force.
fn orchestrated_first(options: &Options, ticket: &TicketId) {
    let wanted = AgentChoice::read(&options.data_dir) == AgentChoice::Claude
        && Orchestrator::read(&options.data_dir) != Orchestrator::Off;
    let decided = autopilot_engine::watch(&options.data_dir, ticket, 0)
        .is_ok_and(|history| autopilot_engine::triage_kept(&history).is_some());
    if wanted && !decided {
        let _ = orchestrate(options, ticket);
    }
}

fn fixture_agent() -> Result<FixtureAgent, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let first = FixtureAgent::beside(&executable);
    if let Ok(agent) = first {
        return Ok(agent);
    }
    let up = executable
        .parent()
        .and_then(Path::parent)
        .zip(executable.file_name())
        .map(|(folder, name)| folder.join(name));
    if let Some(agent) = up.and_then(|path| FixtureAgent::beside(&path).ok()) {
        return Ok(agent);
    }
    Err("the fixture agent is missing. Build the workspace first: cargo build --workspace".into())
}

/// Run an action on its own thread and report the outcome.
pub fn run(options: Options, action: Action, context: egui::Context, sender: Sender<Message>) {
    thread::spawn(move || {
        let outcome = perform(&options, &action);
        let _ = sender.send(Message::Done(outcome));
        context.request_repaint();
    });
}

/// Do one action, now, on this thread.
///
/// # Errors
/// Returns the reason in plain words.
#[allow(
    clippy::too_many_lines,
    reason = "one flat match over every window action"
)]
pub fn perform(options: &Options, action: &Action) -> Result<String, String> {
    let directory = options.data_dir.as_path();
    match action {
        Action::Start {
            title,
            description,
            attachments,
            workspace,
        } => start(
            options,
            title,
            description,
            attachments,
            workspace.as_deref(),
        ),
        Action::Attach { ticket, paths } => autopilot_engine::attach(directory, ticket, paths)
            .map(|count| format!("Attached {count}"))
            .map_err(|error| error.to_string()),
        Action::Decide {
            ticket,
            approve,
            always,
        } => {
            answer(directory, ticket, |body| match body {
                RequestBody::Approval { forced, .. } => Ok(Reply::Approval {
                    decision: if *approve {
                        Decision::Approve
                    } else {
                        Decision::Reject
                    },
                    allow_for_project: *always && *approve && !*forced,
                }),
                RequestBody::Question { .. } => {
                    Err("that is a question: answer it instead of approving".to_owned())
                }
            })?;
            Ok(if *approve { "Approved" } else { "Rejected" }.to_owned())
        }
        Action::AcceptDefaults { ticket } => {
            answer(directory, ticket, |body| match body {
                RequestBody::Question { questions } => Ok(Reply::Question {
                    choices: vec![Choice::Recommended; questions.len()],
                }),
                RequestBody::Approval { .. } => {
                    Err("that is an approval: approve or reject it".to_owned())
                }
            })?;
            Ok("Used the recommended answers".to_owned())
        }
        Action::RerunUnderstandWithMoreTurns(ticket) => {
            raise_turns(options, ticket)?;
            perform(options, &Action::RerunUnderstand(ticket.clone()))
        }
        Action::RerunUnderstand(ticket) => {
            let folder = ticket_folder(options, ticket);
            let agent = chosen_agent(options, Step::Understand, Some(ticket), folder.as_deref())?;
            let ask = autopilot_engine::load_profile(&options.profile)
                .ok()
                .and_then(|profile| {
                    profile
                        .step(Step::Understand)
                        .map(|settings| settings.ask_after)
                });
            autopilot_engine::rerun_understand(directory, ticket, &agent, ask)
                .map(|_| "Understand ran again".to_owned())
                .map_err(|error| error.to_string())
        }
        Action::MakePlan(ticket) => {
            orchestrated_first(options, ticket);
            let folder = ticket_folder(options, ticket);
            let agent = chosen_agent(options, Step::Plan, Some(ticket), folder.as_deref())?;
            autopilot_engine::plan_ticket(directory, ticket, &agent)
                .map(|plan| format!("Plan ready: {} criteria", plan.criteria.len()))
                .map_err(|error| error.to_string())
        }
        Action::Explain { ticket, step } => {
            let chosen = chosen_agent(options, Step::Understand, Some(ticket), None)?;
            autopilot_engine::explain_step(directory, ticket, *step, |prompt| match &chosen {
                Chosen::Claude(agent) => agent.explain(prompt).map_err(|error| error.to_string()),
                Chosen::Practice(_) => Ok(
                    "Practice agent: this is where a plain-words explanation from Claude appears. \
                     Switch the agent to Claude to get a real one."
                        .to_owned(),
                ),
            })
            .map(|_| "Explanation kept".to_owned())
            .map_err(|error| error.to_string())
        }
        Action::MakeDesign(ticket) => {
            orchestrated_first(options, ticket);
            let folder = ticket_folder(options, ticket);
            let agent = chosen_agent(options, Step::Design, Some(ticket), folder.as_deref())?;
            let history =
                autopilot_engine::watch(directory, ticket, 0).map_err(|error| error.to_string())?;
            let words = autopilot_engine::ticket_words(ticket, &history)
                .map_err(|error| error.to_string())?;
            autopilot_engine::run_design(directory, &words, &agent)
                .map(|design| match design {
                    Some(design) => format!("Design ready: {} mockups", design.mockups.len()),
                    None => "No design needed: the step was skipped".to_owned(),
                })
                .map_err(|error| error.to_string())
        }
        Action::RerunDesign(ticket) => {
            let folder = ticket_folder(options, ticket);
            let agent = chosen_agent(options, Step::Design, Some(ticket), folder.as_deref())?;
            autopilot_engine::rerun_design(directory, ticket, &agent)
                .map(|_| "New design ready".to_owned())
                .map_err(|error| error.to_string())
        }
        Action::OpenDesign(ticket) => {
            let folder = autopilot_engine::design_folder(directory, ticket);
            if !folder.is_dir() {
                return Err("there is no saved design for this ticket yet".to_owned());
            }
            let guardian = beside("autopilot-process-guardian")?;
            autopilot_update::open_installer(&guardian, &folder)?;
            Ok("Opened the mockups".to_owned())
        }
        Action::Orchestrate(ticket) => orchestrate(options, ticket),
        Action::RerunPlan(ticket) => {
            let folder = ticket_folder(options, ticket);
            let agent = chosen_agent(options, Step::Plan, Some(ticket), folder.as_deref())?;
            autopilot_engine::rerun_plan(directory, ticket, &agent)
                .map(|plan| format!("New plan ready: {} criteria", plan.criteria.len()))
                .map_err(|error| error.to_string())
        }
        Action::Pause(ticket) => autopilot_engine::request_pause(directory, ticket)
            .map(|()| "Pause requested. It takes effect between turns.".to_owned())
            .map_err(|error| error.to_string()),
        Action::Resume(ticket) => autopilot_engine::request_resume(directory, ticket)
            .map(|()| "Resume requested".to_owned())
            .map_err(|error| error.to_string()),
        Action::Stop(ticket) => autopilot_engine::request_stop(directory, ticket)
            .map(|()| "Stop requested. It takes effect between turns.".to_owned())
            .map_err(|error| error.to_string()),
    }
}

fn start(
    options: &Options,
    title: &str,
    description: &str,
    attachments: &[PathBuf],
    workspace: Option<&Path>,
) -> Result<String, String> {
    let directory = options.data_dir.as_path();
    if let Some(folder) = workspace
        && !folder.is_dir()
    {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let agent = chosen_agent(options, Step::Understand, None, workspace)?;
    let id = TicketId::from(format!("t-{}", now_ms()));
    let ticket = Ticket {
        id: id.clone(),
        title: title.trim().to_owned(),
        description: description.trim().to_owned(),
    };
    let ask = autopilot_engine::load_profile(&options.profile)
        .ok()
        .and_then(|profile| {
            profile
                .step(Step::Understand)
                .map(|settings| settings.ask_after)
        });
    let staged = autopilot_engine::stage(directory, &id, attachments, (0, 0))
        .map_err(|error| error.to_string())?;
    if let Err(error) =
        autopilot_engine::run_understand_attached(directory, &ticket, &agent, ask, &staged)
    {
        // The ticket was not made, so its copies must not stay behind.
        let _ = std::fs::remove_dir_all(directory.join("attachments").join(id.as_str()));
        return Err(error.to_string());
    }
    if let Some(folder) = workspace {
        // Kept in the history so every later step looks at the same project.
        autopilot_engine::record_workspace(directory, &id, folder)
            .map_err(|error| format!("the ticket started, but its folder was not kept: {error}"))?;
    }
    // Clarify is decided right away. Plan and the later steps need a real
    // agent, which the window does not start yet.
    // The orchestrator is not run here: it takes a capable model's time, and the
    // window starts it as its own action so the ticket shows up first.
    match autopilot_engine::start_clarify(directory, &id) {
        Ok(_) => Ok(format!("Started \"{}\"", ticket.title)),
        Err(error) => Ok(format!(
            "Started \"{}\". Clarify has not started yet: {error}",
            ticket.title
        )),
    }
}

fn answer(
    directory: &Path,
    ticket: &TicketId,
    reply: impl FnOnce(&RequestBody) -> Result<Reply, String>,
) -> Result<(), String> {
    let inbox = Inbox::in_data_dir(directory);
    let waiting = inbox
        .requests()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|request| &request.ticket == ticket)
        .ok_or_else(|| "nothing is waiting on this ticket".to_owned())?;
    let reply = reply(&waiting.body)?;
    check_reply(&waiting, &reply).map_err(|error| error.to_string())?;
    inbox
        .write_answer(&waiting.id, &reply)
        .map_err(|error| error.to_string())?;
    autopilot_engine::sync_inbox(directory, ticket).map_err(|error| error.to_string())?;
    Ok(())
}

/// Plain words for the request a ticket is waiting on.
#[must_use]
pub fn waiting_text(options: &Options, request: &RequestId) -> Option<String> {
    let inbox = Inbox::in_data_dir(&options.data_dir);
    let request = inbox.read_request(request).ok()??;
    Some(match request.body {
        RequestBody::Approval { action, forced } => {
            if forced {
                format!("{action} (this confirmation cannot be switched off)")
            } else {
                action
            }
        }
        RequestBody::Question { questions } => questions
            .iter()
            .map(|question| question.question.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

/// The most turns a retry may ask for, the same top value the settings offer.
const MAX_RETRY_TURNS: u32 = 200;

/// Double this ticket's turn cap for Understand, up to the limit, and record
/// it on the ticket alone. The project setting is left as it is.
pub(crate) fn raise_turns(options: &Options, ticket: &TicketId) -> Result<(), String> {
    let profile = autopilot_engine::load_profile(&options.profile).map_err(|e| e.to_string())?;
    let configured = profile
        .step(Step::Understand)
        .map_or(4, |settings| settings.limits.turns);
    let current =
        autopilot_engine::effective_overrides(&options.data_dir, ticket, Step::Understand)
            .map_err(|error| error.to_string())?
            .turns
            .unwrap_or(configured);
    let raised = current.saturating_mul(2).min(MAX_RETRY_TURNS);
    if raised <= current {
        return Err(format!(
            "the turn cap is already {current}, the most allowed"
        ));
    }
    let proposal = autopilot_engine::Proposal {
        step: Step::Understand,
        field: autopilot_engine::StepField::Turns(raised),
        before: current.to_string(),
        after: raised.to_string(),
    };
    autopilot_engine::apply_to_ticket(&options.data_dir, ticket, &[proposal])
        .map_err(|error| error.to_string())
}
