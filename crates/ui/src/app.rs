//! The window: title bar, ticket sidebar and the three screens of the design.
//! It holds no logic of its own. It paints what `engine` reports and turns
//! clicks into engine actions.

use std::sync::mpsc::{self, Receiver, Sender};

use autopilot_core::{Step, TicketId};
use autopilot_engine::{Badge, TicketView, dollars, load_profile};
use autopilot_profile::Profile;
use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Stroke};

use crate::backend::{self, Action, Message, Options, Snapshot};
use crate::theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
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
    Overview,
    Input,
    Prompt,
    Output,
    Log,
    Changes,
    Cost,
    Settings,
}

#[derive(Default)]
pub struct NewTicket {
    focused: bool,
    pub title: String,
    pub description: String,
}

#[allow(clippy::struct_excessive_bools, reason = "independent view toggles")]
pub struct App {
    pub(crate) options: Options,
    pub(crate) profile: Result<Profile, String>,
    pub(crate) snapshot: Snapshot,
    pub(crate) mode: Mode,
    pub(crate) screen: Screen,
    pub(crate) selected: Option<TicketId>,
    pub(crate) design_step: Step,
    pub(crate) advanced: bool,
    pub(crate) tab: Tab,
    pub(crate) notice: Option<(bool, String)>,
    pub(crate) new_ticket: Option<NewTicket>,
    pub(crate) change_text: String,
    pub(crate) profile_problem: Option<(Step, String)>,
    pub(crate) help_open: bool,
    pub(crate) busy: bool,
    new_started: bool,
    pub(crate) light: bool,
    pub(crate) accounts: Option<autopilot_accounts::Accounts>,
    open_newest: bool,
    icon: Option<egui::TextureHandle>,
    pub(crate) context: egui::Context,
    pub(crate) sender: Sender<Message>,
    receiver: Receiver<Message>,
}

impl App {
    /// Build the window state. `watch` starts the thread that follows the data
    /// directory; tests leave it off and feed snapshots themselves.
    #[must_use]
    pub fn new(context: &egui::Context, options: Options, watch: bool) -> Self {
        let light = read_theme(&options);
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
            mode: Mode::Design,
            screen: Screen::Run,
            selected: None,
            design_step: Step::BuildCheck,
            advanced: true,
            tab: Tab::Overview,
            notice: None,
            new_ticket: None,
            change_text: String::new(),
            profile_problem: None,
            help_open: false,
            busy: false,
            new_started: false,
            light,
            accounts: None,
            open_newest: false,
            icon: theme::icon_texture(context),
            context: context.clone(),
            sender,
            receiver,
        }
    }

    /// Show a snapshot as if the watcher had just sent it.
    pub fn set_snapshot(&mut self, snapshot: Snapshot) {
        let first = self.snapshot.tickets.is_empty() && !snapshot.tickets.is_empty();
        self.snapshot = snapshot;
        if first {
            self.apply_start_view();
        }
        if self.open_newest {
            self.open_newest = false;
            if let Some(newest) = self.snapshot.tickets.first().map(|view| view.id.clone()) {
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

    /// Run an engine action without blocking the window.
    pub(crate) fn act(&mut self, action: Action) {
        self.busy = true;
        self.notice = None;
        backend::run(
            self.options.clone(),
            action,
            self.context.clone(),
            self.sender.clone(),
        );
    }

    /// Open a ticket from the sidebar or from a "needs you" card.
    pub(crate) fn open(&mut self, id: &TicketId) {
        self.selected = Some(id.clone());
        self.mode = Mode::Watch;
        self.screen = Screen::Run;
    }

    fn drain(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::Snapshot(snapshot) => self.set_snapshot(snapshot),
                Message::Accounts(accounts) => self.accounts = Some(*accounts),
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
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.new_ticket = Some(NewTicket::default());
        }
        self.title_bar(ctx);
        self.sidebar(ctx);
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
                        self.notice_bar(ui);
                        match self.mode {
                            Mode::Design => self.pipeline_screen(ui),
                            Mode::Watch => self.watch_screen(ui),
                            Mode::Cost => self.cost_screen(ui),
                            Mode::Accounts => self.accounts_screen(ui),
                        }
                    });
            });
        self.dialogs(ctx);
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
                    ui.label(RichText::new("Pipeline").size(16.0).strong());
                    ui.separator();
                    self.crumbs(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("?").on_hover_text("Help").clicked() {
                            self.help_open = !self.help_open;
                        }
                        let label = if self.light { "Dark" } else { "Light" };
                        if ui.button(label).on_hover_text("Switch theme").clicked() {
                            self.light = !self.light;
                            theme::apply(&self.context, self.light);
                            save_theme(&self.options, self.light);
                        }
                        self.spend_chip(ui);
                    });
                });
            });
    }

    fn crumbs(&self, ui: &mut egui::Ui) {
        if let (Mode::Watch, Some(view)) = (self.mode, self.current()) {
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

    fn spend_chip(&self, ui: &mut egui::Ui) {
        let budget = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.budgets.daily_microusd);
        let mut text = format!("today {}", dollars(self.snapshot.today_api));
        if let Some(limit) = budget {
            text = format!(
                "{text} of {}",
                dollars(autopilot_core::Money::from_microusd(limit.get()))
            );
        }
        Frame::new()
            .stroke(Stroke::new(1.0_f32, theme::BORDER()))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    theme::cost_tag(ui, false);
                    ui.label(RichText::new(text).color(theme::TEXT_SOFT()));
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(270.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .inner_margin(Margin::same(12)),
            )
            .show(ctx, |ui| {
                ui.add_space(40.0);
                let new = egui::Button::new(RichText::new("+  New ticket").size(16.0))
                    .fill(theme::SELECTED())
                    .min_size(egui::vec2(ui.available_width(), 46.0));
                if ui.add(new).clicked() {
                    self.new_ticket = Some(NewTicket::default());
                }
                ui.add_space(24.0);
                ui.label(RichText::new("   Tickets").color(theme::TEXT_MUTED()));
                if self.snapshot.tickets.is_empty() {
                    ui.label(theme::muted(
                        "   No tickets yet. Start one, or run with --demo.",
                    ));
                }
                self.needs_you_cards(ui);
                let mut open = None;
                for view in &self.snapshot.tickets {
                    if ticket_row(ui, view, self.selected.as_ref() == Some(&view.id)) {
                        open = Some(view.id.clone());
                    }
                }
                if let Some(id) = open {
                    self.open(&id);
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    if self.account_dots(ui) {
                        self.mode = Mode::Accounts;
                    }
                    ui.add_space(6.0);
                    let settings = egui::Button::new(RichText::new("Settings").size(16.0))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .min_size(egui::vec2(ui.available_width(), 44.0));
                    if ui
                        .add(settings)
                        .on_hover_text("Step settings are on the pipeline screen")
                        .clicked()
                    {
                        self.mode = Mode::Design;
                    }
                });
            });
    }

    fn notice_bar(&mut self, ui: &mut egui::Ui) {
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
            ui.label(theme::muted("Working..."));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut start = None;
        let mut close = false;
        if let Some(form) = &mut self.new_ticket {
            egui::Window::new("New ticket")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label("Title");
                    let title = ui.add(egui::TextEdit::singleline(&mut form.title).desired_width(420.0));
                    if !form.focused {
                        title.request_focus();
                        form.focused = true;
                    }
                    ui.label("What should change?");
                    ui.add(
                        egui::TextEdit::multiline(&mut form.description)
                            .desired_rows(4)
                            .desired_width(420.0),
                    );
                    ui.label(theme::muted(
                        "Runs the local fixture agent: Understand, then Clarify. Real agents arrive with the Claude Code step (S6).",
                    ));
                    let enter = ctx.input(|input| {
                        input.key_pressed(egui::Key::Enter) && input.modifiers.command
                    });
                    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                    ui.horizontal(|ui| {
                        let ready = !form.title.trim().is_empty();
                        if ui.add_enabled(ready, theme::primary("Start")).clicked()
                            || (enter && ready)
                        {
                            start = Some(Action::Start {
                                title: form.title.clone(),
                                description: form.description.clone(),
                            });
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
        }
        if let Some(action) = start {
            self.new_ticket = None;
            self.mode = Mode::Watch;
            self.new_started = true;
            self.act(action);
        } else if close {
            self.new_ticket = None;
        }
        if self.help_open {
            let mut open = true;
            egui::Window::new("Help").open(&mut open).show(ctx, |ui| {
                ui.label("Design the pipeline: choose the settings of each step. They are saved in profile.toml.");
                ui.label("Watch a run: pick a ticket in the sidebar, then click a step for its details.");
                ui.label("Accounts: who is signed in to GitHub, Claude, Codex and Azure. Ctrl+N starts a ticket; Ctrl+Enter in the form sends it.");
                ui.label("Cost: what each ticket spent. API spend is exact; subscription spend is an estimate and is never added to it.");
                ui.label("A grey control with a hover note is not built yet.");
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
            ui.set_width(ui.available_width() - 34.0);
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

    pub(crate) fn mode_tabs(&mut self, ui: &mut egui::Ui) {
        theme::segmented(
            ui,
            &mut self.mode,
            &[
                (Mode::Design, "Design the pipeline"),
                (Mode::Watch, "Watch a run"),
                (Mode::Cost, "Cost"),
                (Mode::Accounts, "Accounts"),
            ],
        );
    }

    pub(crate) fn watch_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        match self.current().map(|view| view.id.clone()) {
            None => {
                ui.label(RichText::new("Watch a run").size(30.0).strong());
                ui.label(theme::muted("Pick a ticket in the sidebar to watch it."));
            }
            Some(id) => match self.screen {
                Screen::Run => self.run_screen(ui, &id),
                Screen::Step(step) => self.step_screen(ui, &id, step),
            },
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

fn ticket_row(ui: &mut egui::Ui, view: &TicketView, selected: bool) -> bool {
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

fn read_theme(options: &Options) -> bool {
    std::fs::read_to_string(options.data_dir.join(THEME_FILE))
        .is_ok_and(|text| text.trim() == "light")
}

fn save_theme(options: &Options, light: bool) {
    let _ = std::fs::create_dir_all(&options.data_dir);
    let _ = std::fs::write(
        options.data_dir.join(THEME_FILE),
        if light { "light" } else { "dark" },
    );
}

impl App {
    /// A waiting ticket shows its question right in the sidebar, with the
    /// answer one click away, so approving never needs a screen change.
    fn needs_you_cards(&mut self, ui: &mut egui::Ui) {
        let waiting: Vec<(TicketId, String, String)> = self
            .snapshot
            .tickets
            .iter()
            .filter_map(|view| {
                let (_, request) = view.asking.as_ref()?;
                let text = backend::waiting_text(&self.options, request)?;
                Some((view.id.clone(), view.title.clone(), text))
            })
            .collect();
        for (id, title, text) in waiting {
            let question = text.contains('?');
            Frame::new()
                .fill(theme::AMBER_FILL())
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 20.0);
                    ui.label(
                        RichText::new(format!("{title} needs you"))
                            .strong()
                            .color(theme::AMBER()),
                    );
                    ui.label(RichText::new(text).size(13.0));
                    ui.horizontal_wrapped(|ui| {
                        if question {
                            if ui.button("Use defaults").clicked() {
                                self.act(Action::AcceptDefaults { ticket: id.clone() });
                            }
                        } else {
                            if ui.button("Approve").clicked() {
                                self.act(Action::Decide {
                                    ticket: id.clone(),
                                    approve: true,
                                });
                            }
                            if ui.button("Reject").clicked() {
                                self.act(Action::Decide {
                                    ticket: id.clone(),
                                    approve: false,
                                });
                            }
                        }
                        if ui.button("Open").clicked() {
                            self.open(&id);
                        }
                    });
                });
            ui.add_space(6.0);
        }
    }
}
