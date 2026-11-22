//! Today: every decision across tickets in one queue, with one detail pane.
//! Keys: J and K move, A approves, R rejects, E opens the ticket.

use autopilot_core::{Step, TicketId};
use autopilot_engine::{Badge, TicketView, step_name};
use autopilot_pipeline::StepState;
use eframe::egui::{self, Frame, Margin, RichText, Stroke};

use crate::app::{App, Mode, badge_colors};
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
        ui.add_sized(
            [120.0, 18.0],
            egui::Label::new(theme::muted(key)).halign(egui::Align::LEFT),
        );
        ui.label(value);
    });
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
        ui.label(RichText::new("Today").size(30.0).strong());
        let waiting: Vec<TicketView> = queue(&self.snapshot.tickets).into_iter().cloned().collect();
        if waiting.is_empty() {
            ui.label(theme::muted("Nothing waits for you."));
        } else {
            ui.label(theme::muted("J/K move   A approve   R reject   E open"));
        }
        ui.add_space(10.0);
        let picked = self.today_pick();
        let mut select = None;
        for view in &waiting {
            let chosen = picked.as_ref() == Some(&view.id);
            let button = egui::Button::new(RichText::new(&view.title).size(15.0))
                .fill(if chosen {
                    theme::SELECTED()
                } else {
                    egui::Color32::TRANSPARENT
                })
                .min_size(egui::vec2(ui.available_width(), 38.0));
            if theme::named(ui.add(button), &view.title).clicked() {
                select = Some(view.id.clone());
            }
        }
        if select.is_some() {
            self.today_selected = select;
        }
        if let Some(view) = picked.and_then(|id| self.ticket(&id).cloned()) {
            ui.add_space(12.0);
            self.today_detail(ui, &view);
        }
        self.working_list(ui);
    }

    fn today_detail(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let waiting = view.asking.as_ref().and_then(|(step, request)| {
            backend::waiting_request(&self.options, request).map(|waiting| (*step, waiting))
        });
        Frame::new()
            .fill(theme::CARD())
            .stroke(Stroke::new(1.0_f32, theme::BORDER()))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 34.0);
                ui.label(RichText::new(&view.title).size(18.0).strong());
                let (text, fill, color) = badge_colors(view.badge, view);
                theme::pill(ui, &text, fill, color);
                ui.add_space(8.0);
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
        ui.add_space(18.0);
        ui.label(RichText::new("Working on its own").size(18.0).strong());
        for view in running {
            ui.horizontal(|ui| {
                if ui.link(&view.title).clicked() {
                    self.open(&view.id);
                }
                let step = view.active().map_or("", step_name);
                ui.label(theme::muted(step));
                ui.label(theme::mono(round_strip(&view)));
            });
        }
    }
}
