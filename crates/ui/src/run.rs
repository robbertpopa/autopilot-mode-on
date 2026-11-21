//! "Watch a run": the live strip, the Build and Check panel, the timeline and
//! the panel on the right with progress, spend and actions.

use autopilot_core::{Step, TicketId};
use autopilot_engine::{Badge, TicketView, dollars, step_name};
use autopilot_pipeline::StepState;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke};

use crate::app::{App, Screen, badge_colors};
use crate::backend::{Action, waiting_text};
use crate::theme;

/// Understand and Clarify are settled and Plan has not started.
fn ready_to_plan(view: &TicketView) -> bool {
    let state = |step| {
        view.steps
            .iter()
            .find(|card| card.step == step)
            .map(|card| &card.state)
    };
    state(Step::Understand) == Some(&StepState::Passed)
        && matches!(
            state(Step::Clarify),
            Some(StepState::Passed | StepState::Skipped(_))
        )
        && state(Step::Plan) == Some(&StepState::Idle)
}

/// A size in plain units: `12 KB`, `3.4 MB`.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{}.{} MB", bytes / MB, bytes % MB * 10 / MB)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{bytes} B")
    }
}

/// `14:02:11` in UTC. The window has no timezone data, so it says so.
#[must_use]
pub fn clock(ms: u64) -> String {
    let seconds = ms / 1000 % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn glyph(state: &StepState) -> (&'static str, Color32) {
    match state {
        StepState::Passed => ("✔", theme::GREEN()),
        StepState::Running => ("▶", theme::BLUE()),
        StepState::Asking(_) => ("▶", theme::AMBER()),
        StepState::Paused => ("||", theme::AMBER()),
        StepState::Failed(_) | StepState::Interrupted(_) => ("×", theme::RED()),
        StepState::Skipped(_) | StepState::Idle => ("○", theme::TEXT_MUTED()),
    }
}

pub(crate) fn state_text(state: &StepState) -> String {
    match state {
        StepState::Idle => "waiting".into(),
        StepState::Running => "running".into(),
        StepState::Asking(_) => "waiting for you".into(),
        StepState::Paused => "paused".into(),
        StepState::Skipped(reason) => format!("skipped: {reason}"),
        StepState::Passed => "done".into(),
        StepState::Failed(reason) => format!("failed: {reason}"),
        StepState::Interrupted(reason) => format!("interrupted: {reason}"),
    }
}

impl App {
    pub(crate) fn run_screen(&mut self, ui: &mut egui::Ui, id: &TicketId) {
        let Some(view) = self.ticket(id).cloned() else {
            return;
        };
        self.run_header(ui, &view);
        ui.add_space(14.0);
        self.next_card(ui, &view);
        self.step_chips(ui, &view);
        ui.add_space(14.0);
        self.build_panel(ui, &view);
        ui.add_space(6.0);
        self.files_panel(ui, &view);
        egui::CollapsingHeader::new("History")
            .id_salt("history")
            .show(ui, |ui| {
                ui.label(theme::muted(format!(
                    "Started {} UTC",
                    clock(view.started_ms)
                )));
                for line in &view.timeline {
                    ui.label(theme::mono(format!(
                        "{}   {}",
                        clock(line.at_ms),
                        line.text
                    )));
                }
            });
        egui::CollapsingHeader::new("Change the settings in plain words")
            .id_salt("change")
            .show(ui, |ui| self.change_box(ui));
    }

    /// What is attached to the ticket, and how to add more.
    fn files_panel(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let title = if view.attachments.is_empty() {
            "Files".to_owned()
        } else {
            format!("Files ({})", view.attachments.len())
        };
        egui::CollapsingHeader::new(title)
            .id_salt("files")
            .default_open(!view.attachments.is_empty())
            .show(ui, |ui| {
                for item in &view.attachments {
                    let glyph = match item.kind {
                        autopilot_core::AttachmentKind::Folder => "▤",
                        autopilot_core::AttachmentKind::Image => "▣",
                        autopilot_core::AttachmentKind::File => "▪",
                    };
                    let size = if item.kind == autopilot_core::AttachmentKind::Folder {
                        format!("{} files, {}", item.items, size_text(item.bytes))
                    } else {
                        size_text(item.bytes)
                    };
                    ui.horizontal(|ui| {
                        ui.label(theme::muted(glyph));
                        ui.label(&item.name).on_hover_text(&item.stored);
                        ui.label(theme::muted(size));
                    });
                }
                ui.label(theme::muted(
                    "Kept as data. Nothing opens or runs them, and no agent reads them yet.",
                ));
                if crate::composer::has_picker() {
                    ui.horizontal(|ui| {
                        if ui.button("Add files").clicked() {
                            let paths = crate::composer::pick_files();
                            if !paths.is_empty() {
                                self.act(Action::Attach {
                                    ticket: view.id.clone(),
                                    paths,
                                });
                            }
                        }
                        if ui.button("Add folder").clicked()
                            && let Some(path) = crate::composer::pick_folder()
                        {
                            self.act(Action::Attach {
                                ticket: view.id.clone(),
                                paths: vec![path],
                            });
                        }
                    });
                } else {
                    ui.label(theme::muted(
                        "Drop files or a folder on the window to add them.",
                    ));
                }
            });
    }

    /// Title, status, spend, and the controls that matter.
    fn run_header(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&view.title).size(28.0).strong());
            let (text, fill, color) = badge_colors(view.badge, view);
            theme::pill(ui, &text, fill, color);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("More", |ui| {
                    for (text, why) in [
                        (
                            "Open the worktree",
                            "coming soon: no run has made a worktree yet (needs S6)",
                        ),
                        (
                            "Open in a terminal",
                            "coming soon: it needs the real agent session (S6)",
                        ),
                    ] {
                        ui.add_enabled(false, egui::Button::new(text))
                            .on_disabled_hover_text(why);
                        ui.label(theme::muted(why));
                    }
                });
                let live = matches!(view.badge, Badge::Running | Badge::NeedsYou | Badge::Paused);
                if live
                    && ui
                        .button(RichText::new("Stop").color(theme::RED()))
                        .on_hover_text("Stops between turns")
                        .clicked()
                {
                    self.act(Action::Stop(view.id.clone()));
                }
                let paused = view.badge == Badge::Paused;
                if (view.badge == Badge::Running || paused)
                    && ui
                        .button(if paused { "Resume" } else { "Pause" })
                        .on_hover_text("Takes effect between turns")
                        .clicked()
                {
                    self.act(if paused {
                        Action::Resume(view.id.clone())
                    } else {
                        Action::Pause(view.id.clone())
                    });
                }
            });
        });
        ui.horizontal(|ui| {
            theme::cost_tag(ui, false);
            let spent = dollars(view.api);
            let limit = self
                .profile
                .as_ref()
                .ok()
                .and_then(|profile| profile.budgets.per_ticket_microusd);
            ui.label(match limit {
                Some(limit) => format!(
                    "{spent} of {}",
                    dollars(autopilot_core::Money::from_microusd(limit.get()))
                ),
                None => spent,
            });
            ui.add_space(10.0);
            theme::cost_tag(ui, true);
            ui.label(format!("{} estimated", dollars(view.subscription)));
        });
    }

    /// The one thing to know or do right now.
    fn next_card(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        if view.asking.is_some() {
            self.waiting_card(ui, view);
            return;
        }
        let problem = view
            .steps
            .iter()
            .find(|card| matches!(card.state, StepState::Failed(_) | StepState::Interrupted(_)));
        let (fill, color, text) = if let Some(card) = problem {
            (
                theme::RED_FILL(),
                theme::RED(),
                format!("{} {}", step_name(card.step), state_text(&card.state)),
            )
        } else if view.badge == Badge::Done {
            (
                theme::GREEN_FILL(),
                theme::GREEN(),
                "All steps are done.".to_owned(),
            )
        } else if view.badge == Badge::Paused {
            (
                theme::GREY_FILL(),
                theme::GREY_TEXT(),
                "Paused. Press Resume to carry on.".to_owned(),
            )
        } else if let Some(card) = view.steps.iter().find(|card| card.state.is_active()) {
            (
                theme::BLUE_FILL(),
                theme::BLUE_TEXT(),
                format!("Working on {}.", step_name(card.step)),
            )
        } else if ready_to_plan(view) {
            (
                theme::BLUE_FILL(),
                theme::BLUE_TEXT(),
                "Ready to make a plan.".to_owned(),
            )
        } else {
            return;
        };
        Frame::new()
            .fill(fill)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 38.0);
                ui.label(RichText::new(text).size(16.0).strong().color(color));
                if problem.is_none() && ready_to_plan(view) {
                    let busy = self.busy;
                    if ui
                        .add_enabled(!busy, theme::primary("Make the plan"))
                        .on_hover_text("The agent writes the plan. You approve it before anything is built.")
                        .clicked()
                    {
                        self.act(Action::MakePlan(view.id.clone()));
                    }
                }
                if let Some(card) = problem {
                    if matches!(card.step, Step::Understand | Step::Plan) {
                        if ui.add(theme::primary("Try again")).clicked() {
                            self.act(if card.step == Step::Understand {
                                Action::RerunUnderstand(view.id.clone())
                            } else {
                                Action::RerunPlan(view.id.clone())
                            });
                        }
                    } else {
                        ui.add_enabled(false, egui::Button::new("Try again"))
                            .on_disabled_hover_text(
                                "coming soon: this step cannot run until code can be written safely (S6)",
                            );
                        ui.label(theme::muted("Try again is coming soon for this step."));
                    }
                }
            });
        ui.add_space(14.0);
    }

    /// The six steps as one row of chips. Click one for its details.
    fn step_chips(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        ui.horizontal_wrapped(|ui| {
            for card in &view.steps {
                let (mark, color) = glyph(&card.state);
                let active = card.state.is_active();
                let skipped = matches!(card.state, StepState::Skipped(_));
                let name = RichText::new(format!("{mark}  {}", step_name(card.step))).size(15.0);
                let chip = egui::Button::new(if skipped {
                    name.weak()
                } else {
                    name.color(color)
                })
                .fill(if active {
                    theme::BLUE_FILL()
                } else {
                    theme::CARD()
                })
                .stroke(Stroke::new(
                    if active { 2.0_f32 } else { 1.0_f32 },
                    if active {
                        theme::BLUE()
                    } else {
                        theme::BORDER()
                    },
                ))
                .min_size(egui::vec2(0.0, 38.0));
                let detail = card
                    .in_short
                    .clone()
                    .unwrap_or_else(|| state_text(&card.state));
                if ui.add(chip).on_hover_text(detail).clicked() {
                    self.screen = Screen::Step(card.step);
                }
            }
        });
    }

    fn waiting_card(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let Some((step, request)) = &view.asking else {
            return;
        };
        let text = waiting_text(&self.options, request)
            .unwrap_or_else(|| "This step is waiting for you.".into());
        let question = text.contains('?');
        Frame::new()
            .fill(theme::AMBER_FILL())
            .stroke(Stroke::new(1.0_f32, theme::AMBER()))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 38.0);
                ui.label(
                    RichText::new(format!("{} needs you", step_name(*step)))
                        .size(16.0)
                        .strong()
                        .color(theme::AMBER()),
                );
                ui.label(text);
                ui.horizontal(|ui| {
                    if question {
                        if ui
                            .add(theme::primary("Use the recommended answers"))
                            .clicked()
                        {
                            self.act(Action::AcceptDefaults {
                                ticket: view.id.clone(),
                            });
                        }
                    } else {
                        if ui.add(theme::primary("Approve")).clicked() {
                            self.act(Action::Decide {
                                ticket: view.id.clone(),
                                approve: true,
                            });
                        }
                        if ui.button("Reject").clicked() {
                            self.act(Action::Decide {
                                ticket: view.id.clone(),
                                approve: false,
                            });
                        }
                    }
                });
            });
        ui.add_space(10.0);
    }

    #[allow(clippy::too_many_lines, reason = "one panel, top to bottom")]
    fn build_panel(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let Some(build) = view.step(Step::BuildCheck) else {
            return;
        };
        if matches!(build.state, StepState::Idle) && view.rounds.is_empty() {
            return;
        }
        let running = build.state == StepState::Running;
        let border = if running {
            theme::BLUE()
        } else {
            theme::BORDER()
        };
        theme::card(theme::CARD(), border).show(ui, |ui| {
            ui.set_width(ui.available_width() - 38.0);
            let latest = view.rounds.last();
            let limit = self
                .profile
                .as_ref()
                .ok()
                .and_then(|profile| profile.step(Step::BuildCheck))
                .map(|settings| settings.limits.rounds);
            let title = match (latest, limit) {
                (Some(round), Some(limit)) => {
                    format!("Build ⟲ Check · round {} of {limit}", round.round)
                }
                _ => "Build ⟲ Check".to_owned(),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(19.0).strong());
                theme::pill(
                    ui,
                    &state_text(&build.state),
                    theme::BLUE_FILL(),
                    theme::BLUE_TEXT(),
                );
            });
            if let Some(text) = &build.in_short {
                ui.label(format!("In short: {text}"));
                ui.label(theme::muted(
                    "Made from the run's results by code, so it costs nothing.",
                ));
            }
            ui.add_space(4.0);
            ui.label(theme::muted(
                "Checks, cheapest first. The run stops at the first failure.",
            ));
            if let Some(round) = latest {
                for check in &round.checks {
                    let (mark, color) = if check.passed {
                        ("✔", theme::GREEN())
                    } else {
                        ("×", theme::RED())
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(mark).color(color));
                        ui.label(&check.name);
                    });
                }
                let waiting: Vec<&String> = self
                    .profile
                    .as_ref()
                    .map(|profile| profile.checks.keys().collect())
                    .unwrap_or_default();
                if running {
                    for name in waiting
                        .into_iter()
                        .filter(|name| !round.checks.iter().any(|check| &&check.name == name))
                    {
                        ui.label(theme::muted(format!("○ {name}   waiting")));
                    }
                }
            }
            if let Some(failed) = view
                .rounds
                .iter()
                .rev()
                .skip(1)
                .find_map(|round| round.failure().map(|check| (round.round, check.clone())))
            {
                Frame::new()
                    .fill(theme::AMBER_FILL())
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!(
                                "Round {} failed: {}. The note went back to the build.",
                                failed.0, failed.1.note
                            ))
                            .color(theme::AMBER()),
                        );
                    });
            }
            ui.horizontal(|ui| {
                theme::cost_tag(ui, true);
                ui.label(theme::muted(format!(
                    "est {}   ·   turns, tokens and cache are not recorded yet",
                    dollars(build.subscription)
                )));
            });
        });
    }

    /// "Ask for a change": the words are read by a fixed grammar, shown as a
    /// diff, and applied only when the owner says so.
    pub(crate) fn change_box(&mut self, ui: &mut egui::Ui) {
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 38.0);
            let edit = ui.add(
                egui::TextEdit::multiline(&mut self.change_text)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("Ask for a change, e.g. \"use opus for the plan\" or \"allow only 2 refine rounds\""),
            );
            let send = edit.has_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter) && input.modifiers.command);
            ui.horizontal(|ui| {
                let ready = !self.change_text.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new("Propose")).clicked() || (send && ready) {
                    self.reading = Some(match &self.profile {
                        Ok(profile) => autopilot_engine::propose(&self.change_text, profile),
                        Err(problem) => Err(format!("the profile cannot be read: {problem}")),
                    });
                }
                ui.label(theme::muted("Ctrl+Enter to propose. Nothing changes until you apply."));
            });
            self.change_preview(ui);
        });
    }

    fn change_preview(&mut self, ui: &mut egui::Ui) {
        let Some(reading) = self.reading.clone() else {
            return;
        };
        let reading = match reading {
            Err(why) => {
                super::app::banner(ui, &why, false);
                return;
            }
            Ok(reading) => reading,
        };
        let ticket = self.current().map(|view| view.id.clone());
        let one_ticket_ok = ticket.is_some()
            && reading
                .proposals
                .iter()
                .all(autopilot_engine::Proposal::fits_one_ticket);
        if !one_ticket_ok {
            self.for_project = true;
        }
        Frame::new()
            .stroke(Stroke::new(1.0_f32, theme::BLUE()))
            .fill(theme::BLUE_FILL())
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.label(RichText::new("Proposed change. Nothing is applied until you say so.").strong());
                for proposal in &reading.proposals {
                    ui.label(theme::mono(proposal.describe()));
                }
                for note in &reading.notes {
                    ui.label(RichText::new(note).color(theme::AMBER()));
                }
                ui.horizontal(|ui| {
                    ui.label(theme::muted("Applies to"));
                    ui.add_enabled_ui(one_ticket_ok, |ui| {
                        theme::segmented(
                            ui,
                            &mut self.for_project,
                            &[(false, "This ticket only"), (true, "All future tickets")],
                        );
                    });
                    if !one_ticket_ok {
                        ui.label(theme::muted("(turns and rounds are project settings)"));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add(theme::primary("Apply")).clicked() {
                        self.apply_change(&reading, ticket.as_ref());
                    }
                    if ui.button("Discard").clicked() {
                        self.reading = None;
                    }
                });
                ui.label(theme::muted(
                    "Takes effect when the next step starts; the step running now is not touched. A spending limit cannot be loosened from here.",
                ));
            });
    }

    fn apply_change(&mut self, reading: &autopilot_engine::Reading, ticket: Option<&TicketId>) {
        let outcome = if self.for_project || ticket.is_none() {
            autopilot_engine::apply_to_project(&self.options.profile, &reading.proposals)
                .map(|profile| {
                    self.profile = Ok(profile);
                    "Saved in profile.toml for all future tickets".to_owned()
                })
                .map_err(|error| error.to_string())
        } else {
            let id = ticket.cloned().unwrap_or_else(|| TicketId::from(""));
            autopilot_engine::apply_to_ticket(&self.options.data_dir, &id, &reading.proposals)
                .map(|()| "Recorded for this ticket".to_owned())
                .map_err(|error| error.to_string())
        };
        if outcome.is_ok() {
            self.change_text.clear();
            self.reading = None;
        }
        self.notice = Some(match outcome {
            Ok(text) => (true, text),
            Err(text) => (false, text),
        });
    }

    pub(crate) fn side_panel(&mut self, ctx: &egui::Context) {
        let Some(view) = self.current().cloned() else {
            return;
        };
        egui::SidePanel::right("details")
            .exact_width(380.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .inner_margin(Margin::same(24)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Screen::Step(step) = self.screen {
                        self.step_side(ui, &view, step);
                    }
                });
            });
    }
}
