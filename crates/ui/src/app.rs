//! The window: title bar, ticket sidebar and the three screens of the design.
//! It holds no logic of its own. It paints what `engine` reports and turns
//! clicks into engine actions.

use std::sync::mpsc::{self, Receiver, Sender};

use autopilot_core::{RequestId, Step, TicketId};
use autopilot_engine::{Badge, TicketView, dollars, load_profile};
use autopilot_pipeline::StepState;
use autopilot_profile::Profile;
use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Stroke};

use crate::backend::{self, Action, Message, Options, Snapshot};
use crate::composer::NewTicket;
use crate::theme;

/// The width of the one column the screens are laid out in.
const COLUMN: f32 = 900.0;

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
    pub(crate) draft: Option<autopilot_engine::PipelineDraft>,
    /// The owner picked the pipeline on New ticket, so the suggestion stops.
    pub(crate) pipeline_chosen: bool,
    /// The project open on the Projects page, by normalised folder path.
    pub(crate) project_selected: Option<String>,
    pub(crate) rules_text: String,
    pub(crate) rules_saved: String,
    pub(crate) rules_notice: Option<(bool, String)>,
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
    pub(crate) help_open: bool,
    pub(crate) busy: bool,
    pub(crate) new_started: bool,
    /// The palette in use, resolved from `theme`.
    pub(crate) light: bool,
    pub(crate) theme: theme::ThemeChoice,
    pub(crate) agent: backend::AgentChoice,
    /// What the running action is doing, in words.
    busy_what: &'static str,
    /// Texts of what each waiting request asks, read once per snapshot.
    waiting_cache: std::collections::HashMap<(TicketId, RequestId), String>,
    /// Whether the data folder holds the sample-data marker, read once per snapshot.
    sample_data: bool,
    /// The project folder last chosen, read once.
    pub(crate) saved_workspace: Option<std::path::PathBuf>,
    /// Tickets started in this window that the orchestrator has not looked at.
    fresh: Vec<TicketId>,
    pub(crate) orchestrator: backend::Orchestrator,
    /// "Do not ask again after this kind of step" on an approval.
    pub(crate) always: bool,
    pub(crate) notify: bool,
    announced: bool,
    last_badges: std::collections::HashMap<TicketId, Badge>,
    update: Option<autopilot_update::Update>,
    installing: bool,
    pub(crate) home: NewTicket,
    pub(crate) kind: crate::new_ticket::Kind,
    watching: bool,
    next_update_check: Option<std::time::Instant>,
    pub(crate) accounts: Option<autopilot_accounts::Accounts>,
    open_newest: bool,
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
    pub fn new(context: &egui::Context, options: Options, watch: bool) -> Self {
        let choice = read_theme(&options);
        let light = choice.is_light(context.system_theme());
        let agent = backend::AgentChoice::read(&options.data_dir);
        let orchestrator = backend::Orchestrator::read(&options.data_dir);
        let sample_data = options.data_dir.join(crate::demo::MARKER).exists();
        let saved_workspace = backend::saved_workspace(&options.data_dir);
        let notify = backend::notifications_on(&options.data_dir);
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
            draft: None,
            pipeline_chosen: false,
            project_selected: None,
            rules_text: String::new(),
            rules_saved: String::new(),
            rules_notice: None,
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
            help_open: false,
            busy: false,
            new_started: false,
            light,
            theme: choice,
            agent,
            busy_what: "Working",
            waiting_cache: std::collections::HashMap::new(),
            sample_data,
            saved_workspace,
            fresh: Vec::new(),
            orchestrator,
            always: false,
            notify,
            announced: false,
            last_badges: std::collections::HashMap::new(),
            update: None,
            installing: false,
            home: NewTicket::default(),
            kind: crate::new_ticket::Kind::default(),
            watching: watch,
            next_update_check: None,
            accounts: None,
            open_newest: false,
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
        self.review_if_changed();
        if first {
            self.apply_start_view();
        }
        if self.open_newest {
            self.open_newest = false;
            if let Some(newest) = self.snapshot.tickets.first().map(|view| view.id.clone()) {
                self.fresh.push(newest.clone());
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
    fn apply_start_view(&mut self) {
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
            ("checks", _) => self.mode = Mode::Checks,
            ("look", _) => self.mode = Mode::Look,
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
        if self.busy
            || self.agent != backend::AgentChoice::Claude
            || self.orchestrator == backend::Orchestrator::Off
        {
            return;
        }
        let ready = self.fresh.iter().position(|id| {
            self.ticket(id).is_some_and(|view| {
                view.triage.is_none()
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

    /// Run an engine action without blocking the window.
    pub(crate) fn act(&mut self, action: Action) {
        self.busy = true;
        self.busy_what = action.describe();
        self.notice = None;
        backend::run(
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
                Message::Done(outcome) => {
                    self.busy = false;
                    self.open_newest = outcome.is_ok() && self.new_started;
                    self.new_started = false;
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
        self.drain();
        self.update_clock(ctx);
        self.auto_orchestrate();
        self.follow_system_theme(ctx);
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
        self.title_bar(ctx);
        self.nav(ctx);
        if self.mode == Mode::Watch && self.current().is_some() {
            self.side_panel(ctx);
        }
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND())
                    .inner_margin(Margin::symmetric(40, 26)),
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
                                }
                            });
                        });
                    });
            });
        self.dialogs(ctx);
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

    fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("title")
            .exact_height(54.0)
            .frame(
                Frame::new()
                    .fill(theme::TITLE_BAR())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .inner_margin(Margin::symmetric(20, 9)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(icon) = &self.icon {
                        ui.add(egui::Image::new(icon).fit_to_exact_size(egui::vec2(30.0, 30.0)));
                    }
                    ui.label(RichText::new("Autopilot Mode On").size(16.0).strong());
                    ui.separator();
                    self.crumbs(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::named(ui.button("?").on_hover_text("Help"), "Help").clicked() {
                            self.help_open = !self.help_open;
                        }
                        self.update_button(ui);
                    });
                });
            });
    }

    fn crumbs(&self, ui: &mut egui::Ui) {
        if let (Mode::Watch, Screen::Step(_), Some(view)) = (self.mode, self.screen, self.current())
        {
            {
                ui.label(RichText::new(&view.title).color(theme::TEXT_SOFT()));
                if let Screen::Step(step) = self.screen {
                    ui.label(theme::muted(">"));
                    ui.label(autopilot_engine::step_name(step));
                } else {
                    let (text, fill, color) = badge_colors(view.badge, view);
                    theme::pill(ui, &text, fill, color);
                }
            }
        } else {
            {
                let name = self
                    .options
                    .profile
                    .parent()
                    .and_then(|folder| folder.file_name())
                    .map_or_else(
                        || "project".to_owned(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                ui.label(theme::mono(name));
            }
        }
    }

    fn notice_bar(&mut self, ui: &mut egui::Ui) {
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
        if self.busy {
            ui.label(theme::muted(format!("{}...", self.busy_what)));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.help_open {
            let mut open = true;
            egui::Window::new("Help").open(&mut open).show(ctx, |ui| {
                ui.label("Home: describe what should change and press Start. Tickets that need you are listed first.");
                ui.label("A ticket: the box on top says what to do next. Click a step for its details.");
                ui.label("Settings (bottom left): pipeline steps, cost, and who is signed in. Ctrl+N starts a ticket.");
                ui.label("A grey control marked coming soon is not built yet.");
                ui.label("Keyboard: Tab and Shift+Tab move between controls, Enter or Space press one, Escape closes a dialog.");
            });
            self.help_open = open;
        }
    }

    fn cost_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(
            RichText::new("What did the tickets cost?")
                .size(30.0)
                .strong(),
        );
        ui.label(theme::muted(
            "API spend is exact. Subscription spend is an estimate and is shown apart.",
        ));
        ui.add_space(12.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 38.0);
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
    let fill = if selected {
        theme::SELECTED()
    } else {
        Color32::TRANSPARENT
    };
    let width = ui.available_width();
    let response = ui.add(
        egui::Button::new("")
            .fill(fill)
            .stroke(Stroke::NONE)
            .min_size(egui::vec2(width, 48.0)),
    );
    let painter = ui.painter();
    let rect = response.rect;
    painter.text(
        rect.left_center() + egui::vec2(16.0, 0.0),
        egui::Align2::LEFT_CENTER,
        &view.title,
        egui::FontId::proportional(16.0),
        theme::TEXT(),
    );
    let (label, bg, fg) = badge_colors(view.badge, view);
    if view.badge == Badge::Running {
        painter.circle_filled(
            rect.right_center() - egui::vec2(18.0, 0.0),
            4.5,
            theme::GREEN(),
        );
    } else {
        let short = label.split(" · ").next().unwrap_or(&label).to_owned();
        let galley = painter.layout_no_wrap(short, egui::FontId::proportional(12.0), fg);
        let size = galley.size() + egui::vec2(18.0, 6.0);
        let chip = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 12.0 - size.x, rect.center().y - size.y / 2.0),
            size,
        );
        painter.rect_filled(chip, egui::CornerRadius::same(99), bg);
        painter.galley(chip.min + egui::vec2(9.0, 3.0), galley, fg);
    }
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
