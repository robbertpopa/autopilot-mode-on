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
    },
    /// Approve, or reject, what a ticket is waiting on.
    Decide {
        ticket: TicketId,
        approve: bool,
    },
    /// Answer a waiting question with the recommended options.
    AcceptDefaults {
        ticket: TicketId,
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
        let directory = std::env::temp_dir().join("pipeline-updates");
        let outcome = autopilot_update::download(&guardian, &update, &directory)
            .and_then(|file| autopilot_update::open_installer(&guardian, &file));
        let _ = sender.send(Message::Installing(outcome));
        context.request_repaint();
    });
}

/// The fixture agent sits beside this program, or one folder up when the
/// program is a test binary in `deps`.
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
pub fn perform(options: &Options, action: &Action) -> Result<String, String> {
    let directory = options.data_dir.as_path();
    match action {
        Action::Start { title, description } => start(options, title, description),
        Action::Decide { ticket, approve } => {
            answer(directory, ticket, |body| match body {
                RequestBody::Approval { .. } => Ok(Reply::Approval {
                    decision: if *approve {
                        Decision::Approve
                    } else {
                        Decision::Reject
                    },
                    allow_for_project: false,
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

fn start(options: &Options, title: &str, description: &str) -> Result<String, String> {
    let directory = options.data_dir.as_path();
    let agent = fixture_agent()?;
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
    autopilot_engine::run_understand_with(directory, &ticket, &agent, ask)
        .map_err(|error| error.to_string())?;
    // Clarify is decided right away. Plan and the later steps need a real
    // agent, which the window does not start yet.
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
