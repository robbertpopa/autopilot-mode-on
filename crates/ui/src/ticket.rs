//! The ticket page: one story, newest last, a composer under it, and a right
//! panel with the vertical stepper. A click on a story entry or a step opens
//! the drawer (`step.rs`) in the same panel.

use autopilot_core::{Step, TicketId};
use autopilot_engine::{TicketView, dollars, step_name};
use autopilot_pipeline::StepState;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Shape, Stroke};

use crate::app::{App, Screen};
use crate::run::state_text;
use crate::theme;

const CODE_WHY: &str = "coming soon: changing code needs real Build runs (S6)";
const ASK_WHY: &str = "coming soon: a side question to the agent is not built yet";
const UNSURE_WHY: &str = "needs request R3: a per-ticket when_unsure setting";
const PIPELINE_WHY: &str = "needs request R7: named pipelines per ticket";

/// The mark and colour for a step in the stepper and the story.
fn mark(state: &StepState) -> (&'static str, Color32) {
    match state {
        StepState::Passed => ("✔", theme::GREEN()),
        StepState::Running => ("▶", theme::BLUE_TEXT()),
        StepState::Asking(_) => ("?", theme::AMBER()),
        StepState::Failed(_) | StepState::Interrupted(_) => ("×", theme::RED()),
        StepState::Paused => ("⏸", theme::GREY_TEXT()),
        StepState::Skipped(_) | StepState::Idle => ("-", theme::TEXT_MUTED()),
    }
}

/// A dashed outline, because a skipped step is drawn dashed with its reason.
fn dashed(ui: &egui::Ui, rect: egui::Rect) {
    let points = vec![
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    let stroke = Stroke::new(1.0_f32, theme::TEXT_MUTED());
    ui.painter()
        .extend(Shape::dashed_line(&points, stroke, 5.0, 4.0));
}

/// The worktree chip text. A worktree exists only once Build has run.
fn worktree_chip(view: &TicketView) -> &'static str {
    if view.rounds.is_empty() {
        "no worktree yet"
    } else {
        "worktree · isolated"
    }
}

impl App {
    /// The ticket page: top bar, story, composer.
    pub(crate) fn ticket_page(&mut self, ui: &mut egui::Ui, id: &TicketId) {
        let Some(view) = self.ticket(id).cloned() else {
            return;
        };
        self.run_header(ui, &view);
        ui.horizontal(|ui| {
            let project = crate::nav::project_name(&view);
            theme::pill(
                ui,
                project.as_deref().unwrap_or("no project"),
                theme::GREY_FILL(),
                theme::GREY_TEXT(),
            );
            theme::pill(
                ui,
                worktree_chip(&view),
                theme::GREY_FILL(),
                theme::GREY_TEXT(),
            );
        });
        ui.add_space(12.0);
        self.next_card(ui, &view);
        self.orchestrator_card(ui, &view);
        self.story(ui, &view);
        ui.add_space(6.0);
        self.build_panel(ui, &view);
        self.files_panel(ui, &view);
        ui.add_space(12.0);
        self.composer_bar(ui);
    }

    /// One entry per step that has started, in order, so the newest is last.
    /// Skipped steps are dashed and say why.
    fn story(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let mut open = None;
        for card in &view.steps {
            if matches!(card.state, StepState::Idle) {
                continue;
            }
            // Build shows its rounds in the build panel below.
            if card.step == Step::BuildCheck && !view.rounds.is_empty() {
                continue;
            }
            let skipped = matches!(card.state, StepState::Skipped(_));
            let (glyph, color) = mark(&card.state);
            let shown = Frame::new()
                .fill(if skipped {
                    Color32::TRANSPARENT
                } else {
                    theme::CARD()
                })
                .stroke(if skipped {
                    Stroke::NONE
                } else {
                    Stroke::new(1.0_f32, theme::BORDER())
                })
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 24.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(glyph).color(color));
                        ui.label(RichText::new(step_name(card.step)).strong());
                        ui.label(theme::muted(state_text(&card.state)));
                    });
                    if let Some(short) = &card.in_short {
                        ui.label(short);
                    }
                });
            let rect = shown.response.rect;
            if skipped {
                dashed(ui, rect);
            }
            let click = ui.interact(
                rect,
                ui.id().with(("story", card.step as u8)),
                egui::Sense::click(),
            );
            if theme::named(click, step_name(card.step)).clicked() {
                open = Some(card.step);
            }
            ui.add_space(6.0);
        }
        if let Some(step) = open {
            self.screen = Screen::Step(step);
        }
    }

    /// Mode row and the plain-words change box. Modes that need a real agent
    /// are greyed with their reason.
    fn composer_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (label, why) in [("code", CODE_WHY), ("ask", ASK_WHY)] {
                ui.add_enabled(false, egui::Button::new(label))
                    .on_disabled_hover_text(why);
            }
            let selected = egui::Button::new("pipeline").fill(theme::SELECTED());
            ui.add(selected)
                .on_hover_text("Change how this ticket runs, in plain words");
            ui.add_enabled(false, egui::Button::new("/btw"))
                .on_disabled_hover_text(ASK_WHY);
            ui.add_enabled(false, egui::Button::new("when_unsure ask"))
                .on_disabled_hover_text(UNSURE_WHY);
            if ui.link("approvals").clicked() {
                self.mode = crate::app::Mode::Design;
            }
        });
        self.change_box(ui);
    }

    /// The right panel without a drawer: pipeline, stepper, cost, actions.
    pub(crate) fn stepper_panel(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        ui.label(
            RichText::new("PIPELINE")
                .size(13.0)
                .strong()
                .color(theme::TEXT_MUTED()),
        );
        ui.add_enabled(false, egui::Button::new("default pipeline"))
            .on_disabled_hover_text(PIPELINE_WHY);
        ui.label(theme::muted("changes only steps not started"));
        ui.add_space(8.0);
        let mut open = None;
        for card in &view.steps {
            let (glyph, color) = mark(&card.state);
            let ask = self
                .profile
                .as_ref()
                .ok()
                .and_then(|profile| profile.step(card.step))
                .is_some_and(|settings| settings.ask_after);
            let locked = matches!(card.step, Step::Design | Step::PullRequest);
            let skipped = matches!(card.state, StepState::Skipped(_));
            let row = ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).color(color));
                let name = RichText::new(step_name(card.step));
                let name = if skipped { name.weak() } else { name };
                if ui.link(name).clicked() {
                    open = Some(card.step);
                }
                if locked {
                    ui.label(theme::muted("ask · locked"));
                } else if ask {
                    ui.label(theme::muted("ask"));
                }
            });
            if let StepState::Skipped(reason) = &card.state {
                ui.label(theme::muted(format!("   skipped: {reason}")));
                dashed(ui, row.response.rect);
            }
        }
        if let Some(step) = open {
            self.screen = Screen::Step(step);
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new("COST")
                .size(13.0)
                .strong()
                .color(theme::TEXT_MUTED()),
        );
        ui.horizontal(|ui| {
            theme::cost_tag(ui, false);
            ui.label(dollars(view.api));
            theme::cost_tag(ui, true);
            ui.label(dollars(view.subscription));
        });
        ui.label(theme::muted("context  [not reported]"));
        ui.add_space(12.0);
        ui.label(
            RichText::new("ACTIONS")
                .size(13.0)
                .strong()
                .color(theme::TEXT_MUTED()),
        );
        for (text, why) in [
            (
                "Re-run from here",
                "coming soon: only Understand can run again until real agents (S6)",
            ),
            ("Open the worktree", "coming soon: no worktree yet (S6)"),
            (
                "Open in a terminal",
                "coming soon: needs a real session (S6)",
            ),
            (
                "Explain in plain words",
                "open a step, then use Explain in its drawer",
            ),
        ] {
            ui.add_enabled(false, egui::Button::new(text))
                .on_disabled_hover_text(why);
        }
    }
}
