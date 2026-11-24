//! Local fixture actions shared by the CLI and future clients.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::Path,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use autopilot_agent::{Agent, AgentError, AgentEvent, AgentRun, StepRequest};
use autopilot_core::{
    AttemptId, Event, NewEvent, OpenQuestion, Plan, RecordedEvent, RequestId, Step, Ticket,
    TicketId, Understanding,
};
use autopilot_pipeline::{
    StepState,
    clarify::{self, Choice, ClarifyPlan, QaPair},
    fold,
    inbox::{Decision, Inbox, InboxError, Reply, Request, RequestBody, check_reply},
};
use autopilot_store::{SqliteStore, Store, StoreError};

mod advise;
mod allowances;
mod attachments;
mod build;
mod catalog;
mod change;
mod control;
mod custom;
mod design;
mod draft;
mod explain;
mod know;
mod metrics;
mod notes;
mod overlap;
mod pipelines;
mod places;
mod plan;
mod pr_rules;
mod pr_watch;
mod profile_edit;
mod proposals;
mod publish;
mod rerun;
mod review;
mod rules;
mod settings;
mod triage;
mod tuning;
mod view;
mod workspace;
pub use advise::{
    advise_ticket, calls as advisor_calls, chat_calls, chat_context, chat_of, chat_ticket,
    kept as advice_kept,
};
pub use allowances::{allow, allowed, list as allowances, revoke};
pub use attachments::{
    Attachment, MAX_ATTACHMENTS, MAX_FILE_BYTES, MAX_FOLDER_ITEMS, MAX_TICKET_BYTES, attach, note,
    stage,
};
pub use autopilot_checks::resolve_executable;
pub use autopilot_tracker::Repo;
pub use build::{BuildOutcome, BuildSetup, Isolation, run_build};
pub use catalog::{ToolSuggestion, allowed_tools, set_tool_allowed, suggest_tools};
pub use change::{Proposal, Reading, apply_to_project, apply_to_ticket, propose};
pub use control::{request_pause, request_resume, request_stop};
pub use custom::{latest as custom_outcome, run_custom};
pub use design::{
    context as design_context, folder as design_folder, latest as latest_design, rerun_design,
    run_design,
};
pub use draft::{
    PipelineDraft, draft_pipeline, draft_pipeline_for, save_draft, validate_draft,
    validate_draft_for,
};
pub use explain::{explain_step, kept as explanation_kept, prompt as explain_prompt};
pub use know::{
    CardInfo, DetectedCommand, MAX_CARD_BYTES, ProjectCard, card_changes, card_path, get_to_know,
    read_card, render_card, scan_workspace,
};
pub use metrics::{PipelineMetrics, StepMetrics, UsageLine, metrics_of, pipeline_metrics};
pub use overlap::{TicketOverlap, find_overlaps};
pub use pipelines::{
    choose_pipeline, chosen as chosen_pipeline, chosen_custom as chosen_custom_steps,
};
pub use places::{Places, gh_for, places, worktree_of};
pub use plan::{approved_plan, plan_ticket, run_plan};
pub use pr_rules::{
    ChecksState, MergeState, PrAction, PrState, PrSummary, ReviewDecision, next_action,
};
pub use pr_watch::{PrWatch, watch_pull_request};
pub use profile_edit::{
    ProfileEditError, ProjectField, ReviewField, StepField, add_detected_check, add_pipeline,
    edit_project, edit_review, edit_step, free_custom_slot, load_profile, remove_custom_step,
    save_pipeline, set_custom_step,
};
pub use proposals::{Suggestion, dismiss_proposal, proposals_for};
pub use publish::{PublishSetup, ci_status, prepare_publish, run_publish};
pub use rerun::ticket_words;
pub use rerun::{rerun_plan, rerun_understand, reset_step};
pub use review::{Finding, ReviewOutcome, Rule, findings_for, open_findings, run_review};
pub use rules::{
    MAX_RULES_BYTES, RulesInfo, check_rules, project_key, read_rules, rules_info, rules_path,
    save_rules,
};
pub use settings::{
    effective_overrides, effective_when_unsure, override_settings, project_overrides,
    set_when_unsure,
};
pub use triage::{EFFORTS, MODELS, kept as triage_kept, prompt as triage_prompt, triage_ticket};
pub use tuning::{
    ActiveTuning, Tuning, TuningOutcome, active_tunings, run_tuning, tune_directory, tuned_effort,
    undo_tuning,
};
pub use view::{
    AttachmentView, Badge, CheckFinding, CheckRow, CostLine, LogLine, RoundView, StepView,
    TicketView, TimelineLine, dollars, load_ticket, name_custom_steps, spent_since, step_name,
    ticket_view, tickets,
};
pub use workspace::{
    canonical as canonical_folder, of as workspace_of, record as record_workspace,
};

pub(crate) const DATABASE: &str = "events.sqlite3";
const UNDERSTAND: Option<Step> = Some(Step::Understand);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ticket ID and title must be non-empty")]
    InvalidTicket,
    #[error("an action is already running in this data directory")]
    Busy,
    #[error("this ticket already has a run in this data directory")]
    AlreadyStarted,
    #[error("clarify cannot run now: {0}")]
    NotReady(String),
    #[error(transparent)]
    Answer(#[from] clarify::ClarifyError),
    #[error(transparent)]
    Inbox(#[from] InboxError),
    #[error("cannot explain: {0}")]
    Explain(String),
    #[error("the orchestrator could not decide: {0}")]
    Triage(String),
    #[error("cannot attach: {0}")]
    Attachment(String),
    #[error("the pipeline draft was refused: {0}")]
    Draft(String),
    #[error("the rules file cannot be used: {0}")]
    Rules(String),
    #[error("the project card cannot be used: {0}")]
    Card(String),
    #[error("the recorded history is invalid: {0}")]
    History(String),
    #[error("ticket has no recorded run")]
    UnknownTicket,
    #[error("the current time is outside the supported range")]
    Clock,
    #[error("agent execution failed: {0}")]
    Agent(#[source] AgentError),
    #[error("the agent stopped at a limit: {0}")]
    Stopped(String),
    #[error(transparent)]
    Storage(#[from] StoreError),
    #[error(transparent)]
    Git(#[from] autopilot_git::Error),
    #[error(transparent)]
    Check(#[from] autopilot_checks::Error),
    #[error(transparent)]
    Tracker(#[from] autopilot_tracker::Error),
    #[error(transparent)]
    PushRefused(#[from] autopilot_policy::PushRefusal),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Run and durably record one Understand attempt.
///
/// # Errors
/// Returns an error if the ticket, agent, data directory or store cannot be used.
pub fn run_understand(
    directory: &Path,
    ticket: &Ticket,
    agent: &impl Agent,
) -> Result<Understanding, Error> {
    run_understand_with(directory, ticket, agent, None)
}

/// Like [`run_understand`]. With `ask_after` (or the project setting when it is `None`) the step waits in the inbox for
/// the owner to approve, edit or reject its result before it counts as passed.
///
/// # Errors
/// Returns an error if the ticket, agent, data directory or store cannot be used.
pub fn run_understand_with(
    directory: &Path,
    ticket: &Ticket,
    agent: &impl Agent,
    ask_after: Option<bool>,
) -> Result<Understanding, Error> {
    run_understand_attached(directory, ticket, agent, ask_after, &[])
}

/// Like [`run_understand_with`], with attachments staged by [`stage`]. They
/// are recorded in the opening batch, so the first attempt already has them.
///
/// # Errors
/// Returns an error if the ticket, agent, data directory or store cannot be used.
pub fn run_understand_attached(
    directory: &Path,
    ticket: &Ticket,
    agent: &impl Agent,
    ask_after: Option<bool>,
    attachments: &[Attachment],
) -> Result<Understanding, Error> {
    run_understand_in(directory, ticket, agent, ask_after, attachments, None)
}

/// Like [`run_understand_attached`], for a ticket in a project folder. The
/// project's rules file reaches the prompt; the folder is recorded by the
/// caller afterwards, so the first run is told about it here.
///
/// # Errors
/// Returns an error if the ticket, agent, data directory or store cannot be used.
pub fn run_understand_in(
    directory: &Path,
    ticket: &Ticket,
    agent: &impl Agent,
    ask_after: Option<bool>,
    attachments: &[Attachment],
    workspace: Option<&Path>,
) -> Result<Understanding, Error> {
    // An explicit choice wins; otherwise the project setting decides.
    let ask_after = match ask_after {
        Some(choice) => choice,
        None => {
            project_overrides(directory, Step::Understand)?
                .ask_after
                .unwrap_or(false)
                && !allowances::allowed(directory, Step::Understand)
        }
    };
    if ticket.id.as_str().trim().is_empty() || ticket.title.trim().is_empty() {
        return Err(Error::InvalidTicket);
    }
    let _owner = lock_run(directory, &ticket.id)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let started_at = timestamp()?;
    let mut start_events = vec![
        event(
            started_at,
            None,
            Event::TicketCreated {
                title: ticket.title.clone(),
            },
        ),
        event(
            started_at,
            None,
            Event::Described {
                text: ticket.description.clone(),
            },
        ),
        event(
            started_at,
            UNDERSTAND,
            Event::StepStarted {
                attempt: AttemptId::from("understand-1"),
                session: None,
            },
        ),
    ];
    // Attachments sit between the ticket's words and the step's start.
    let at = start_events.len() - 1;
    for (offset, attachment) in attachments.iter().cloned().enumerate() {
        start_events.insert(
            at + offset,
            event(started_at, None, attachment.into_event()),
        );
    }
    let start = match store.append(&ticket.id, 0, &start_events) {
        Err(StoreError::SequenceConflict { .. }) => return Err(Error::AlreadyStarted),
        result => result?,
    };
    let last = start.last().map_or(0, |record| record.seq);

    let workspace = workspace.map(|path| {
        workspace::canonical(path).unwrap_or_else(|| path.to_string_lossy().into_owned())
    });
    run_attempt(
        directory,
        &mut store,
        ticket,
        last,
        ask_after,
        agent,
        workspace.as_deref(),
    )
}

/// Run the Understand agent for an attempt that is already recorded as
/// started, and record how it ended.
pub(crate) fn run_attempt(
    directory: &Path,
    store: &mut SqliteStore,
    ticket: &Ticket,
    last: u64,
    ask_after: bool,
    agent: &impl Agent,
    workspace: Option<&str>,
) -> Result<Understanding, Error> {
    let history = store.events(&ticket.id, 0)?;
    let mut notes = attachments::note(directory, &history);
    let project = workspace
        .map(str::to_owned)
        .or_else(|| workspace::of(&history));
    let last = rules::include_in_prompt(
        store,
        directory,
        &ticket.id,
        Step::Understand,
        last,
        project.as_deref(),
        &mut notes,
    )?;
    let started = agent.start(StepRequest {
        step: Step::Understand,
        ticket,
        context: &notes,
    });
    let (response, last) = match pump(store, &ticket.id, UNDERSTAND, last, started)? {
        (Pumped::Understanding(result), last) => (Ok(result), last),
        (Pumped::Plan(_) | Pumped::Design(_) | Pumped::Custom(_), last) => (
            Err(Outcome::Failed(
                io::Error::other("the agent answered Understand with a plan").into(),
            )),
            last,
        ),
        (Pumped::Stopped(reason), last) => (Err(Outcome::Stopped(reason)), last),
        (Pumped::Failed(error), last) => (Err(Outcome::Failed(error)), last),
    };
    let asked = response.is_ok() && ask_after;
    let approval = approval_id(&ticket.id, &history, Step::Understand);
    let result = conclude(store, &ticket.id, last, response, ask_after, &approval)?;
    if asked {
        Inbox::in_data_dir(directory).write_request(&approval_request(
            &ticket.id,
            &approval,
            Step::Understand,
            false,
        ))?;
    }
    Ok(result)
}

/// How a step's agent run ended.
enum Pumped {
    Understanding(Understanding),
    Plan(Plan),
    Design(autopilot_core::Design),
    Custom(autopilot_core::CustomOutcome),
    Stopped(String),
    Failed(AgentError),
}

/// Record the run's activity and cost as it arrives and return how it ended
/// with the new last sequence number.
fn pump(
    store: &mut SqliteStore,
    ticket: &TicketId,
    step: Option<Step>,
    mut last: u64,
    started: Result<AgentRun, AgentError>,
) -> Result<(Pumped, u64), Error> {
    let mut run = match started {
        Ok(run) => run,
        Err(error) => return Ok((Pumped::Failed(error), last)),
    };
    let pumped = loop {
        let kind = match run.next() {
            Some(AgentEvent::Started | AgentEvent::Session(_)) => continue,
            Some(AgentEvent::BuildDone) => {
                break Pumped::Failed(
                    io::Error::other("a step that expects a result got a build result").into(),
                );
            }
            Some(AgentEvent::Message { summary }) => Event::AgentActivity { summary },
            Some(AgentEvent::Usage(usage)) => Event::UsageReported { usage },
            Some(AgentEvent::Cost { amount, estimated }) => Event::CostTick { amount, estimated },
            Some(AgentEvent::Result(result)) => break Pumped::Understanding(result),
            Some(AgentEvent::DraftResult(_)) => {
                break Pumped::Failed(
                    io::Error::other("a step that expects a result got a pipeline draft").into(),
                );
            }
            Some(AgentEvent::ReviewResult(_)) => {
                break Pumped::Failed(
                    io::Error::other("a step that expects a result got review findings").into(),
                );
            }
            Some(AgentEvent::PlanResult(plan)) => break Pumped::Plan(plan),
            Some(AgentEvent::DesignResult(design)) => break Pumped::Design(design),
            Some(AgentEvent::CustomResult(outcome)) => break Pumped::Custom(outcome),
            Some(AgentEvent::Stopped(reason)) => break Pumped::Stopped(reason),
            Some(AgentEvent::Failed(error)) => break Pumped::Failed(error),
            None => {
                break Pumped::Failed(
                    io::Error::other("agent stream ended without a result").into(),
                );
            }
        };
        let committed = store.append(ticket, last, &[event(timestamp()?, step, kind)])?;
        last = committed.last().map_or(last, |record| record.seq);
    };
    Ok((pumped, last))
}

/// The id of the approval a step asks for. Answers are kept once per request
/// id, so a step that is sent back, rejected or run again asks under a new id
/// each time: the first is `plan-approval`, then `plan-approval-2`, and so on.
pub(crate) fn approval_id(ticket: &TicketId, history: &[RecordedEvent], step: Step) -> RequestId {
    let base = format!("{}-approval", step.as_str());
    let used = history
        .iter()
        .filter(|record| {
            matches!(&record.event.kind, Event::Answered { request, .. }
                if request.as_str().starts_with(&base))
        })
        .count();
    if used == 0 {
        scoped(ticket, &base)
    } else {
        scoped(ticket, &format!("{base}-{}", used + 1))
    }
}

/// Why a step failed, in words the owner can act on. A usage limit is said
/// out loud, with the time the agent gave, because "step failed" alone sends
/// the owner looking for a bug that is not there.
pub(crate) fn failure_reason(
    store: &impl Store,
    ticket: &TicketId,
    step: Option<Step>,
    label: &str,
    error: &dyn std::fmt::Display,
) -> String {
    let limit = store.events(ticket, 0).ok().and_then(|history| {
        history
            .iter()
            .rev()
            .find_map(|record| match &record.event.kind {
                Event::AgentActivity { summary }
                    if record.event.step == step
                        && summary.starts_with("assistant: ")
                        && summary.to_lowercase().contains("limit") =>
                {
                    Some(summary.trim_start_matches("assistant: ").to_owned())
                }
                _ => None,
            })
    });
    let why = limit.unwrap_or_else(|| error.to_string());
    format!(
        "{label} step failed: {}",
        why.chars().take(300).collect::<String>()
    )
}

/// A request id that belongs to one ticket. The inbox keeps one file per id,
/// so two tickets waiting on the same kind of question must not share one: the
/// tag keeps the files apart and a ticket always finds its own.
pub(crate) fn scoped(ticket: &TicketId, base: &str) -> RequestId {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(ticket.as_str().as_bytes());
    RequestId::from(format!("{base}-{}", attachments::hex(&digest[..4])))
}

fn plan_request(ticket: &TicketId, plan: &Plan, id: RequestId) -> Request {
    let analysis = autopilot_pipeline::analyze::analyze(plan);
    Request {
        id,
        ticket: ticket.clone(),
        body: RequestBody::Approval {
            action: format!("approve the plan ({})", analysis.describe()),
            // A plan that cannot be checked as written is a forced confirmation.
            forced: analysis.blocking(),
        },
    }
}

fn approval_request(ticket: &TicketId, id: &RequestId, step: Step, forced: bool) -> Request {
    Request {
        id: id.clone(),
        ticket: ticket.clone(),
        body: RequestBody::Approval {
            action: format!("continue after the {} step", step.as_str()),
            forced,
        },
    }
}

/// Record how the attempt ended.
fn conclude(
    store: &mut SqliteStore,
    ticket: &TicketId,
    last: u64,
    response: Result<Understanding, Outcome>,
    ask_after: bool,
    approval: &RequestId,
) -> Result<Understanding, Error> {
    let finished_at = timestamp()?;
    match response {
        Ok(result) => {
            let closing = if ask_after {
                Event::Asked {
                    request: approval.clone(),
                }
            } else {
                Event::StepFinished
            };
            let terminal = [
                event(
                    finished_at,
                    UNDERSTAND,
                    Event::UnderstandResult {
                        result: result.clone(),
                    },
                ),
                event(finished_at, UNDERSTAND, closing),
            ];
            store.append(ticket, last, &terminal)?;
            Ok(result)
        }
        Err(Outcome::Stopped(reason)) => {
            store.append(
                ticket,
                last,
                &[event(
                    finished_at,
                    UNDERSTAND,
                    Event::Interrupted {
                        reason: reason.clone(),
                    },
                )],
            )?;
            Err(Error::Stopped(reason))
        }
        Err(Outcome::Failed(error)) => {
            store.append(
                ticket,
                last,
                &[event(
                    finished_at,
                    UNDERSTAND,
                    Event::Failed {
                        reason: failure_reason(store, ticket, UNDERSTAND, "Understand", &error),
                    },
                )],
            )?;
            Err(Error::Agent(error))
        }
    }
}

/// A limit stop is kept apart from failure so callers do not retry it blindly.
enum Outcome {
    Stopped(String),
    Failed(AgentError),
}

/// What Clarify decided after Understand.
#[derive(Debug, PartialEq, Eq)]
pub enum ClarifyStart {
    Skipped(&'static str),
    Asking {
        request: RequestId,
        questions: Vec<OpenQuestion>,
    },
}

const CLARIFY: Option<Step> = Some(Step::Clarify);
const CLARIFY_REQUEST: &str = "clarify-1";

/// Skip Clarify, or start it and ask. The questions come from the recorded
/// Understand result, so a restarted process asks the same ones.
///
/// # Errors
/// Returns an error if Understand has not passed or Clarify already ran.
pub fn start_clarify(directory: &Path, ticket: &TicketId) -> Result<ClarifyStart, Error> {
    let _owner = lock_run(directory, ticket)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::Understand) != &StepState::Passed {
        return Err(Error::NotReady("Understand has not passed".into()));
    }
    if matches!(state.step(Step::Clarify), StepState::Skipped(reason) if reason.starts_with(pipelines::OFF_PREFIX))
    {
        // The pipeline turned it off; there is nothing to ask.
        return Ok(ClarifyStart::Skipped("off in this pipeline"));
    }
    if state.step(Step::Clarify) != &StepState::Idle {
        return Err(Error::NotReady("Clarify already started".into()));
    }
    let understanding = latest_understanding(&history)
        .ok_or_else(|| Error::History("no Understand result".into()))?;
    let at = timestamp()?;
    match clarify::plan(understanding) {
        ClarifyPlan::Skip(reason) => {
            let skip = Event::StepSkipped {
                reason: reason.into(),
            };
            store.append(ticket, last, &[event(at, CLARIFY, skip)])?;
            Ok(ClarifyStart::Skipped(reason))
        }
        ClarifyPlan::Ask(questions) => {
            let request = scoped(ticket, CLARIFY_REQUEST);
            let started = Event::StepStarted {
                attempt: AttemptId::from("clarify-1"),
                session: None,
            };
            let asked = Event::Asked {
                request: request.clone(),
            };
            store.append(
                ticket,
                last,
                &[event(at, CLARIFY, started), event(at, CLARIFY, asked)],
            )?;
            Inbox::in_data_dir(directory).write_request(&Request {
                id: request.clone(),
                ticket: ticket.clone(),
                body: RequestBody::Question {
                    questions: questions.clone(),
                },
            })?;
            Ok(ClarifyStart::Asking { request, questions })
        }
    }
}

/// Record the owner's answers and finish Clarify. The `Q → A` pairs are stored
/// in the answer event, so they are part of the replayable history.
///
/// # Errors
/// Returns an error if Clarify is not waiting on this request or the answers
/// do not match the questions.
pub fn answer_clarify(
    directory: &Path,
    ticket: &TicketId,
    request: &RequestId,
    choices: &[Choice],
) -> Result<Vec<QaPair>, Error> {
    let _owner = lock_run(directory, ticket)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let pairs = commit_clarify_answer(&mut store, ticket, &history, request, choices)?;
    Inbox::in_data_dir(directory).clean_up(request)?;
    Ok(pairs)
}

fn commit_clarify_answer(
    store: &mut SqliteStore,
    ticket: &TicketId,
    history: &[RecordedEvent],
    request: &RequestId,
    choices: &[Choice],
) -> Result<Vec<QaPair>, Error> {
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = fold(history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::Clarify) != &StepState::Asking(request.clone()) {
        return Err(Error::NotReady(
            "Clarify is not waiting on that request".into(),
        ));
    }
    let understanding = latest_understanding(history)
        .ok_or_else(|| Error::History("no Understand result".into()))?;
    let pairs = clarify::resolve(&understanding.open_questions, choices)?;
    let answer =
        serde_json::to_string(&pairs).map_err(|error| Error::History(error.to_string()))?;
    let at = timestamp()?;
    store.append(
        ticket,
        last,
        &[
            event(
                at,
                CLARIFY,
                Event::Answered {
                    request: request.clone(),
                    answer,
                },
            ),
            event(at, CLARIFY, Event::StepFinished),
        ],
    )?;
    Ok(pairs)
}

/// Mark an attempt left running by a process that no longer exists as
/// `Interrupted`. Holding the run lock proves no other process is working on
/// the ticket, so a step that still reads as running was cut off. Waiting and
/// paused steps are durable states and are left alone.
///
/// # Errors
/// Returns an error if another action holds the data directory or the history
/// is invalid.
pub fn recover(directory: &Path, ticket: &TicketId) -> Result<Vec<Step>, Error> {
    let _owner = lock_run(directory, ticket)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let mut last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    let mut recovered = Vec::new();
    for step in Step::EVERY {
        if state.step(step) != &StepState::Running {
            continue;
        }
        let interrupted = Event::Interrupted {
            reason: "the process ended before the step finished".into(),
        };
        let committed = store.append(
            ticket,
            last,
            &[event(timestamp()?, Some(step), interrupted)],
        )?;
        last = committed.last().map_or(last, |record| record.seq);
        recovered.push(step);
    }
    Ok(recovered)
}

/// Reconcile the inbox folder with the event log for one ticket.
///
/// A waiting question gets its request file back if it is missing. A valid
/// answer file is committed once, and only then are the files removed. Files
/// for requests that are no longer waiting (a crash after the commit) are
/// removed without being applied.
///
/// # Errors
/// Returns an error for an invalid history or an unusable answer file. An
/// invalid answer leaves its files in place for the owner to replace.
pub fn sync_inbox(directory: &Path, ticket: &TicketId) -> Result<Vec<RequestId>, Error> {
    let _owner = lock_run(directory, ticket)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let inbox = Inbox::in_data_dir(directory);
    let history = store.events(ticket, 0)?;
    if history.is_empty() {
        return Err(Error::UnknownTicket);
    }
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    let mut applied = Vec::new();
    for step in Step::EVERY {
        let StepState::Asking(request) = state.step(step) else {
            continue;
        };
        let mirror = waiting_request(&history, ticket, step, request)?;
        if inbox.read_request(request)?.is_none() {
            inbox.write_request(&mirror)?;
        }
        let Some(reply) = inbox.read_answer(request)? else {
            continue;
        };
        check_reply(&mirror, &reply)?;
        match reply {
            Reply::Question { choices } => {
                commit_clarify_answer(&mut store, ticket, &history, request, &choices)?;
            }
            Reply::Approval {
                decision,
                allow_for_project,
            } => {
                // A forced confirmation was already refused by `check_reply`.
                if allow_for_project && !matches!(decision, Decision::Reject) && step != Step::Plan
                {
                    allowances::allow(directory, step)?;
                }
                commit_approval(
                    &mut store, directory, ticket, &history, step, request, &decision,
                )?;
            }
        }
        inbox.clean_up(request)?;
        applied.push(request.clone());
        // One answer per pass: the history changed, so the caller syncs again.
        break;
    }
    let history = store.events(ticket, 0)?;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    for request in inbox.requests()? {
        let waiting = Step::EVERY
            .iter()
            .any(|step| state.step(*step) == &StepState::Asking(request.id.clone()));
        if request.ticket == *ticket && !waiting {
            inbox.clean_up(&request.id)?;
        }
    }
    // The request file is removed first, so a crash can leave a lone answer
    // file. It is only safe to delete when the log proves it was applied.
    for id in inbox.answer_ids()? {
        let committed = history.iter().any(|record| {
            matches!(&record.event.kind, Event::Answered { request, .. } if *request == id)
        });
        if committed {
            inbox.clean_up(&id)?;
        }
    }
    Ok(applied)
}

/// The request a waiting step should show, rebuilt from the log. A lost
/// approval request comes back as forced: that is the safe reading, because a
/// forced confirmation cannot be turned into a standing allowance.
fn waiting_request(
    history: &[RecordedEvent],
    ticket: &TicketId,
    step: Step,
    request: &RequestId,
) -> Result<Request, Error> {
    if step == Step::Clarify && request.as_str().starts_with(CLARIFY_REQUEST) {
        let understanding = latest_understanding(history)
            .ok_or_else(|| Error::History("no Understand result".into()))?;
        return Ok(Request {
            id: request.clone(),
            ticket: ticket.clone(),
            body: RequestBody::Question {
                questions: understanding.open_questions.clone(),
            },
        });
    }
    if step == Step::BuildCheck && request.as_str().starts_with(build::STUCK_REQUEST) {
        return Ok(build::stuck_request(ticket, history));
    }
    if step == Step::PullRequest && request.as_str().starts_with(publish::PUBLISH_REQUEST) {
        return publish::request(ticket, history);
    }
    if step == Step::Design
        && let Some(design) = design::latest(history)
    {
        return Ok(design::request(ticket, design));
    }
    if step == Step::Plan {
        let plan =
            plan::latest_plan(history).ok_or_else(|| Error::History("no Plan result".into()))?;
        return Ok(plan_request(ticket, plan, request.clone()));
    }
    Ok(approval_request(ticket, request, step, true))
}

/// Approve and edit let the step pass; the edit text stays in the log with the
/// reply. Reject stops the step as interrupted so it can be run again.
fn commit_approval(
    store: &mut SqliteStore,
    directory: &Path,
    ticket: &TicketId,
    history: &[RecordedEvent],
    step: Step,
    request: &RequestId,
    decision: &Decision,
) -> Result<(), Error> {
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let tidy = match decision {
        Decision::SendBack(note) => match notes::clean_note(note) {
            text if text.is_empty() => Decision::Reject,
            text => Decision::SendBack(text),
        },
        Decision::ApproveCorrected(fixes) => Decision::ApproveCorrected(
            fixes
                .iter()
                .map(|fix| autopilot_core::AssumptionCorrection {
                    assumption: notes::clean_note(&fix.assumption),
                    correction: notes::clean_note(&fix.correction),
                })
                .filter(|fix| !fix.correction.is_empty())
                .collect(),
        ),
        other => other.clone(),
    };
    let decision = &tidy;
    let answer =
        serde_json::to_string(decision).map_err(|error| Error::History(error.to_string()))?;
    let at = timestamp()?;
    let rejected = matches!(decision, Decision::Reject | Decision::SendBack(_));
    let mut events = vec![event(
        at,
        Some(step),
        Event::Answered {
            request: request.clone(),
            answer,
        },
    )];
    if rejected {
        let reason = if matches!(decision, Decision::SendBack(_)) {
            "sent back with notes"
        } else {
            "rejected by the owner"
        };
        let reason = reason.into();
        events.push(event(at, Some(step), Event::Interrupted { reason }));
    } else {
        if let Decision::ApproveCorrected(corrections) = decision {
            events.push(event(
                at,
                Some(step),
                Event::AssumptionsCorrected {
                    corrections: corrections.clone(),
                },
            ));
        }
        if step == Step::Plan {
            let sha256 = plan::store_approved(directory, ticket, history)?;
            events.push(event(at, Some(step), Event::PlanApproved { sha256 }));
        }
        // A stuck build is not finished by an approval. The answer sends it
        // back to Running, and the next build run continues with more rounds.
        if !matches!(step, Step::BuildCheck | Step::PullRequest) {
            events.push(event(at, Some(step), Event::StepFinished));
        }
    }
    store.append(ticket, last, &events)?;
    Ok(())
}

fn latest_understanding(history: &[RecordedEvent]) -> Option<&Understanding> {
    history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::UnderstandResult { result } => Some(result),
            _ => None,
        })
}

/// Replay committed events after a cursor through a read-only connection.
///
/// # Errors
/// Returns an error for an unknown ticket or an unreadable database.
pub fn watch(directory: &Path, ticket: &TicketId, after: u64) -> Result<Vec<RecordedEvent>, Error> {
    let store = SqliteStore::open_read_only(directory.join(DATABASE))?;
    let events = store.events(ticket, after)?;
    if events.is_empty() && (after == 0 || store.events(ticket, 0)?.is_empty()) {
        return Err(Error::UnknownTicket);
    }
    Ok(events)
}

/// Replay a ticket's history after `after`, then keep delivering new events
/// as they are committed. This is what a window uses to open late and still
/// show everything, then update on events only.
///
/// `on_event` is called once per event, in order. Return `false` from it to
/// stop. The call also ends after the `Done` event has been delivered.
///
/// # Errors
/// Returns an error if the ticket has no history or the store cannot be read.
pub fn follow(
    directory: &Path,
    ticket: &TicketId,
    after: u64,
    poll: Duration,
    mut on_event: impl FnMut(&RecordedEvent) -> bool,
) -> Result<(), Error> {
    let mut seen = after;
    let mut first = true;
    loop {
        // A new handle each time, so a writer in another process is seen.
        let events = if first {
            watch(directory, ticket, seen)?
        } else {
            SqliteStore::open_read_only(directory.join(DATABASE))?.events(ticket, seen)?
        };
        first = false;
        for record in &events {
            seen = record.seq;
            if !on_event(record) || matches!(record.event.kind, Event::Done) {
                return Ok(());
            }
        }
        if events.is_empty() {
            thread::sleep(poll);
        }
    }
}

/// One lock per ticket, so tickets run side by side while one ticket never has
/// two actions at once. The name is hashed: an id may hold characters a file
/// name cannot.
fn lock_run(directory: &Path, ticket: &TicketId) -> Result<File, Error> {
    use sha2::{Digest, Sha256};
    fs::create_dir_all(directory)?;
    let digest = Sha256::digest(ticket.as_str().as_bytes());
    let name = format!("run-{}.lock", attachments::hex(&digest[..8]));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(name))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(Error::Busy),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

pub(crate) fn timestamp() -> Result<u64, Error> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Clock)?;
    u64::try_from(duration.as_millis()).map_err(|_| Error::Clock)
}

fn event(at_ms: u64, step: Option<Step>, kind: Event) -> NewEvent {
    NewEvent { at_ms, step, kind }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    // Invariant: two tickets can each hold their own lock at once, one ticket
    // cannot hold it twice. A lock per data folder would make tickets queue
    // behind each other; none at all would let one ticket run two actions.
    #[test]
    fn the_run_lock_is_per_ticket() {
        let data = tempfile::tempdir().unwrap();
        let one = TicketId::from("t-1");
        let two = TicketId::from("t-2");
        let held = lock_run(data.path(), &one).unwrap();
        assert!(matches!(lock_run(data.path(), &one), Err(Error::Busy)));
        let other = lock_run(data.path(), &two).unwrap();
        drop(held);
        assert!(lock_run(data.path(), &one).is_ok());
        drop(other);
    }

    // Invariant: the same kind of request from two tickets gets two ids, and the
    // id stays a safe file name. One shared id made two tickets waiting on a
    // design overwrite each other's inbox file, so one could not be approved.
    #[test]
    fn request_ids_are_per_ticket() {
        let one = scoped(&TicketId::from("t-1"), "design-approval");
        let two = scoped(&TicketId::from("t-2"), "design-approval");
        assert_ne!(one, two);
        assert!(one.as_str().starts_with("design-approval-"));
        assert!(one.as_str().len() <= 64);
    }

    // Invariant: a usage limit the agent reported is what the failure says; any
    // other failure keeps the error. A fixed "step failed" sent the owner
    // looking for a bug when the cause was the 5-hour limit.
    #[test]
    fn a_failed_step_says_when_the_cause_was_a_usage_limit() {
        let data = tempfile::tempdir().unwrap();
        let mut store = SqliteStore::open(data.path().join(DATABASE)).unwrap();
        let ticket = TicketId::from("t-9");
        let step = Some(Step::Design);
        let first = event(
            1,
            step,
            Event::AgentActivity {
                summary: "assistant: thinking".into(),
            },
        );
        let committed = store.append(&ticket, 0, &[first]).unwrap();
        let plain = failure_reason(&store, &ticket, step, "Design", &"the stream broke");
        assert_eq!(plain, "Design step failed: the stream broke");
        let limit = event(
            2,
            step,
            Event::AgentActivity {
                summary: "assistant: You've hit your 5-hour limit, resets 7:30pm".into(),
            },
        );
        store
            .append(&ticket, committed.last().unwrap().seq, &[limit])
            .unwrap();
        let said = failure_reason(&store, &ticket, step, "Design", &"the stream broke");
        assert!(said.contains("5-hour limit"), "{said}");
    }
}
