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
const UNSURE_WHY: &str =
    "recorded per ticket, but no step reads it yet, so a switch here would change nothing";

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
        self.overlap_notice(ui, &view);
        self.orchestrator_card(ui, &view);
        self.story(ui, &view);
        ui.add_space(6.0);
        self.build_panel(ui, &view);
        self.files_panel(ui, &view);
        ui.add_space(12.0);
        self.composer_bar(ui);
    }

    /// Another open ticket in this project that predicts the same files. It
    /// only tells: starting, holding or merging is the owner's call.
    fn overlap_notice(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let key = (view.id.clone(), view.last_seq);
        let fresh = self
            .overlaps
            .as_ref()
            .is_some_and(|(id, seq, _)| *id == key.0 && *seq == key.1);
        if !fresh {
            let found = autopilot_engine::find_overlaps(&self.options.data_dir, &view.id)
                .unwrap_or_default();
            self.overlaps = Some((key.0, key.1, found));
        }
        let Some((_, _, found)) = self.overlaps.clone() else {
            return;
        };
        let mut open = None;
        for overlap in found {
            crate::app::banner(
                ui,
                &format!(
                    "{} also changes {}. Finish one first, or open it to compare.",
                    overlap.title, overlap.path
                ),
                false,
            );
            if ui.link(format!("Open {}", overlap.title)).clicked() {
                open = Some(overlap.other);
            }
        }
        if let Some(id) = open {
            self.open(&id);
        }
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
                ui.id().with(("story", card.step.as_str())),
                egui::Sense::click(),
            );
            if theme::named(click, step_name(card.step)).clicked() {
                open = Some(card.step);
            }
            ui.add_space(6.0);
        }
        if view.badge == autopilot_engine::Badge::Done
            && let Ok(profile) = &self.profile
        {
            crate::health::review_card(ui, view, profile.review.peak_context_permille);
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

    /// The pipeline of this ticket. A change applies to steps that have not
    /// started; the engine refuses one that would switch off a step that ran.
    fn ticket_pipeline_select(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let Ok(profile) = &self.profile else {
            return;
        };
        let names: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
        let current = view.pipeline.as_ref().map_or_else(
            || autopilot_profile::DEFAULT_PIPELINE.to_owned(),
            |(name, _)| name.clone(),
        );
        let mut chosen = current.clone();
        egui::ComboBox::from_id_salt("ticket-pipeline")
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut chosen, name.clone(), name);
                }
            });
        if chosen != current {
            self.act(crate::backend::Action::ChoosePipeline {
                ticket: view.id.clone(),
                name: chosen,
            });
        }
    }

    /// The right panel without a drawer: pipeline, stepper, cost, actions.
    pub(crate) fn stepper_panel(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let mut open = None;
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("PIPELINE")
                    .monospace()
                    .size(11.0)
                    .strong()
                    .color(theme::TEXT_MUTED()),
            );
            self.ticket_pipeline_select(ui, view);
            ui.label(theme::muted("changes only steps not started"));
            ui.add_space(8.0);
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new(glyph).color(color));
                    let name = RichText::new(step_name(card.step));
                    let name = if skipped { name.weak() } else { name };
                    if ui.link(name).clicked() {
                        open = Some(card.step);
                    }
                    // The state sits on the right, as in the design, so the
                    // column of names stays aligned.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if skipped {
                            ui.label(theme::muted("skipped"));
                        } else if locked {
                            ui.label(theme::muted("ask · locked"));
                        } else if ask {
                            ui.label(theme::muted("ask"));
                        }
                    });
                });
                if let StepState::Skipped(reason) = &card.state {
                    ui.label(theme::muted(format!("      {reason}")));
                }
            }
        });
        if let Some(step) = open {
            self.screen = Screen::Step(step);
        }
        ui.add_space(10.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("COST")
                    .monospace()
                    .size(11.0)
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
        });
        ui.add_space(10.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
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
                ui.add_enabled(
                    false,
                    egui::Button::new(text)
                        .frame(false)
                        .min_size(egui::vec2(ui.available_width(), 28.0)),
                )
                .on_disabled_hover_text(why);
            }
        });
    }
}
