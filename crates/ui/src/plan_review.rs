//! Plan review: the whole plan on one page before anything is built. Each
//! criterion shows how it will be verified; a plan that cannot be checked as
//! written says why, and its approval is a forced confirmation.

use autopilot_core::{Plan, Step, Verify};
use autopilot_engine::TicketView;
use autopilot_pipeline::StepState;
use autopilot_pipeline::analyze::{Analysis, Severity, analyze};
use eframe::egui::{self, RichText};

use crate::app::{App, Mode};
use crate::backend::Action;
use crate::theme;

/// The recorded plan of a ticket, if it has one and it reads back.
#[must_use]
pub fn plan_of(view: &TicketView) -> Option<Plan> {
    let (_, text) = view.outputs.iter().find(|(step, _)| *step == Step::Plan)?;
    serde_json::from_str(text).ok()
}

/// Whether the ticket is waiting for the owner on its plan.
#[must_use]
pub fn awaiting_plan(view: &TicketView) -> bool {
    matches!(
        view.step(Step::Plan).map(|card| &card.state),
        Some(StepState::Asking(_))
    )
}

fn verify_label(verify: Verify) -> &'static str {
    match verify {
        Verify::Test => "test",
        Verify::Command => "command",
        Verify::Unknown => "unknown",
    }
}

impl App {
    pub(crate) fn plan_review(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.current().cloned() else {
            self.mode = Mode::Watch;
            return;
        };
        if ui.link("< Back to the ticket").clicked() {
            self.mode = Mode::Watch;
        }
        ui.label(RichText::new("Plan review").size(18.0).strong());
        ui.label(theme::muted(&view.title));
        ui.add_space(10.0);
        let Some(plan) = plan_of(&view) else {
            ui.label(theme::muted("This ticket has no plan to review yet."));
            return;
        };
        let analysis = analyze(&plan);
        // Below this width the side card goes under the main column instead of being cut off.
        let narrow = ui.available_width() < 820.0;
        let aside = if narrow {
            ui.available_width()
        } else {
            280.0_f32.min(ui.available_width() * 0.3)
        };
        let main = if narrow {
            ui.available_width()
        } else {
            ui.available_width() - aside - 24.0
        };
        let columns = egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true);
        ui.with_layout(columns, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(16.0, 12.0);
            ui.allocate_ui_with_layout(
                egui::vec2(main, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(main);
                    Self::criteria_table(ui, &plan, &analysis);
                    ui.add_space(10.0);
                    list(ui, "Files", &plan.files, "The plan lists no files.");
                    ui.add_space(6.0);
                    list(
                        ui,
                        "Test plan",
                        &plan.test_plan,
                        "The plan has no test plan.",
                    );
                    ui.add_space(10.0);
                    self.assumptions_box(ui, &view);
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(aside, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(aside);
                    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.label(
                            RichText::new("ON APPROVE")
                                .monospace()
                                .size(11.0)
                                .strong()
                                .color(theme::TEXT_MUTED()),
                        );
                        for line in [
                            "1. build in worktree",
                            "2. checks cheapest first",
                            "3. ask before pr",
                        ] {
                            ui.label(theme::mono(line).size(12.0));
                        }
                        ui.add_space(4.0);
                        self.plan_actions(ui, &view, &analysis);
                    });
                },
            );
        });
    }

    fn criteria_table(ui: &mut egui::Ui, plan: &Plan, analysis: &Analysis) {
        ui.label(RichText::new("Criteria").size(14.0).strong());
        for (criterion, (_, covered)) in plan.criteria.iter().zip(&analysis.coverage) {
            theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                ui.set_width(ui.available_width() - 38.0);
                // The text goes under the id and the pill: a long criterion in
                // the same row was drawn past the window's right edge.
                ui.horizontal(|ui| {
                    ui.label(theme::mono(&criterion.id));
                    let unknown = criterion.verify == Verify::Unknown;
                    let (fill, color) = if unknown {
                        (theme::RED_FILL(), theme::RED())
                    } else {
                        (theme::GREY_FILL(), theme::GREY_TEXT())
                    };
                    let text = if criterion.verification_id.is_empty() {
                        verify_label(criterion.verify).to_owned()
                    } else {
                        format!(
                            "{} {}",
                            verify_label(criterion.verify),
                            criterion.verification_id
                        )
                    };
                    theme::pill(ui, &text, fill, color);
                });
                ui.label(&criterion.text);
                if criterion.verify != Verify::Unknown && !covered {
                    ui.label(theme::muted("not named in the test plan"));
                }
                for finding in analysis
                    .findings
                    .iter()
                    .filter(|finding| finding.criterion.as_deref() == Some(criterion.id.as_str()))
                {
                    let color = if finding.severity == Severity::Error {
                        theme::RED()
                    } else {
                        theme::AMBER()
                    };
                    ui.label(RichText::new(&finding.message).color(color));
                }
            });
            ui.add_space(4.0);
        }
    }

    /// The assumptions Understand made, each with a box for a correction, and
    /// what was corrected before.
    fn assumptions_box(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let assumptions = assumptions_of(view);
        ui.label(RichText::new("Assumptions").size(14.0).strong());
        if assumptions.is_empty() {
            ui.label(theme::muted("Understand recorded no assumptions."));
        }
        if self.corrections.len() != assumptions.len() {
            self.corrections = vec![String::new(); assumptions.len()];
        }
        for (assumption, correction) in assumptions.iter().zip(&mut self.corrections) {
            ui.label(assumption);
            ui.add(
                egui::TextEdit::singleline(correction)
                    .hint_text("Correct it, or leave empty to keep it")
                    .desired_width(ui.available_width()),
            );
        }
        for fix in &view.corrections {
            ui.label(theme::muted(format!(
                "corrected: {} -> {}",
                fix.assumption, fix.correction
            )));
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Note for sending back").strong());
        ui.add(
            egui::TextEdit::multiline(&mut self.plan_note)
                .desired_rows(2)
                .desired_width(ui.available_width())
                .hint_text("What should the plan do differently?"),
        );
    }

    /// Approve, carrying any corrections the owner typed.
    fn approve_plan(&mut self, view: &TicketView) {
        let assumptions = assumptions_of(view);
        let corrections: Vec<autopilot_core::AssumptionCorrection> = assumptions
            .iter()
            .zip(&self.corrections)
            .filter(|(_, fix)| !fix.trim().is_empty())
            .map(|(assumption, fix)| autopilot_core::AssumptionCorrection {
                assumption: assumption.clone(),
                correction: fix.trim().to_owned(),
            })
            .collect();
        if corrections.is_empty() {
            self.act(Action::Decide {
                ticket: view.id.clone(),
                approve: true,
                always: false,
            });
        } else {
            self.act(Action::ApproveCorrected {
                ticket: view.id.clone(),
                corrections,
            });
        }
        self.corrections.clear();
        self.mode = Mode::Watch;
    }

    fn plan_actions(&mut self, ui: &mut egui::Ui, view: &TicketView, analysis: &Analysis) {
        if !awaiting_plan(view) {
            ui.label(theme::muted("The plan is not waiting for a decision."));
            return;
        }
        if analysis.blocking() {
            ui.label(
                RichText::new(
                    "Approval is a forced confirmation: at least one criterion cannot be checked as written.",
                )
                .color(theme::AMBER()),
            );
        }
        ui.vertical(|ui| {
            let label = if analysis.blocking() {
                "Approve anyway"
            } else {
                "Approve"
            };
            if ui.add(theme::primary(label)).clicked() {
                self.approve_plan(view);
            }
            let note = self.plan_note.trim().to_owned();
            if ui
                .add_enabled(!note.is_empty(), egui::Button::new("Send back with notes"))
                .on_disabled_hover_text("Write a note first")
                .clicked()
            {
                self.act(Action::SendBack {
                    ticket: view.id.clone(),
                    note,
                });
                self.plan_note.clear();
                self.mode = Mode::Watch;
            }
            if ui.button("Reject").clicked() {
                self.act(Action::Decide {
                    ticket: view.id.clone(),
                    approve: false,
                    always: false,
                });
                self.mode = Mode::Watch;
            }
        });
    }
}

fn list(ui: &mut egui::Ui, title: &str, items: &[String], empty: &str) {
    ui.label(RichText::new(title).size(14.0).strong());
    if items.is_empty() {
        ui.label(theme::muted(empty));
    }
    for item in items {
        ui.label(theme::mono(item));
    }
}

/// The assumptions Understand recorded, from its saved result.
fn assumptions_of(view: &TicketView) -> Vec<String> {
    view.outputs
        .iter()
        .find(|(step, _)| *step == Step::Understand)
        .and_then(|(_, text)| serde_json::from_str::<autopilot_core::Understanding>(text).ok())
        .map(|result| result.assumptions)
        .unwrap_or_default()
}
