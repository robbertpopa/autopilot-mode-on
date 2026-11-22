//! "Design the pipeline": the six steps in order, each with its settings. A
//! change is written to `profile.toml` through `engine` at once.

use autopilot_core::Step;
use autopilot_engine::{StepField, edit_step, step_name};
use autopilot_profile::{Auth, StepSettings};
use eframe::egui::{self, Frame, Margin, RichText, Stroke};

use crate::app::App;
use crate::theme;

const MODELS: [&str; 3] = ["haiku-5-5", "sonnet-5-5", "opus-5-5"];
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const TURNS: [u32; 6] = [10, 20, 30, 60, 100, 200];
const ROUNDS: [u32; 5] = [1, 2, 3, 5, 8];
const BUDGETS: [u64; 6] = [50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000];

fn blurb(step: Step) -> (&'static str, &'static str) {
    match step {
        Step::Understand => (
            "always runs",
            "Reads the ticket and the code. Reports its open questions, its assumptions and whether the ticket needs a design.",
        ),
        Step::Clarify => (
            "only if it has questions",
            "Asks plain questions in your inbox, each with a proposed default. It decides itself whether there is anything to ask.",
        ),
        Step::Design => (
            "only for UI changes",
            "Makes the design in Claude Design and waits for your approval. Feedback goes back to the same canvas.",
        ),
        Step::Plan => ("always runs", "Criteria, each with how it is checked."),
        Step::BuildCheck => (
            "loops until the checks pass",
            "One agent builds in its own worktree. Checks run cheapest first and stop at the first failure; the failed note goes back to the build.",
        ),
        Step::PullRequest => (
            "draft only",
            "Opens a draft PR with an accurate description. CI feedback returns to the build. You merge.",
        ),
    }
}

fn dollars_text(microusd: u64) -> String {
    autopilot_engine::dollars(autopilot_core::Money::from_microusd(microusd))
}

impl App {
    pub(crate) fn pipeline_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(
            RichText::new("How does a ticket become a pull request?")
                .size(30.0)
                .strong(),
        );
        ui.label(theme::muted(
            "Steps a ticket doesn't need are skipped. A step asks you when it needs something, or when you tick \"Ask me after this step\".",
        ));
        ui.add_space(10.0);
        if let Err(problem) = &self.profile {
            super::app::banner(ui, &format!("profile.toml: {problem}"), false);
        }
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 38.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("1   The steps, in order").size(19.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::segmented(
                        ui,
                        &mut self.advanced,
                        &[(false, "Simple"), (true, "Advanced")],
                    );
                    ui.label(theme::muted(format!(
                        "Selected: {} · saved in profile.toml",
                        step_name(self.design_step).to_lowercase()
                    )));
                });
            });
            ui.add_space(8.0);
            self.step_rows(ui);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Built by");
                ui.add_enabled(false, egui::Button::new("Automatic (Claude Code)"))
                    .on_disabled_hover_text("Claude Code is the only agent connected today");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(theme::primary("Start")).clicked() {
                        self.start_new();
                    }
                });
            });
            let name = self
                .options
                .profile
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ui.label(theme::muted(format!(
                "Settings are saved in {name}. Hover for the full path; use --profile <file> for another one."
            )))
            .on_hover_text(self.options.profile.display().to_string());
        });
    }

    fn step_rows(&mut self, ui: &mut egui::Ui) {
        let rows: [[(Step, f32); 3]; 2] = [
            [
                (Step::Understand, 1.0),
                (Step::Clarify, 1.0),
                (Step::Design, 1.0),
            ],
            [
                (Step::Plan, 0.54),
                (Step::BuildCheck, 1.0),
                (Step::PullRequest, 0.51),
            ],
        ];
        let gap = 40.0;
        for (index, row) in rows.iter().enumerate() {
            let total_weight: f32 = row.iter().map(|(_, weight)| weight).sum();
            let room = 2.0_f32.mul_add(-gap, ui.available_width()) - 28.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (position, (step, weight)) in row.iter().enumerate() {
                    let width = room * weight / total_weight;
                    ui.vertical(|ui| self.step_card(ui, *step, width));
                    if position < 2 {
                        arrow(ui, gap);
                    }
                }
            });
            if index == 0 {
                ui.add_space(4.0);
                ui.vertical_centered(|ui| {
                    ui.label(theme::muted("steps it doesn't need are skipped"));
                });
            }
        }
    }

    fn step_card(&mut self, ui: &mut egui::Ui, step: Step, width: f32) {
        let Some(settings) = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.step(step))
            .cloned()
        else {
            return;
        };
        let selected = self.design_step == step;
        let (fill, border) = if selected {
            (theme::BLUE_FILL(), theme::BLUE())
        } else {
            (theme::NODE(), theme::BORDER())
        };
        let (tag, text) = blurb(step);
        let frame = Frame::new()
            .fill(fill)
            .stroke(Stroke::new(
                if selected { 2.0_f32 } else { 1.0_f32 },
                border,
            ))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::same(14));
        let mut pick = false;
        let mut edit: Option<StepField> = None;
        frame.show(ui, |ui| {
            ui.set_width(width - 30.0);
            ui.set_min_height(240.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Label::new(RichText::new(step_name(step)).size(17.0).strong())
                            .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    pick = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (bg, fg) = if selected {
                        (theme::BLUE_FILL(), theme::BLUE_TEXT())
                    } else {
                        (theme::GREY_FILL(), theme::GREY_TEXT())
                    };
                    theme::pill(ui, tag, bg, fg);
                });
            });
            ui.label(RichText::new(text).size(13.0).color(theme::TEXT_MUTED()));
            if step == Step::Design {
                ui.add_enabled(false, egui::Label::new("Claude Design"))
                    .on_disabled_hover_text(
                        "The Design step is not built yet (it waits for S4), so its settings do nothing today",
                    );
            }
            if let Some(field) = self.settings_fields(ui, step, &settings) {
                edit = Some(field);
            }
            if step == Step::BuildCheck {
                self.checks_list(ui);
            }
            if let Some((problem_step, problem)) = &self.profile_problem
                && *problem_step == step
            {
                Frame::new()
                    .fill(theme::AMBER_FILL())
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.label(RichText::new(problem).size(12.0).color(theme::AMBER()));
                    });
            }
            let mut ask = settings.ask_after;
            let label = if step == Step::PullRequest {
                "Ask me before opening it"
            } else {
                "Ask me after this step"
            };
            if ui.checkbox(&mut ask, label).changed() {
                edit = Some(StepField::AskAfter(ask));
            }
            self.card_footer(ui, step);
        });
        if pick {
            self.design_step = step;
        }
        if let Some(field) = edit {
            self.design_step = step;
            self.apply_edit(step, &field);
        }
    }

    fn settings_fields(
        &self,
        ui: &mut egui::Ui,
        step: Step,
        settings: &StepSettings,
    ) -> Option<StepField> {
        let mut edit = None;
        let id = step.as_str();
        let model = settings.model.clone().unwrap_or_else(|| "default".into());
        if self.advanced {
            let auth = match settings.auth {
                Auth::Subscription => "subscription",
                Auth::Api => "api key",
            };
            field_row(ui, "auth", |ui| {
                combo(ui, (id, "auth"), auth, |ui| {
                    if ui.selectable_label(false, "subscription").clicked() {
                        edit = Some(StepField::Auth(Auth::Subscription));
                    }
                    if ui.selectable_label(false, "api key").clicked() {
                        edit = Some(StepField::Auth(Auth::Api));
                    }
                });
            });
        }
        field_row(ui, "model", |ui| {
            combo(ui, (id, "model"), &model, |ui| {
                for name in MODELS {
                    if ui.selectable_label(model == name, name).clicked() {
                        edit = Some(StepField::Model(name.to_owned()));
                    }
                }
            });
        });
        if !self.advanced {
            return edit;
        }
        let effort = settings.effort.clone().unwrap_or_else(|| "default".into());
        field_row(ui, "effort", |ui| {
            combo(ui, (id, "effort"), &effort, |ui| {
                for name in EFFORTS {
                    if ui.selectable_label(effort == name, name).clicked() {
                        edit = Some(StepField::Effort(name.to_owned()));
                    }
                }
            });
        });
        field_row(ui, "limit", |ui| {
            let shown = settings.limits.budget_microusd.map_or_else(
                || format!("turns {}", settings.limits.turns),
                |budget| format!("budget {}", dollars_text(budget.as_microusd())),
            );
            combo(ui, (id, "limit"), &shown, |ui| {
                for turns in TURNS {
                    if ui
                        .selectable_label(false, format!("turns {turns}"))
                        .clicked()
                    {
                        edit = Some(StepField::Turns(turns));
                    }
                }
                if settings.auth == Auth::Api {
                    ui.separator();
                    for budget in BUDGETS {
                        let text = format!("budget {}", dollars_text(budget));
                        if ui.selectable_label(false, text).clicked() {
                            edit = Some(StepField::BudgetMicrousd(Some(budget)));
                        }
                    }
                    if ui.selectable_label(false, "no dollar budget").clicked() {
                        edit = Some(StepField::BudgetMicrousd(None));
                    }
                }
            });
        });
        if step == Step::BuildCheck {
            field_row(ui, "refine rounds", |ui| {
                let shown = format!("at most {}", settings.limits.rounds);
                combo(ui, (id, "rounds"), &shown, |ui| {
                    for rounds in ROUNDS {
                        if ui
                            .selectable_label(false, format!("at most {rounds}"))
                            .clicked()
                        {
                            edit = Some(StepField::Rounds(rounds));
                        }
                    }
                });
            });
        }
        edit
    }

    fn checks_list(&self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("CHECKS, CHEAPEST FIRST")
                .size(12.0)
                .strong()
                .color(theme::BLUE_TEXT()),
        );
        let names: Vec<String> = self
            .profile
            .as_ref()
            .map(|profile| profile.checks.keys().cloned().collect())
            .unwrap_or_default();
        for (index, name) in names.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{} · {name}", index + 1));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new("code · free")
                            .monospace()
                            .size(11.0)
                            .color(theme::GREEN()),
                    );
                });
            });
        }
        for (label, note) in [
            ("protected paths, diff size", "code · free"),
            ("plan criteria met", "code · \"unknown\" cannot pass"),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(note)
                            .monospace()
                            .size(11.0)
                            .color(theme::GREEN()),
                    );
                });
            });
        }
        if names.is_empty() {
            ui.label(theme::muted(
                "No build or test commands yet. Add them under [checks] in profile.toml.",
            ));
        }
    }

    /// The "In short" line and cost of the most recent ticket, if there is one.
    fn card_footer(&self, ui: &mut egui::Ui, step: Step) {
        ui.separator();
        let latest = self.snapshot.tickets.first();
        let view = latest.and_then(|ticket| ticket.step(step));
        if let (Some(ticket), Some(view)) = (latest, view)
            && let Some(text) = &view.in_short
        {
            ui.label(
                RichText::new(format!("In short: {text}"))
                    .size(13.0)
                    .color(theme::TEXT_SOFT()),
            );
            ui.horizontal(|ui| {
                let estimated = view.subscription.as_microusd() > 0;
                theme::cost_tag(ui, estimated);
                let spent = if estimated {
                    view.subscription
                } else {
                    view.api
                };
                ui.label(theme::muted(format!(
                    "{} on \"{}\"",
                    autopilot_engine::dollars(spent),
                    ticket.title
                )));
            });
        } else {
            ui.label(theme::muted("In short: no ticket has run this step yet."));
        }
    }

    pub(crate) fn apply_edit(&mut self, step: Step, field: &StepField) {
        match edit_step(&self.options.profile, step, field) {
            Ok(profile) => {
                self.profile = Ok(profile);
                self.profile_problem = None;
            }
            Err(error) => self.profile_problem = Some((step, error.to_string())),
        }
    }
}

fn field_row(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [84.0, 24.0],
            egui::Label::new(RichText::new(label).size(13.0).color(theme::TEXT_MUTED())),
        );
        content(ui);
    });
}

fn combo(ui: &mut egui::Ui, id: (&str, &str), shown: &str, items: impl FnOnce(&mut egui::Ui)) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(shown).monospace().size(12.0))
        .width((ui.available_width() - 8.0).max(90.0))
        .show_ui(ui, items);
}

fn arrow(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::hover());
    let painter = ui.painter();
    let y = rect.top() + 24.0;
    let stroke = Stroke::new(2.0_f32, theme::ARROW());
    painter.line_segment(
        [
            egui::pos2(rect.left() + 8.0, y),
            egui::pos2(rect.right() - 8.0, y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(rect.right() - 8.0, y),
            egui::pos2(rect.right() - 15.0, y - 5.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(rect.right() - 8.0, y),
            egui::pos2(rect.right() - 15.0, y + 5.0),
        ],
        stroke,
    );
}
