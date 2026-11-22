//! Today: every decision across tickets in one queue, with one detail pane.
//! Keys: J and K move, A approves, R rejects, E opens the ticket.

use autopilot_core::{Step, TicketId};
use autopilot_engine::{Badge, TicketView, step_name};
use autopilot_pipeline::StepState;
use eframe::egui::{self, Frame, Margin, RichText, Stroke};

use crate::app::{App, Mode};
use crate::backend::{self, Action};
use crate::run::state_text;
use crate::theme;

/// A key pressed on the Today page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TodayKey {
    Next,
    Previous,
    Approve,
    Reject,
    Open,
}

/// The tickets that need a decision: asking first, then stopped ones, each
/// group oldest first so nothing waits behind newer work.
#[must_use]
pub fn queue(tickets: &[TicketView]) -> Vec<&TicketView> {
    let rank = |view: &TicketView| match view.badge {
        Badge::NeedsYou => Some(0),
        Badge::Failed => Some(1),
        Badge::Interrupted => Some(2),
        _ => None,
    };
    let mut waiting: Vec<(i32, &TicketView)> = tickets
        .iter()
        .filter_map(|view| rank(view).map(|rank| (rank, view)))
        .collect();
    waiting.sort_by_key(|(rank, view)| (*rank, view.last_ms));
    waiting.into_iter().map(|(_, view)| view).collect()
}

/// Steps that have not run yet, in order, for "what happens next".
fn next_steps(view: &TicketView) -> Vec<&'static str> {
    view.steps
        .iter()
        .filter(|card| matches!(card.state, StepState::Idle))
        .map(|card| step_name(card.step))
        .collect()
}

fn row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        theme::cell(
            ui,
            120.0,
            RichText::new(key.to_uppercase())
                .monospace()
                .size(11.0)
                .strong()
                .color(theme::TEXT_MUTED()),
        );
        ui.label(value);
    });
}

/// A small mono caption, like the canvas `lbl`.
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .monospace()
            .size(11.0)
            .strong()
            .color(theme::TEXT_MUTED()),
    );
}

/// What a waiting ticket is called in the queue and in what colours.
fn kind_chip(view: &TicketView, question: bool) -> (&'static str, egui::Color32, egui::Color32) {
    match view.badge {
        Badge::NeedsYou if question => ("question", theme::AMBER_FILL(), theme::AMBER()),
        Badge::NeedsYou => ("approve", theme::AMBER_FILL(), theme::AMBER()),
        Badge::Interrupted => ("interrupted", theme::RED_FILL(), theme::RED()),
        _ => ("failed", theme::RED_FILL(), theme::RED()),
    }
}

fn age_text(last_ms: u64) -> String {
    let minutes = crate::now_ms().saturating_sub(last_ms) / 60_000;
    match minutes {
        0 => "now".to_owned(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

fn project_of(view: &TicketView) -> String {
    crate::nav::project_name(view).unwrap_or_else(|| "no project".to_owned())
}

fn chip(ui: &mut egui::Ui, text: &str, fill: egui::Color32, color: egui::Color32) {
    theme::pill(ui, text, fill, color);
}

fn round_strip(view: &TicketView) -> String {
    view.rounds
        .iter()
        .map(|round| {
            let mark = if round.failure().is_some() {
                "✗"
            } else {
                "✔"
            };
            format!("R{} {mark}", round.round)
        })
        .collect::<Vec<_>>()
        .join("  ")
}

impl App {
    /// The ticket selected in the queue, or the first one when the selection
    /// is gone.
    fn today_pick(&self) -> Option<TicketId> {
        let waiting = queue(&self.snapshot.tickets);
        self.today_selected
            .as_ref()
            .filter(|id| waiting.iter().any(|view| &view.id == *id))
            .cloned()
            .or_else(|| waiting.first().map(|view| view.id.clone()))
    }

    pub(crate) fn today_count(&self) -> usize {
        queue(&self.snapshot.tickets).len()
    }

    /// Read the Today keys for this frame. Text fields keep their own keys.
    pub(crate) fn today_keys(&mut self, ctx: &egui::Context) {
        if self.mode != Mode::Today || ctx.wants_keyboard_input() {
            return;
        }
        let pressed = |key| ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
        let key = if pressed(egui::Key::J) {
            TodayKey::Next
        } else if pressed(egui::Key::K) {
            TodayKey::Previous
        } else if pressed(egui::Key::A) {
            TodayKey::Approve
        } else if pressed(egui::Key::R) {
            TodayKey::Reject
        } else if pressed(egui::Key::E) {
            TodayKey::Open
        } else {
            return;
        };
        self.today_key(key);
    }

    pub(crate) fn today_key(&mut self, key: TodayKey) {
        let waiting: Vec<TicketId> = queue(&self.snapshot.tickets)
            .iter()
            .map(|view| view.id.clone())
            .collect();
        let Some(current) = self.today_pick() else {
            return;
        };
        let at = waiting.iter().position(|id| *id == current).unwrap_or(0);
        match key {
            TodayKey::Next => {
                self.today_selected = waiting.get(at + 1).or(waiting.last()).cloned();
            }
            TodayKey::Previous => {
                self.today_selected = waiting.get(at.saturating_sub(1)).cloned();
            }
            TodayKey::Approve | TodayKey::Reject => self.decide(&current, key == TodayKey::Approve),
            TodayKey::Open => self.open(&current),
        }
    }

    /// Approve or reject what the ticket asks. A question has no approval:
    /// A takes the recommended answers instead.
    fn decide(&mut self, id: &TicketId, approve: bool) {
        let Some(view) = self.ticket(id).cloned() else {
            return;
        };
        let Some((_, request)) = &view.asking else {
            return;
        };
        let Some(waiting) = backend::waiting_request(&self.options, request) else {
            return;
        };
        if waiting.question {
            if approve {
                self.act(Action::AcceptDefaults { ticket: id.clone() });
            }
            return;
        }
        self.act(Action::Decide {
            ticket: id.clone(),
            approve,
            always: self.always && !waiting.forced,
        });
    }

    pub(crate) fn today_screen(&mut self, ui: &mut egui::Ui) {
        let waiting: Vec<TicketView> = queue(&self.snapshot.tickets).into_iter().cloned().collect();
        let running = self
            .snapshot
            .tickets
            .iter()
            .filter(|view| view.badge == Badge::Running)
            .count();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Inbox").size(18.0).strong());
            chip(
                ui,
                &format!("{} waiting", waiting.len()),
                theme::AMBER_FILL(),
                theme::AMBER(),
            );
            chip(
                ui,
                &format!("{running} running"),
                theme::BLUE_FILL(),
                theme::BLUE_TEXT(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new("j k nav   a approve   e note   r reject")
                        .monospace()
                        .size(11.0)
                        .color(theme::TEXT_MUTED()),
                );
            });
        });
        if waiting.is_empty() {
            ui.label(theme::muted("Nothing waits for you."));
        }
        ui.add_space(8.0);
        let picked = self.today_pick();
        let mut select = None;
        let total = ui.available_width();
        let queue_width = (total * 0.34).clamp(260.0, 380.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.allocate_ui_with_layout(
                egui::vec2(queue_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(queue_width);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for view in &waiting {
                        let chosen = picked.as_ref() == Some(&view.id);
                        if self.queue_card(ui, view, chosen) {
                            select = Some(view.id.clone());
                        }
                    }
                    self.working_list(ui);
                },
            );
            ui.vertical(|ui| {
                if let Some(view) = picked.as_ref().and_then(|id| self.ticket(id).cloned()) {
                    self.today_detail(ui, &view);
                }
            });
        });
        if select.is_some() {
            self.today_selected = select;
        }
        self.health_items(ui);
    }

    /// One waiting ticket in the queue. Returns true when it was clicked.
    fn queue_card(&self, ui: &mut egui::Ui, view: &TicketView, chosen: bool) -> bool {
        let question = view
            .asking
            .as_ref()
            .and_then(|(_, request)| backend::waiting_request(&self.options, request))
            .is_some_and(|waiting| waiting.question);
        let (kind, fill, color) = kind_chip(view, question);
        let step = view
            .asking
            .as_ref()
            .map(|(step, _)| step_name(*step))
            .or_else(|| {
                view.steps
                    .iter()
                    .find(|card| {
                        matches!(card.state, StepState::Failed(_) | StepState::Interrupted(_))
                    })
                    .map(|card| step_name(card.step))
            })
            .unwrap_or("");
        let shown = Frame::new()
            .fill(if chosen {
                theme::CARD()
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0_f32,
                if chosen {
                    theme::AMBER()
                } else {
                    theme::BORDER()
                },
            ))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    chip(ui, kind, fill, color);
                    ui.label(theme::mono(step).size(11.0).color(theme::TEXT_MUTED()));
                    ui.label(
                        theme::mono(format!("· {}", project_of(view)))
                            .size(11.0)
                            .color(theme::TEXT_MUTED()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            theme::mono(age_text(view.last_ms))
                                .size(11.0)
                                .color(theme::TEXT_MUTED()),
                        );
                    });
                });
                ui.label(
                    RichText::new(&view.title)
                        .monospace()
                        .size(13.0)
                        .strong()
                        .color(theme::TEXT()),
                );
            });
        let click = ui.interact(
            shown.response.rect,
            ui.id().with(("queue", view.id.as_str())),
            egui::Sense::click(),
        );
        theme::named(click, &view.title).clicked()
    }

    /// Open review findings, each linking to Health. Nothing shows when the
    /// review is off or has found nothing.
    fn health_items(&mut self, ui: &mut egui::Ui) {
        if self.findings.is_empty() && self.tunings.is_empty() {
            return;
        }
        ui.add_space(18.0);
        ui.label(RichText::new("Pipeline health").size(14.0).strong());
        let mut open = None;
        let mut undo = None;
        for each in &self.tunings {
            let tuning = &each.tuning;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tuned").color(theme::GREEN()));
                if ui
                    .link(format!(
                        "{} · {}: effort {} to {}",
                        tuning.pipeline,
                        autopilot_engine::step_name(tuning.step),
                        tuning.effort_before.as_deref().unwrap_or("unset"),
                        tuning.effort_after
                    ))
                    .clicked()
                {
                    open = Some(tuning.pipeline.clone());
                }
                if ui.button("Undo").clicked() {
                    undo = Some((tuning.pipeline.clone(), tuning.step));
                }
            });
        }
        if let Some((pipeline, step)) = undo {
            self.notice = Some(
                match autopilot_engine::undo_tuning(&self.options.data_dir, &pipeline, step) {
                    Ok(_) => {
                        self.tunings = autopilot_engine::active_tunings(&self.options.data_dir)
                            .unwrap_or_default();
                        (true, "The tuning is undone.".to_owned())
                    }
                    Err(error) => (false, format!("Could not undo: {error}")),
                },
            );
        }
        for finding in &self.findings {
            ui.horizontal(|ui| {
                if ui
                    .link(format!(
                        "{} · {}",
                        finding.pipeline,
                        autopilot_engine::step_name(finding.step)
                    ))
                    .clicked()
                {
                    open = Some(finding.pipeline.clone());
                }
                ui.label(theme::muted(&finding.evidence));
            });
        }
        if let Some(name) = open {
            self.health_selected = Some(name);
            self.mode = Mode::Health;
        }
    }

    fn today_detail(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let waiting = view.asking.as_ref().and_then(|(step, request)| {
            backend::waiting_request(&self.options, request).map(|waiting| (*step, waiting))
        });
        let question = waiting
            .as_ref()
            .is_some_and(|(_, waiting)| waiting.question);
        let (kind, fill, color) = kind_chip(view, question);
        let border = if view.badge == Badge::NeedsYou {
            theme::AMBER()
        } else {
            theme::RED()
        };
        Frame::new()
            .fill(theme::CARD())
            .stroke(Stroke::new(1.0_f32, border))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::symmetric(18, 16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    chip(ui, kind, fill, color);
                    ui.label(RichText::new(&view.title).monospace().size(14.0).strong());
                    if let Some((step, _)) = &waiting {
                        ui.label(theme::mono(step_name(*step)).color(theme::TEXT_MUTED()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.link("open →").clicked() {
                            self.open(&view.id);
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                match waiting {
                    Some((step, waiting)) => self.approval_rows(ui, view, step, &waiting),
                    None => self.stopped_rows(ui, view),
                }
            });
    }

    fn approval_rows(
        &mut self,
        ui: &mut egui::Ui,
        view: &TicketView,
        step: Step,
        waiting: &backend::Waiting,
    ) {
        let kind = if waiting.question {
            "question"
        } else {
            "approval"
        };
        row(ui, "waiting on", &format!("{} ({kind})", step_name(step)));
        row(ui, "request", &waiting.text);
        if let Some(short) = view.step(step).and_then(|card| card.in_short.as_deref()) {
            row(ui, "in short", short);
        }
        if !view.attachments.is_empty() {
            let names: Vec<&str> = view
                .attachments
                .iter()
                .map(|file| file.name.as_str())
                .collect();
            row(ui, "files", &names.join(", "));
        }
        let next = next_steps(view);
        if !next.is_empty() {
            row(ui, "then", &next.join(" > "));
        }
        ui.add_space(8.0);
        if waiting.forced {
            ui.label(theme::muted(
                "Forced: protected paths, risk or a budget. It cannot be switched off.",
            ));
        } else if !waiting.question && step != Step::Plan {
            ui.checkbox(&mut self.always, "Don't ask after this step");
        }
        ui.horizontal(|ui| {
            if waiting.question {
                if ui
                    .add(theme::primary("Use recommended answers (A)"))
                    .clicked()
                {
                    self.decide(&view.id, true);
                }
            } else {
                if ui.add(theme::primary("Approve (A)")).clicked() {
                    self.decide(&view.id, true);
                }
                if ui.button("Reject (R)").clicked() {
                    self.decide(&view.id, false);
                }
            }
            if ui.button("Open (E)").clicked() {
                self.open(&view.id);
            }
        });
    }

    fn stopped_rows(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let stopped = view
            .steps
            .iter()
            .find(|card| matches!(card.state, StepState::Failed(_) | StepState::Interrupted(_)));
        if let Some(card) = stopped {
            row(ui, "stopped at", step_name(card.step));
            row(ui, "why", &state_text(&card.state));
        }
        let hit_turns = stopped.is_some_and(|card| {
            card.step == Step::Understand && state_text(&card.state).contains("max_turns")
        });
        if hit_turns
            && ui
                .button("Try again with more turns")
                .on_hover_text("Doubles the turn cap for this ticket only")
                .clicked()
        {
            self.act(Action::RerunUnderstandWithMoreTurns(view.id.clone()));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if stopped.is_some_and(|card| card.step == Step::Understand)
                && ui.add(theme::primary("Try again")).clicked()
            {
                self.act(Action::RerunUnderstand(view.id.clone()));
            }
            if ui.button("Open (E)").clicked() {
                self.open(&view.id);
            }
        });
    }

    fn working_list(&mut self, ui: &mut egui::Ui) {
        let running: Vec<TicketView> = self
            .snapshot
            .tickets
            .iter()
            .filter(|view| view.badge == Badge::Running)
            .cloned()
            .collect();
        if running.is_empty() {
            return;
        }
        ui.add_space(8.0);
        caption(ui, "Running");
        for view in running {
            let shown = Frame::new()
                .fill(theme::CARD())
                .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&view.title).monospace().strong());
                        let step = view.active().map_or("", step_name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(theme::mono(step).size(11.0).color(theme::BLUE_TEXT()));
                        });
                    });
                    let steps = u16::try_from(view.steps.len()).unwrap_or(6).max(1);
                    let done = view
                        .steps
                        .iter()
                        .filter(|card| {
                            matches!(card.state, StepState::Passed | StepState::Skipped(_))
                        })
                        .count();
                    let count = f32::from(steps);
                    let width = (ui.available_width() - 3.0 * (count - 1.0)) / count;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for index in 0..steps {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(width.max(4.0), 4.0),
                                egui::Sense::hover(),
                            );
                            let color = match usize::from(index).cmp(&done) {
                                std::cmp::Ordering::Less => theme::GREEN(),
                                std::cmp::Ordering::Equal => theme::BLUE(),
                                std::cmp::Ordering::Greater => theme::BORDER(),
                            };
                            ui.painter()
                                .rect_filled(rect, egui::CornerRadius::same(2), color);
                        }
                    });
                    let rounds = round_strip(&view);
                    if !rounds.is_empty() {
                        ui.label(theme::mono(rounds).size(11.0).color(theme::TEXT_MUTED()));
                    }
                });
            let click = ui.interact(
                shown.response.rect,
                ui.id().with(("running", view.id.as_str())),
                egui::Sense::click(),
            );
            if theme::named(click, &view.title).clicked() {
                self.open(&view.id);
            }
        }
    }
}
