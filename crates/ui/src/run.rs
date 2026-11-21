//! "Watch a run": the live strip, the Build and Check panel, the timeline and
//! the panel on the right with progress, spend and actions.

use autopilot_core::{Step, TicketId};
use autopilot_engine::{Badge, TicketView, dollars, step_name};
use autopilot_pipeline::StepState;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke};

use crate::app::{App, Mode, Screen, badge_colors};
use crate::backend::{Action, waiting_text};
use crate::theme;

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
        self.waiting_card(ui, &view);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 34.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Pipeline · live").size(17.0).strong());
                ui.label(theme::muted(format!(
                    "click a step for details · started {} UTC",
                    clock(view.started_ms)
                )));
            });
            ui.add_space(6.0);
            // Two rows of three when the window is narrow.
            let per_row = if ui.available_width() < 900.0 { 3 } else { 6 };
            for chunk in Step::ALL.chunks(per_row) {
                ui.columns(per_row, |columns| {
                    for (index, step) in chunk.iter().enumerate() {
                        if let Some(card) = view.step(*step)
                            && columns[index].vertical(|ui| strip_card(ui, card)).inner
                        {
                            self.screen = Screen::Step(*step);
                        }
                    }
                });
            }
        });
        ui.add_space(10.0);
        self.build_panel(ui, &view);
        ui.add_space(10.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 34.0);
            ui.label(theme::muted("Timeline (UTC)"));
            for line in &view.timeline {
                ui.label(theme::mono(format!(
                    "{}   {}",
                    clock(line.at_ms),
                    line.text
                )));
            }
        });
        ui.add_space(10.0);
        self.change_box(ui);
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
                ui.set_width(ui.available_width() - 34.0);
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
            ui.set_width(ui.available_width() - 34.0);
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
            ui.set_width(ui.available_width() - 34.0);
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
                    if self.screen == Screen::Run {
                        self.run_side(ui, &view);
                    } else if let Screen::Step(step) = self.screen {
                        self.step_side(ui, &view, step);
                    }
                });
            });
    }

    fn run_side(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&view.title).size(21.0).strong());
        });
        let (text, fill, color) = badge_colors(view.badge, view);
        theme::pill(ui, &text, fill, color);
        ui.add_space(8.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 34.0);
            ui.label(theme::muted("Progress"));
            for step in &view.steps {
                let (mark, color) = glyph(&step.state);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(mark).color(color));
                    let name = RichText::new(step_name(step.step)).size(16.0);
                    ui.label(if step.state.is_active() {
                        name.strong()
                    } else {
                        name
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::muted(state_text(&step.state)));
                    });
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                theme::cost_tag(ui, false);
                ui.label(theme::muted("spent"));
                let limit = self
                    .profile
                    .as_ref()
                    .ok()
                    .and_then(|profile| profile.budgets.per_ticket_microusd);
                let spent = dollars(view.api);
                ui.label(match limit {
                    Some(limit) => format!(
                        "{spent} of {}",
                        dollars(autopilot_core::Money::from_microusd(limit.get()))
                    ),
                    None => spent,
                });
            });
            ui.horizontal(|ui| {
                theme::cost_tag(ui, true);
                ui.label(theme::muted("estimated"));
                ui.label(dollars(view.subscription));
            });
        });
        self.other_needs_you(ui, view);
        self.ticket_actions(ui, view);
    }

    fn other_needs_you(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let other = self
            .snapshot
            .tickets
            .iter()
            .find(|other| other.id != view.id && other.badge == Badge::NeedsYou)
            .map(|other| (other.id.clone(), other.title.clone()));
        if let Some((id, title)) = other {
            ui.add_space(8.0);
            Frame::new()
                .fill(theme::AMBER_FILL())
                .stroke(Stroke::new(1.0_f32, theme::AMBER()))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 34.0);
                    ui.label(
                        RichText::new("Another ticket needs you")
                            .strong()
                            .color(theme::AMBER()),
                    );
                    ui.label(format!("{title}: waiting for your answer."));
                    if ui.add(theme::primary("Open it")).clicked() {
                        self.open(&id);
                    }
                });
        }
    }

    pub(crate) fn ticket_actions(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        ui.add_space(8.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 34.0);
            let off = |ui: &mut egui::Ui, text: &str, why: &str| {
                ui.add_enabled(
                    false,
                    egui::Button::new(text).min_size(egui::vec2(ui.available_width(), 36.0)),
                )
                .on_disabled_hover_text(why);
            };
            off(
                ui,
                "Open worktree",
                "Not built yet: the window cannot open a folder or a terminal",
            );
            off(
                ui,
                "Open in terminal (resume)",
                "Not built yet: it needs the real agent session (S6)",
            );
            let paused = view.badge == Badge::Paused;
            let label = if paused { "Resume" } else { "Pause" };
            if ui
                .add(egui::Button::new(label).min_size(egui::vec2(ui.available_width(), 36.0)))
                .on_hover_text("Takes effect between turns")
                .clicked()
            {
                self.act(if paused {
                    Action::Resume(view.id.clone())
                } else {
                    Action::Pause(view.id.clone())
                });
            }
            let stop = egui::Button::new(RichText::new("Stop the ticket").color(theme::RED()))
                .min_size(egui::vec2(ui.available_width(), 36.0));
            if ui
                .add(stop)
                .on_hover_text("Stops between turns. Deleting a ticket is not built yet.")
                .clicked()
            {
                self.act(Action::Stop(view.id.clone()));
            }
            if ui.button("Back to the pipeline design").clicked() {
                self.mode = Mode::Design;
            }
        });
    }
}

fn strip_card(ui: &mut egui::Ui, card: &autopilot_engine::StepView) -> bool {
    let skipped = matches!(card.state, StepState::Skipped(_));
    let active = card.state.is_active();
    let (fill, border, width) = if active {
        (theme::BLUE_FILL(), theme::BLUE(), 2.0_f32)
    } else if skipped || matches!(card.state, StepState::Idle) {
        (Color32::TRANSPARENT, theme::BORDER_STRONG(), 1.0_f32)
    } else {
        (theme::NODE(), theme::BORDER(), 1.0_f32)
    };
    let (mark, color) = glyph(&card.state);
    let response = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(width, border))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_min_height(86.0);
            ui.set_width(ui.available_width() - 34.0);
            ui.label(
                RichText::new(format!("{mark} {}", step_name(card.step)))
                    .size(15.0)
                    .strong()
                    .color(if skipped {
                        theme::TEXT_MUTED()
                    } else {
                        theme::TEXT()
                    }),
            );
            let detail = card
                .in_short
                .clone()
                .unwrap_or_else(|| state_text(&card.state));
            ui.label(RichText::new(detail).size(12.5).color(if active {
                theme::BLUE_TEXT()
            } else {
                theme::TEXT_SOFT()
            }));
            let _ = color;
        })
        .response
        .interact(egui::Sense::click());
    response.clicked()
}
