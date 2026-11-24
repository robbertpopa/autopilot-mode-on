//! The window: title bar, ticket sidebar and the three screens of the design.
//! It holds no logic of its own. It paints what `engine` reports and turns
//! clicks into engine actions.

use std::sync::mpsc::{self, Receiver, Sender};

use autopilot_core::{RequestId, Step, TicketId};
use autopilot_engine::{Badge, TicketView, dollars, load_profile};
use autopilot_pipeline::StepState;
use autopilot_profile::Profile;
use eframe::egui::{self, Color32, Frame, Margin, RichText};

use crate::backend::{self, Action, Message, Options, Snapshot};
use crate::composer::NewTicket;
use crate::theme;

/// The width of the one column the screens are laid out in.
const COLUMN: f32 = 1180.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Today,
    Review,
    Health,
    Pipelines,
    Projects,
    Board,
    Asking,
    Checks,
    Look,
    NewTicket,
    PlanReview,
    Design,
    Watch,
    Cost,
    Accounts,
    Sandbox,
}

/// Which tickets the window drives by itself, and what it already started.
#[derive(Default)]
pub(crate) struct AutoRun {
    pub(crate) driven: Vec<TicketId>,
    pub(crate) tried: Vec<(TicketId, Step)>,
}

/// What the Watch tab shows for the selected ticket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Run,
    Step(Step),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Summary,
    Output,
    Log,
    Changes,
}

/// One engine action on its own thread.
struct Running {
    id: u64,
    ticket: Option<TicketId>,
    what: &'static str,
    starts_ticket: bool,
}

#[allow(clippy::struct_excessive_bools, reason = "independent view toggles")]
pub struct App {
    pub(crate) options: Options,
    pub(crate) profile: Result<Profile, String>,
    pub(crate) snapshot: Snapshot,
    pub(crate) mode: Mode,
    pub(crate) screen: Screen,
    pub(crate) selected: Option<TicketId>,
    pub(crate) today_selected: Option<TicketId>,
    /// Open pipeline-review findings, from the record and the latest pass.
    pub(crate) findings: Vec<autopilot_engine::Finding>,
    /// Tunings in force, read when the data changes, never per frame.
    pub(crate) tunings: Vec<autopilot_engine::ActiveTuning>,
    /// Tier 2 changes waiting for the owner, recomputed when findings or the
    /// profile change, never per frame.
    pub(crate) suggestions: Vec<autopilot_engine::Suggestion>,
    review_stamp: u64,
    review_running: bool,
    /// Review settings being edited on the Pipeline review page.
    pub(crate) review_edit: Option<autopilot_profile::ReviewSettings>,
    pub(crate) health_selected: Option<String>,
    pub(crate) health_cache: Option<(String, u64, autopilot_engine::PipelineMetrics)>,
    pub(crate) plan_note: String,
    pub(crate) corrections: Vec<String>,
    /// The overlap check of the open ticket, kept until its history changes.
    pub(crate) overlaps: Option<(TicketId, u64, Vec<autopilot_engine::TicketOverlap>)>,
    pub(crate) search_open: bool,
    pub(crate) search_query: String,
    pub(crate) search_selected: usize,
    pub(crate) pipeline_selected: Option<String>,
    pub(crate) pipeline_notice: Option<(bool, String)>,
    pub(crate) draft_text: String,
    /// The question typed for the advisor on the open ticket.
    pub(crate) advise_text: String,
    /// What the owner is typing in a step's chat, by step.
    pub(crate) chat_text: String,
    /// The pipeline the agent box is changing, or none for a new one.
    pub(crate) draft_editing: Option<String>,
    /// The step row open on the Pipelines page.
    pub(crate) step_open: Option<Step>,
    /// A custom step being renamed or re-worded: its slot, name and prompt.
    pub(crate) custom_edit: Option<(Step, String, String)>,
    /// The "add a step" form: name and prompt.
    pub(crate) new_step: Option<(String, String)>,
    pub(crate) draft: Option<autopilot_engine::PipelineDraft>,
    /// The owner picked the pipeline on New ticket, so the suggestion stops.
    pub(crate) pipeline_chosen: bool,
    /// The project open on the Projects page, by normalised folder path.
    pub(crate) project_selected: Option<String>,
    pub(crate) rules_text: String,
    pub(crate) rules_saved: String,
    pub(crate) rules_notice: Option<(bool, String)>,
    pub(crate) card: crate::projects::CardState,
    /// A snapshot has arrived, so an empty list means no tickets, not no data yet.
    pub(crate) loaded: bool,
    pub(crate) ask_edits: std::collections::BTreeMap<Step, bool>,
    pub(crate) design_step: Step,
    pub(crate) advanced: bool,
    pub(crate) tab: Tab,
    pub(crate) notice: Option<(bool, String)>,
    pub(crate) change_text: String,
    pub(crate) reading: Option<Result<autopilot_engine::Reading, String>>,
    pub(crate) for_project: bool,
    pub(crate) profile_problem: Option<(Step, String)>,
    /// The actions running now. Tickets run side by side; one ticket has one
    /// action at a time.
    running: Vec<Running>,
    next_action: u64,
    /// The palette in use, resolved from `theme`.
    pub(crate) light: bool,
    pub(crate) theme: theme::ThemeChoice,
    pub(crate) agent: backend::AgentChoice,
    /// Texts of what each waiting request asks, read once per snapshot.
    waiting_cache: std::collections::HashMap<(TicketId, RequestId), String>,
    /// Whether the data folder holds the sample-data marker, read once per snapshot.
    pub(crate) sample_data: bool,
    /// The project folder last chosen, read once.
    pub(crate) saved_workspace: Option<std::path::PathBuf>,
    /// Tickets started in this window that the orchestrator has not looked at.
    fresh: Vec<TicketId>,
    /// Tickets started in this window and the steps already started for them
    /// by themselves.
    pub(crate) auto: AutoRun,
    pub(crate) orchestrator: backend::Orchestrator,
    /// "Do not ask again after this kind of step" on an approval.
    pub(crate) always: bool,
    pub(crate) notify: bool,
    /// Whether the first screen has been dealt with, kept in the data folder.
    pub(crate) welcomed: bool,
    /// Which step of the full-screen welcome the owner is on.
    pub(crate) welcome_step: usize,
    announced: bool,
    last_badges: std::collections::HashMap<TicketId, Badge>,
    update: Option<autopilot_update::Update>,
    installing: bool,
    pub(crate) home: NewTicket,
    pub(crate) kind: crate::new_ticket::Kind,
    watching: bool,
    next_update_check: Option<std::time::Instant>,
    pub(crate) accounts: Option<autopilot_accounts::Accounts>,
    pub(crate) sandbox: crate::sandbox::Probe,
    /// The API key is saved in the user environment (maybe after this start).
    pub(crate) key_saved: bool,
    /// Who Git signs commits as, read from the global Git settings.
    pub(crate) git_identity: Option<autopilot_accounts::GitIdentity>,
    /// The name and email being typed on the welcome Tools step.
    pub(crate) git_edit: Option<(String, String)>,
    /// The service logos of the welcome screens, loaded on first use.
    pub(crate) welcome_logos: Vec<egui::TextureHandle>,
    open_newest: bool,
    /// Tickets that finished starting and are not yet handed to the driver.
    new_tickets: usize,
    pub(crate) icon: Option<egui::TextureHandle>,
    pub(crate) group_by_status: bool,
    pub(crate) closed_groups: std::collections::HashSet<String>,
    pub(crate) context: egui::Context,
    pub(crate) sender: Sender<Message>,
    receiver: Receiver<Message>,
}

impl App {
    /// Build the window state. `watch` starts the thread that follows the data
    /// directory; tests leave it off and feed snapshots themselves.
    #[must_use]
    #[allow(clippy::too_many_lines, reason = "one field per piece of window state")]
    pub fn new(context: &egui::Context, options: Options, watch: bool) -> Self {
        let choice = read_theme(&options);
        let light = choice.is_light(context.system_theme());
        let agent = backend::AgentChoice::read(&options.data_dir);
        let orchestrator = backend::Orchestrator::read(&options.data_dir);
        let sample_data = options.data_dir.join(crate::demo::MARKER).exists();
        let saved_workspace = backend::saved_workspace(&options.data_dir);
        let notify = backend::notifications_on(&options.data_dir);
        // An install that already chose a folder predates the welcome.
        let welcomed = backend::welcomed(&options.data_dir) || saved_workspace.is_some();
        theme::apply(context, light);
        let (sender, receiver) = mpsc::channel();
        if watch {
            backend::watch(options.clone(), context.clone(), sender.clone());
            backend::fetch_accounts(context.clone(), sender.clone());
        }
        let profile = load_profile(&options.profile).map_err(|error| error.to_string());
        Self {
            options,
            profile,
            snapshot: Snapshot::default(),
            mode: Mode::Watch,
            screen: Screen::Run,
            selected: None,
            today_selected: None,
            findings: Vec::new(),
            tunings: Vec::new(),
            suggestions: Vec::new(),
            review_stamp: 0,
            review_running: false,
            review_edit: None,
            health_selected: None,
            health_cache: None,
            plan_note: String::new(),
            corrections: Vec::new(),
            overlaps: None,
            search_open: false,
            search_query: String::new(),
            search_selected: 0,
            pipeline_selected: None,
            pipeline_notice: None,
            draft_text: String::new(),
            advise_text: String::new(),
            chat_text: String::new(),
            draft_editing: None,
            step_open: None,
            custom_edit: None,
            new_step: None,
            draft: None,
            pipeline_chosen: false,
            project_selected: None,
            rules_text: String::new(),
            rules_saved: String::new(),
            rules_notice: None,
            card: crate::projects::CardState::default(),
            loaded: false,
            ask_edits: std::collections::BTreeMap::new(),
            design_step: Step::BuildCheck,
            advanced: true,
            tab: Tab::Summary,
            notice: None,
            change_text: String::new(),
            reading: None,
            for_project: false,
            profile_problem: None,
            running: Vec::new(),
            next_action: 0,
            light,
            theme: choice,
            agent,
            waiting_cache: std::collections::HashMap::new(),
            sample_data,
            saved_workspace,
            fresh: Vec::new(),
            auto: AutoRun::default(),
            orchestrator,
            always: false,
            notify,
            welcomed,
            welcome_step: 0,
            announced: false,
            last_badges: std::collections::HashMap::new(),
            update: None,
            installing: false,
            home: NewTicket::default(),
            kind: crate::new_ticket::Kind::default(),
            watching: watch,
            next_update_check: None,
            accounts: None,
            sandbox: crate::sandbox::Probe::Unknown,
            key_saved: false,
            git_identity: None,
            git_edit: None,
            welcome_logos: Vec::new(),
            open_newest: false,
            new_tickets: 0,
            icon: theme::icon_texture(context),
            group_by_status: false,
            closed_groups: std::collections::HashSet::new(),
            context: context.clone(),
            sender,
            receiver,
        }
    }

    /// Show a snapshot as if the watcher had just sent it.
    /// Tell the owner, with a desktop notification, about a ticket that just
    /// started waiting for them, failed or finished, when the window is not
    /// in front. The first snapshot only records what is already there.
    fn announce(&mut self, snapshot: &Snapshot) {
        let started = self.announced;
        let focused = self.context.input(|input| input.focused);
        for view in &snapshot.tickets {
            let before = self.last_badges.insert(view.id.clone(), view.badge);
            if !self.notify || focused || !started || before == Some(view.badge) {
                continue;
            }
            let message = match view.badge {
                Badge::NeedsYou => "is waiting for you",
                Badge::Failed => "failed",
                Badge::Done => "is done",
                _ => continue,
            };
            backend::notify(
                "Autopilot Mode On".to_owned(),
                format!("{} {message}", view.title),
            );
        }
        // After the first snapshot every change counts.
        self.announced = true;
    }

    /// Tell the owner about findings that opened in the last pass, once each,
    /// and only when the window is not in front and notifications are on.
    fn announce_findings(&self, opened: &[autopilot_engine::Finding]) {
        let focused = self.context.input(|input| input.focused);
        if !self.notify || focused {
            return;
        }
        for finding in opened {
            backend::notify(
                "Autopilot Mode On".to_owned(),
                format!(
                    "{} · {}: {}",
                    finding.pipeline,
                    autopilot_engine::step_name(finding.step),
                    finding.evidence
                ),
            );
        }
    }

    pub(crate) fn refresh_suggestions(&mut self) {
        self.suggestions = match &self.profile {
            Ok(profile) => autopilot_engine::proposals_for(
                &self.options.data_dir,
                &self.findings,
                profile,
                &self.tunings,
            )
            .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
    }

    /// Start a review pass when the data changed and the review is on. A pass
    /// runs off the window thread and never when the review is off.
    fn review_if_changed(&mut self) {
        let Ok(profile) = &self.profile else {
            return;
        };
        let settings = profile.review;
        let profile = profile.clone();
        if settings.mode == autopilot_profile::ReviewMode::Off || self.review_running {
            return;
        }
        let stamp: u64 = self
            .snapshot
            .tickets
            .iter()
            .map(|view| view.last_seq)
            .sum::<u64>()
            + self.snapshot.tickets.len() as u64;
        if stamp == self.review_stamp {
            return;
        }
        self.review_stamp = stamp;
        self.review_running = true;
        backend::review(
            self.options.data_dir.clone(),
            profile,
            self.context.clone(),
            self.sender.clone(),
        );
    }

    pub fn set_snapshot(&mut self, snapshot: Snapshot) {
        let first = self.snapshot.tickets.is_empty() && !snapshot.tickets.is_empty();
        self.loaded = true;
        self.waiting_cache.clear();
        self.sample_data = self.options.data_dir.join(crate::demo::MARKER).exists();
        self.announce(&snapshot);
        self.snapshot = snapshot;
        if self.findings.is_empty() {
            self.findings =
                autopilot_engine::open_findings(&self.options.data_dir).unwrap_or_default();
        }
        self.tunings = autopilot_engine::active_tunings(&self.options.data_dir).unwrap_or_default();
        self.refresh_suggestions();
        self.review_if_changed();
        if first {
            self.apply_start_view();
        }
        // Every ticket started here is driven, opened or not: the owner may have
        // gone on to write the next one.
        while self.new_tickets > 0 {
            let newest = self
                .snapshot
                .tickets
                .iter()
                .map(|view| view.id.clone())
                .find(|id| !self.auto.driven.contains(id));
            let Some(newest) = newest else {
                break;
            };
            self.new_tickets -= 1;
            self.fresh.push(newest.clone());
            self.auto.driven.push(newest.clone());
            if self.open_newest {
                self.open_newest = false;
                self.open(&newest);
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|id| self.ticket(id).is_none())
        {
            self.selected = None;
        }
    }

    /// Honour `--view` and `--ticket` once there are tickets to show.
    pub(crate) fn apply_start_view(&mut self) {
        let Some(view) = self.options.view.clone() else {
            return;
        };
        let wanted = self.options.ticket.clone();
        let id = self
            .snapshot
            .tickets
            .iter()
            .find(|ticket| {
                wanted
                    .as_deref()
                    .is_none_or(|name| ticket.id.as_str() == name)
            })
            .map(|ticket| ticket.id.clone());
        match (view.as_str(), id) {
            ("today" | "inbox", _) => self.mode = Mode::Today,
            ("plan", Some(id)) => {
                self.open(&id);
                self.mode = Mode::PlanReview;
            }
            ("new", _) => self.mode = Mode::NewTicket,
            ("settings" | "asking", _) => self.mode = Mode::Asking,
            ("design", _) => self.mode = Mode::Design,
            ("checks", _) => self.mode = Mode::Checks,
            ("look", _) => self.mode = Mode::Look,
            ("sandbox", _) => self.mode = Mode::Sandbox,
            ("board", _) => self.mode = Mode::Board,
            ("projects", _) => self.mode = Mode::Projects,
            ("pipelines", _) => self.mode = Mode::Pipelines,
            ("health", _) => self.mode = Mode::Health,
            ("review", _) => self.mode = Mode::Review,
            ("cost", _) => self.mode = Mode::Cost,
            ("accounts", _) => self.mode = Mode::Accounts,
            ("run", Some(id)) => self.open(&id),
            ("step", Some(id)) => {
                self.open(&id);
                self.screen = Screen::Step(Step::BuildCheck);
            }
            _ => {}
        }
    }

    pub(crate) fn ticket(&self, id: &TicketId) -> Option<&TicketView> {
        self.snapshot.tickets.iter().find(|view| &view.id == id)
    }

    pub(crate) fn current(&self) -> Option<&TicketView> {
        self.selected.as_ref().and_then(|id| self.ticket(id))
    }

    /// What a waiting request asks, read from the inbox once per snapshot.
    pub(crate) fn waiting_for(&mut self, ticket: &TicketId, request: &RequestId) -> Option<String> {
        let key = (ticket.clone(), request.clone());
        if let Some(text) = self.waiting_cache.get(&key) {
            return Some(text.clone());
        }
        let text = backend::waiting_text(&self.options, request)?;
        self.waiting_cache.insert(key, text.clone());
        Some(text)
    }

    /// Start the orchestrator for a ticket started in this window, once its
    /// Understand step has passed. A ticket from an earlier session is never
    /// picked up here, so opening the window never spends anything.
    fn auto_orchestrate(&mut self) {
        if self.agent != backend::AgentChoice::Claude
            || self.orchestrator == backend::Orchestrator::Off
        {
            return;
        }
        let ready = self.fresh.iter().position(|id| {
            self.ticket(id).is_some_and(|view| {
                view.triage.is_none()
                    && !self.running_for(id)
                    && view
                        .step(Step::Understand)
                        .is_some_and(|card| card.state == StepState::Passed)
            })
        });
        if let Some(at) = ready {
            let id = self.fresh.remove(at);
            self.act(Action::Orchestrate(id));
        }
    }

    /// Start the next step of a ticket started in this window, without a click:
    /// the design when the ticket changes a screen, then the plan, then the
    /// owner's own steps. It stops for anything that needs the owner: a
    /// question, an approval, a paused or failed ticket. Each step is tried once
    /// per ticket, so a refusal is never retried in a loop that spends money;
    /// the button on the ticket stays as the way to try again. Opening the
    /// window never starts anything: only a ticket started here is driven.
    fn auto_advance(&mut self) {
        let next = self.auto.driven.iter().find_map(|id| {
            if self.running_for(id) {
                return None;
            }
            if self.fresh.contains(id) {
                // The orchestrator goes first and picks the later steps' models.
                return None;
            }
            let view = self.ticket(id)?;
            let stopped = view
                .steps
                .iter()
                .any(|card| matches!(card.state, StepState::Failed(_) | StepState::Interrupted(_)));
            if view.asking.is_some()
                || stopped
                || view.badge == Badge::Paused
                || view.steps.iter().any(|card| card.state.is_active())
            {
                return None;
            }
            let (step, action) = if crate::run::ready_to_design(view) {
                (Step::Design, Action::MakeDesign(id.clone()))
            } else if crate::run::ready_to_plan(view) {
                (Step::Plan, Action::MakePlan(id.clone()))
            } else {
                let step = self.ready_custom(view)?;
                (
                    step,
                    Action::RunCustom {
                        ticket: id.clone(),
                        step,
                    },
                )
            };
            let key = (id.clone(), step);
            (!self.auto.tried.contains(&key)).then_some((key, action))
        });
        if let Some((key, action)) = next {
            self.auto.tried.push(key);
            self.act(action);
        }
    }

    /// Whether an action is running for this ticket.
    fn running_for(&self, id: &TicketId) -> bool {
        self.running
            .iter()
            .any(|each| each.ticket.as_ref() == Some(id))
    }

    /// Whether the open ticket has an action running, which is when its own
    /// buttons wait. Another ticket's work never blocks it.
    pub(crate) fn busy(&self) -> bool {
        self.current_work().is_some()
    }

    /// What the open ticket is doing, in words. With no ticket open, a ticket
    /// being created counts, so the first seconds are not blank.
    fn current_work(&self) -> Option<&'static str> {
        self.running
            .iter()
            .find(|each| each.ticket == self.selected)
            .map(|each| each.what)
    }

    /// Run an engine action without blocking the window.
    pub(crate) fn act(&mut self, action: Action) {
        self.next_action += 1;
        self.running.push(Running {
            id: self.next_action,
            ticket: action.ticket().cloned(),
            what: action.describe(),
            starts_ticket: matches!(action, Action::Start { .. }),
        });
        self.notice = None;
        backend::run(
            self.next_action,
            self.options.clone(),
            action,
            self.context.clone(),
            self.sender.clone(),
        );
    }

    /// Open a ticket from the sidebar or from a "needs you" card.
    /// "New ticket": go to the home screen with an empty message box.
    pub(crate) fn start_new(&mut self) {
        self.selected = None;
        self.mode = Mode::NewTicket;
        self.home = NewTicket::default();
    }

    pub(crate) fn open(&mut self, id: &TicketId) {
        self.selected = Some(id.clone());
        self.mode = Mode::Watch;
        self.screen = Screen::Run;
    }

    pub(crate) fn drain(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::Snapshot(snapshot) => self.set_snapshot(snapshot),
                Message::Review(outcome) => {
                    self.review_running = false;
                    if let Ok(outcome) = outcome {
                        self.findings.clone_from(&outcome.open);
                        self.refresh_suggestions();
                        self.announce_findings(&outcome.opened);
                    }
                }
                Message::Tuned(tuned) => {
                    self.tunings = autopilot_engine::active_tunings(&self.options.data_dir)
                        .unwrap_or_default();
                    let mut lines: Vec<String> = tuned
                        .applied
                        .iter()
                        .map(|each| {
                            format!(
                                "Tuned {} in {}: effort {} to {}. {}",
                                each.step.as_str(),
                                each.pipeline,
                                each.effort_before.as_deref().unwrap_or("unset"),
                                each.effort_after,
                                each.why
                            )
                        })
                        .collect();
                    lines.extend(tuned.reverted.iter().map(|(each, why)| {
                        format!(
                            "Reverted the tuning of {} in {}: {why}",
                            each.step.as_str(),
                            each.pipeline
                        )
                    }));
                    self.notice = Some((
                        true,
                        lines.join(
                            "
",
                        ),
                    ));
                }
                Message::Accounts(accounts) => self.accounts = Some(*accounts),
                Message::Sandbox(outcome) => {
                    self.sandbox = match outcome {
                        Ok(()) => crate::sandbox::Probe::Ready,
                        Err(refusal) => crate::sandbox::Probe::Refused(refusal),
                    };
                }
                Message::KeySaved(saved) => self.key_saved = saved,
                Message::GitIdentity(identity) => self.git_identity = Some(identity),
                Message::Update(Ok(found)) => self.update = found,
                // A failed check is not worth a banner: the next check retries.
                Message::Update(Err(_)) => {}
                Message::Installing(Ok(())) => {
                    self.context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Message::Installing(Err(why)) => {
                    self.installing = false;
                    self.notice = Some((false, format!("The update was not installed: {why}")));
                }
                Message::Done(id, outcome) => {
                    let ended = id
                        .and_then(|id| self.running.iter().position(|each| each.id == id))
                        .map(|at| self.running.remove(at));
                    // A new ticket opens by itself only when the owner is still
                    // waiting on it, not when they went on to something else.
                    if outcome.is_ok() && ended.is_some_and(|each| each.starts_ticket) {
                        self.new_tickets += 1;
                        self.open_newest = self.selected.is_none() && self.mode == Mode::Watch;
                    }
                    self.notice = Some(match outcome {
                        Ok(text) => (true, text),
                        Err(text) => (false, text),
                    });
                }
            }
        }
    }

    /// Lay out one frame.
    pub fn ui(&mut self, ctx: &egui::Context) {
        // Every screen words a custom step with the name its owner gave it.
        if let Ok(profile) = &self.profile {
            autopilot_engine::name_custom_steps(profile);
        }
        self.drain();
        self.update_clock(ctx);
        self.auto_orchestrate();
        self.auto_advance();
        self.follow_system_theme(ctx);
        if self.is_first_run() {
            self.welcome(ctx);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.start_new();
        }
        self.search_keys(ctx);
        self.today_keys(ctx);
        if matches!(self.screen, Screen::Step(_))
            && !ctx.wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.screen = Screen::Run;
        }
        self.take_drops(ctx);
        self.nav(ctx);
        if self.mode == Mode::Watch && self.current().is_some() {
            self.side_panel(ctx);
        }
        // The change box stays at the bottom of a ticket, so it is never
        // scrolled out of reach.
        if self.mode == Mode::Watch && self.current().is_some() {
            egui::TopBottomPanel::bottom("change")
                .resizable(false)
                // Room for the mode row and the box on the first frame; the panel
                // otherwise starts small and keeps that size until something moves.
                .min_height(150.0)
                .frame(
                    Frame::new()
                        .fill(theme::BACKGROUND())
                        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
                        .inner_margin(Margin::symmetric(32, 10)),
                )
                .show(ctx, |ui| {
                    // A proposal can be tall; the box scrolls inside its own panel
                    // instead of pushing the ticket off the screen.
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .auto_shrink([false, true])
                        .id_salt("change-scroll")
                        .show(ui, |ui| {
                            self.composer_bar(ui);
                        });
                });
        }
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND())
                    .inner_margin(Margin::symmetric(32, 20)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // One centred column, so lines stay short on a wide window.
                        let side = ((ui.available_width() - COLUMN) / 2.0).max(0.0);
                        ui.horizontal_top(|ui| {
                            ui.add_space(side);
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width().min(COLUMN));
                                self.notice_bar(ui);
                                match self.mode {
                                    Mode::Today => self.today_screen(ui),
                                    Mode::Review => self.review_screen(ui),
                                    Mode::Health => self.health_screen(ui),
                                    Mode::Pipelines => self.pipelines_screen(ui),
                                    Mode::Projects => self.projects_screen(ui),
                                    Mode::Board => self.board_screen(ui),
                                    Mode::NewTicket => self.new_ticket_screen(ui),
                                    Mode::PlanReview => self.plan_review(ui),
                                    Mode::Asking => self.asking_screen(ui),
                                    Mode::Checks => self.checks_screen(ui),
                                    Mode::Look => self.look_screen(ui),
                                    Mode::Design => self.pipeline_screen(ui),
                                    Mode::Watch => self.watch_screen(ui),
                                    Mode::Cost => self.cost_screen(ui),
                                    Mode::Accounts => self.accounts_screen(ui),
                                    Mode::Sandbox => self.sandbox_screen(ui),
                                }
                            });
                        });
                    });
            });
        self.search_overlay(ctx);
    }

    /// Files dropped on the window go to the open form, or to the ticket on
    /// screen, or to the start box on the home screen.
    fn take_drops(&mut self, ctx: &egui::Context) {
        let paths: Vec<std::path::PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if paths.is_empty() {
            return;
        }
        if self.mode == Mode::NewTicket {
            self.home.add(paths);
        } else if self.mode == Mode::Watch
            && let Some(ticket) = self.current().map(|view| view.id.clone())
        {
            self.act(Action::Attach { ticket, paths });
        }
    }

    fn notice_bar(&mut self, ui: &mut egui::Ui) {
        if self.update_offered().is_some() || self.update_installing() {
            ui.horizontal(|ui| self.update_button(ui));
        }
        if self.sample_data {
            banner(ui, "Sample data: these tickets are not real work.", false);
        }
        let problem = self.snapshot.problem.clone();
        if let Some(text) = problem {
            banner(ui, &format!("Could not read every ticket: {text}"), false);
        }
        if let Some((ok, text)) = self.notice.clone() {
            ui.horizontal(|ui| {
                banner(ui, &text, ok);
                if ui.small_button("dismiss").clicked() {
                    self.notice = None;
                }
            });
        }
        // A ticket page and the inbox show the same line beside the button that
        // started the work; a second one up here would say it twice.
        if !matches!(self.mode, Mode::Watch | Mode::Today) {
            self.busy_inline(ui);
        }
    }

    /// A spinner and what is happening, while an action runs. Drawn at the top
    /// of the page and beside each button that starts work, so a click always
    /// shows that it was taken.
    pub(crate) fn busy_inline(&self, ui: &mut egui::Ui) {
        let Some(what) = self.current_work() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0).color(theme::BLUE()));
            ui.label(theme::muted(format!("{what}...")));
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }

    fn cost_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(
            RichText::new("What did the tickets cost?")
                .size(18.0)
                .strong(),
        );
        ui.label(theme::muted(
            "API spend is exact. Subscription spend is an estimate and is shown apart.",
        ));
        ui.add_space(12.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Today").strong());
                theme::cost_tag(ui, false);
                ui.label(dollars(self.snapshot.today_api));
                theme::cost_tag(ui, true);
                ui.label(dollars(self.snapshot.today_subscription));
            });
            ui.separator();
            if self.snapshot.tickets.is_empty() {
                ui.label(theme::muted("No tickets yet."));
            }
            for view in &self.snapshot.tickets {
                ui.horizontal(|ui| {
                    ui.add_sized([260.0, 22.0], egui::Label::new(&view.title));
                    theme::cost_tag(ui, false);
                    ui.label(dollars(view.api));
                    ui.add_space(16.0);
                    theme::cost_tag(ui, true);
                    ui.label(dollars(view.subscription));
                });
            }
        });
    }

    /// Settings pages share a row of tabs and a way back. The ticket pages
    /// have no tabs: the sidebar is how you move between tickets.
    pub(crate) fn mode_tabs(&mut self, ui: &mut egui::Ui) {
        if self.mode == Mode::Watch {
            return;
        }
        if ui.link("< Back to tickets").clicked() {
            self.mode = Mode::Watch;
        }
        ui.add_space(6.0);
        theme::segmented(
            ui,
            &mut self.mode,
            &[
                (Mode::Asking, "When it asks you"),
                (Mode::Design, "Pipeline steps"),
                (Mode::Cost, "Cost"),
                (Mode::Review, "Pipeline review"),
                (Mode::Checks, "Checks"),
                (Mode::Accounts, "Accounts"),
                (Mode::Sandbox, "Sandbox"),
                (Mode::Look, "Look"),
            ],
        );
    }

    pub(crate) fn watch_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        match self.current().map(|view| view.id.clone()) {
            None => self.today_screen(ui),
            Some(id) => self.ticket_page(ui, &id),
        }
    }
}

/// The colours of a ticket's chip.
#[must_use]
pub fn badge_colors(badge: Badge, view: &TicketView) -> (String, Color32, Color32) {
    let minutes = view.last_ms.saturating_sub(view.started_ms) / 60_000;
    match badge {
        Badge::NeedsYou => ("Needs you".into(), theme::AMBER_FILL(), theme::AMBER()),
        // The step reads as running, but the owner approved and nothing is
        // working: the push waits for the owner's click.
        Badge::Running if crate::run::pull_request_approved(view) => (
            "Approved · not opened".into(),
            theme::BLUE_FILL(),
            theme::BLUE_TEXT(),
        ),
        Badge::Running => (
            format!("Running · {minutes} min"),
            theme::GREEN_FILL(),
            theme::GREEN(),
        ),
        Badge::Failed | Badge::Interrupted => {
            (badge.label().into(), theme::RED_FILL(), theme::RED())
        }
        Badge::Done => ("Done".into(), theme::GREEN_FILL(), theme::GREEN()),
        Badge::Paused | Badge::Idle => {
            (badge.label().into(), theme::GREY_FILL(), theme::GREY_TEXT())
        }
    }
}

pub(crate) fn ticket_row(ui: &mut egui::Ui, view: &TicketView, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    let painter = ui.painter();
    if selected || response.hovered() {
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(7),
            if selected {
                theme::NODE()
            } else {
                theme::SELECTED()
            },
        );
    }
    let dot = match view.badge {
        Badge::NeedsYou => theme::AMBER_SOLID(),
        Badge::Running => theme::BLUE(),
        Badge::Failed | Badge::Interrupted => theme::RED_SOLID(),
        Badge::Done => theme::GREEN_SOLID(),
        Badge::Paused | Badge::Idle => theme::TEXT_MUTED(),
    };
    painter.circle_filled(rect.right_center() - egui::vec2(14.0, 0.0), 4.0, dot);
    // One line, cut at the dot: a long title must not run under it.
    let mut job = egui::text::LayoutJob::single_section(
        view.title.clone(),
        egui::TextFormat::simple(
            egui::FontId::monospace(12.0),
            if selected {
                theme::TEXT()
            } else {
                theme::TEXT_SOFT()
            },
        ),
    );
    job.wrap.max_width = rect.width() - 44.0;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::TEXT(),
    );
    response
        .on_hover_text(format!("{}: {}", view.title, view.badge.label()))
        .clicked()
}

/// A one-line message with a colour for good or bad news.
pub fn banner(ui: &mut egui::Ui, text: &str, ok: bool) {
    let (fill, color) = if ok {
        (theme::GREEN_FILL(), theme::GREEN())
    } else {
        (theme::AMBER_FILL(), theme::AMBER())
    };
    Frame::new()
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color));
        });
}

const THEME_FILE: &str = "ui-theme.txt";

fn read_theme(options: &Options) -> theme::ThemeChoice {
    std::fs::read_to_string(options.data_dir.join(THEME_FILE))
        .map(|text| theme::ThemeChoice::parse(&text))
        .unwrap_or_default()
}

fn save_theme(options: &Options, choice: theme::ThemeChoice) {
    let _ = std::fs::create_dir_all(&options.data_dir);
    let _ = std::fs::write(options.data_dir.join(THEME_FILE), choice.word());
}

impl App {
    pub(crate) fn choose_theme(&mut self, choice: theme::ThemeChoice) {
        self.theme = choice;
        save_theme(&self.options, choice);
        self.light = choice.is_light(self.context.system_theme());
        theme::apply(&self.context, self.light);
    }

    /// Re-apply the palette when the system switches while `System` is chosen.
    fn follow_system_theme(&mut self, ctx: &egui::Context) {
        let light = self.theme.is_light(ctx.system_theme());
        if light != self.light {
            self.light = light;
            theme::apply(ctx, light);
        }
    }
}

impl App {}

/// How often a running window looks for a new release.
const UPDATE_EVERY: std::time::Duration = std::time::Duration::from_hours(6);

impl App {
    /// The version the update button offers, if any.
    #[must_use]
    pub fn update_offered(&self) -> Option<&str> {
        self.update.as_ref().map(|update| update.version.as_str())
    }

    /// Whether an installer is being downloaded right now.
    #[must_use]
    pub const fn update_installing(&self) -> bool {
        self.installing
    }

    /// "Update 0.2.0": one click downloads, checks and opens the installer.
    fn update_button(&mut self, ui: &mut egui::Ui) {
        let Some(update) = self.update.clone() else {
            return;
        };
        let label = if self.installing {
            format!("Downloading {}...", update.version)
        } else {
            format!("Update to {}", update.version)
        };
        let tip = if update.notes.is_empty() {
            "Download the new version, check it, and install it".to_owned()
        } else {
            update.notes.clone()
        };
        if ui
            .add_enabled(!self.installing, theme::primary(&label))
            .on_hover_text(tip)
            .clicked()
        {
            self.installing = true;
            backend::install_update(self.context.clone(), self.sender.clone(), update);
        }
    }

    /// Check at start and then every few hours while the window stays open.
    pub(crate) fn update_clock(&mut self, ctx: &egui::Context) {
        if !self.watching {
            return;
        }
        let now = std::time::Instant::now();
        if self.next_update_check.is_none_or(|at| now >= at) {
            self.next_update_check = Some(now + UPDATE_EVERY);
            backend::check_for_update(ctx.clone(), self.sender.clone());
        }
        ctx.request_repaint_after(UPDATE_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo, now_ms};

    // Invariant: while an action runs, its words appear once on the ticket
    // page, beside the work, not again in the page header. A header line plus
    // a card line said "Making the plan..." twice.
    #[test]
    fn the_busy_line_shows_once_on_a_ticket_page() {
        let directory = tempfile::tempdir().unwrap();
        let options = crate::backend::Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        crate::backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        demo::seed_build_states(&options.data_dir, now_ms()).unwrap();
        crate::backend::mark_welcomed(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(crate::backend::read(&options));
        let id = TicketId::from("demo-approved");
        app.open(&id);
        app.running.push(Running {
            id: 1,
            ticket: Some(id),
            what: "Making the plan",
            starts_ticket: false,
        });
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1280.0, 4000.0),
            )),
            ..Default::default()
        };
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(input.clone(), |context| app.ui(context)));
        }
        let mut count = 0;
        for clipped in output.unwrap().shapes {
            if let egui::Shape::Text(text) = clipped.shape
                && text.galley.text() == "Making the plan..."
            {
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }
}
