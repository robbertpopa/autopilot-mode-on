//! Pipelines: which steps a ticket runs. The built-in presets are read-only;
//! a custom pipeline can be made from one and its steps switched on or off.
//! A pipeline never changes what a step asks: that stays with Settings and
//! the policy, so Design and the pull request are drawn as locked.

use autopilot_core::Step;
use autopilot_engine::{ProjectField, add_pipeline, edit_project, step_name};
use autopilot_profile::{Pipeline, StepMode};
use eframe::egui::{self, Color32, RichText};

use crate::app::App;
use crate::theme;

const fn mode_word(mode: StepMode) -> &'static str {
    match mode {
        StepMode::On => "on",
        StepMode::IfNeeded => "if needed",
        StepMode::Off => "off",
    }
}

/// The pipeline a New ticket suggests for what the owner says it is.
#[must_use]
pub const fn suggested(kind: crate::new_ticket::Kind) -> &'static str {
    match kind {
        crate::new_ticket::Kind::Bug => "bug-fix",
        crate::new_ticket::Kind::Change => autopilot_profile::DEFAULT_PIPELINE,
        crate::new_ticket::Kind::Ask => autopilot_profile::ASK_ONLY,
    }
}

/// A row of six dots, one per step: solid runs, hollow runs if needed,
/// faint is off.
fn dots(ui: &mut egui::Ui, pipeline: &Pipeline) {
    ui.horizontal(|ui| {
        for step in Step::ALL {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            let painter = ui.painter();
            let center = rect.center();
            match pipeline.mode(step) {
                StepMode::On => {
                    painter.circle_filled(center, 5.0, theme::BLUE());
                }
                StepMode::IfNeeded => {
                    painter.circle_stroke(center, 5.0, egui::Stroke::new(1.5_f32, theme::BLUE()));
                }
                StepMode::Off => {
                    painter.circle_filled(center, 3.0, theme::TEXT_MUTED());
                }
            }
        }
    });
}

impl App {
    pub(crate) fn pipelines_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Pipelines").size(18.0).strong());
        let Ok(profile) = &self.profile else {
            ui.label(theme::muted("The profile could not be read."));
            return;
        };
        let all = profile.pipelines();
        if self.pipeline_selected.is_none() {
            self.pipeline_selected = all.first().map(|pipeline| pipeline.name.clone());
        }
        ui.add_space(8.0);
        let width = (ui.available_width() * 0.28).clamp(220.0, 300.0);
        let mut pick = None;
        let default_name = Some(autopilot_profile::DEFAULT_PIPELINE.to_owned());
        let selected = self
            .pipeline_selected
            .as_deref()
            .and_then(|name| all.iter().find(|pipeline| pipeline.name == name))
            .cloned();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for pipeline in &all {
                        let chosen =
                            self.pipeline_selected.as_deref() == Some(pipeline.name.as_str());
                        let word = if default_name.as_deref() == Some(pipeline.name.as_str()) {
                            "default"
                        } else if pipeline.builtin {
                            "built-in"
                        } else {
                            "custom"
                        };
                        if pipeline_card(ui, pipeline, word, chosen) {
                            pick = Some(pipeline.name.clone());
                        }
                    }
                    let review = egui::Frame::new()
                        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("review")
                                        .monospace()
                                        .strong()
                                        .color(theme::TEXT_MUTED()),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        theme::pill(
                                            ui,
                                            "soon",
                                            theme::GREY_FILL(),
                                            theme::GREY_TEXT(),
                                        );
                                    },
                                );
                            });
                        });
                    review
                        .response
                        .on_hover_text("soon: the review pipeline arrives with M9");
                },
            );
            ui.vertical(|ui| {
                if let Some(pipeline) = &selected {
                    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.pipeline_detail(ui, pipeline);
                    });
                }
                ui.add_space(10.0);
                theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.build_with_agent(ui);
                });
            });
        });
        if let Some(name) = pick {
            self.pipeline_selected = Some(name);
        }
    }

    /// Describe a workflow and an agent drafts a pipeline. The engine checks
    /// the draft before it is shown; it is a dashed preview until the owner
    /// saves it, and an agent cannot unlock Design or the pull request, drop a
    /// forced ask or change a budget.
    fn build_with_agent(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Build with agent").size(14.0).strong());
        ui.add(
            egui::TextEdit::multiline(&mut self.draft_text)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("Describe the workflow, e.g. a small change that needs no questions"),
        );
        let claude = self.agent == crate::backend::AgentChoice::Claude;
        let ready = !self.draft_text.trim().is_empty();
        let button = ui.add_enabled(ready && !claude, egui::Button::new("Draft"));
        let button = if claude {
            button.on_disabled_hover_text(
                "coming soon: a live draft spends money, so only the practice agent drafts for now",
            )
        } else {
            button
        };
        if button.clicked() {
            match crate::backend::draft_pipeline(&self.options, &self.draft_text) {
                Ok(draft) => {
                    self.draft = Some(draft);
                    self.pipeline_notice = None;
                }
                Err(why) => {
                    self.draft = None;
                    self.pipeline_notice = Some((false, why));
                }
            }
        }
        if let Some(draft) = self.draft.clone() {
            let shown = egui::Frame::new()
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("draft · {}", draft.name)).strong());
                    dots(ui, &draft.pipeline);
                    for step in Step::ALL {
                        ui.horizontal(|ui| {
                            theme::cell(ui, 150.0, step_name(step));
                            ui.label(theme::muted(mode_word(draft.pipeline.mode(step))));
                            if let Some((_, why)) =
                                draft.reasons.iter().find(|(each, _)| *each == step)
                            {
                                ui.label(why);
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary("Save to profile.toml")).clicked() {
                            match autopilot_engine::save_draft(&self.options.profile, &draft) {
                                Ok(profile) => {
                                    self.profile = Ok(profile);
                                    self.pipeline_selected = Some(draft.name.clone());
                                    self.pipeline_notice =
                                        Some((true, format!("saved {}", draft.name)));
                                    self.draft = None;
                                }
                                Err(error) => {
                                    self.pipeline_notice = Some((false, error.to_string()));
                                }
                            }
                        }
                        if ui.button("Discard").clicked() {
                            self.draft = None;
                        }
                    });
                });
            // A draft is dashed: nothing is stored until it is saved.
            let rect = shown.response.rect;
            let points = vec![
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
                rect.left_top(),
            ];
            ui.painter().extend(egui::Shape::dashed_line(
                &points,
                egui::Stroke::new(1.0_f32, theme::TEXT_MUTED()),
                5.0,
                4.0,
            ));
        }
        if let Some((ok, text)) = &self.pipeline_notice {
            let color = if *ok { theme::GREEN() } else { theme::RED() };
            ui.label(RichText::new(text).color(color));
        }
    }

    fn pipeline_detail(&mut self, ui: &mut egui::Ui, pipeline: &Pipeline) {
        ui.label(
            RichText::new(&pipeline.name)
                .monospace()
                .size(14.0)
                .strong(),
        );
        if pipeline.builtin {
            ui.label(theme::muted(
                "built-in, read-only. Duplicate it to change it.",
            ));
        }
        let mut modes: Vec<(Step, StepMode)> = Step::ALL
            .iter()
            .map(|step| (*step, pipeline.mode(*step)))
            .collect();
        let mut changed = false;
        for (step, mode) in &mut modes {
            ui.horizontal(|ui| {
                theme::cell(ui, 150.0, step_name(*step));
                if pipeline.builtin {
                    ui.label(mode_word(*mode));
                } else {
                    for (value, label) in [
                        (StepMode::On, "on"),
                        (StepMode::IfNeeded, "if needed"),
                        (StepMode::Off, "off"),
                    ] {
                        if ui.selectable_label(*mode == value, label).clicked() && *mode != value {
                            *mode = value;
                            changed = true;
                        }
                    }
                }
                if matches!(step, Step::Design | Step::PullRequest) && *mode != StepMode::Off {
                    ui.label(theme::muted("ask · locked"));
                } else if let Some(settings) = self
                    .profile
                    .as_ref()
                    .ok()
                    .and_then(|profile| profile.step(*step))
                    && settings.ask_after
                    && *mode != StepMode::Off
                {
                    ui.label(theme::muted("ask"));
                }
            });
        }
        if let Some(rounds) = pipeline.rounds {
            ui.label(theme::muted(format!("build rounds capped at {rounds}")));
        }
        ui.label(theme::muted(
            "Design and the pull request always ask when they run. A pipeline cannot change that.",
        ));
        if changed {
            self.save_pipeline_modes(pipeline, &modes);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Duplicate").clicked() {
                self.duplicate_pipeline(&pipeline.name);
            }
            self.set_default_button(ui, &pipeline.name);
        });
        if let Some((ok, text)) = &self.pipeline_notice {
            let color = if *ok { theme::GREEN() } else { theme::RED() };
            ui.label(RichText::new(text).color(color));
        }
    }

    fn set_default_button(&mut self, ui: &mut egui::Ui, name: &str) {
        let Some(project) = self.project_selected.clone() else {
            ui.add_enabled(false, egui::Button::new("Set as default"))
                .on_disabled_hover_text("Choose a project on the Projects page first");
            return;
        };
        let label = format!("Set as default · {}", autopilot_profile::leaf(&project));
        if ui.button(label).clicked() {
            match edit_project(
                &self.options.profile,
                &project,
                Some(&ProjectField::DefaultPipeline(Some(name.to_owned()))),
            ) {
                Ok(profile) => {
                    self.profile = Ok(profile);
                    self.pipeline_notice = Some((true, format!("{name} is the default now")));
                }
                Err(error) => self.pipeline_notice = Some((false, error.to_string())),
            }
        }
    }

    /// Copy a pipeline under a free name, so a built-in can be changed.
    pub(crate) fn duplicate_pipeline(&mut self, name: &str) {
        let Ok(profile) = &self.profile else {
            return;
        };
        let taken: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
        let mut copy = format!("{name}-copy");
        let mut number = 2;
        while taken.contains(&copy) {
            copy = format!("{name}-copy-{number}");
            number += 1;
        }
        let Some(source) = profile.pipeline(name) else {
            return;
        };
        let modes: Vec<(Step, StepMode)> = Step::ALL
            .iter()
            .map(|step| (*step, source.mode(*step)))
            .collect();
        self.write_pipeline(&copy, &modes, source.rounds);
        self.pipeline_selected = Some(copy);
    }

    fn save_pipeline_modes(&mut self, pipeline: &Pipeline, modes: &[(Step, StepMode)]) {
        self.write_pipeline(&pipeline.name, modes, pipeline.rounds);
    }

    fn write_pipeline(&mut self, name: &str, modes: &[(Step, StepMode)], rounds: Option<u32>) {
        let list = |wanted: StepMode| -> Vec<Step> {
            modes
                .iter()
                .filter(|(_, mode)| *mode == wanted)
                .map(|(step, _)| *step)
                .collect()
        };
        match add_pipeline(
            &self.options.profile,
            name,
            autopilot_profile::DEFAULT_PIPELINE,
            &list(StepMode::On),
            &list(StepMode::IfNeeded),
            &list(StepMode::Off),
            rounds,
        ) {
            Ok(profile) => {
                self.profile = Ok(profile);
                self.pipeline_notice = Some((true, format!("saved {name}")));
            }
            // The file is left as it was, so the page shows what is stored.
            Err(error) => self.pipeline_notice = Some((false, error.to_string())),
        }
    }

    #[cfg(test)]
    pub(crate) fn write_pipeline_for_test(&mut self, name: &str, modes: &[(Step, StepMode)]) {
        self.write_pipeline(name, modes, None);
    }
}

/// One pipeline in the list: name, a chip, and the six step dots.
fn pipeline_card(ui: &mut egui::Ui, pipeline: &Pipeline, word: &str, chosen: bool) -> bool {
    let shown = egui::Frame::new()
        .fill(if chosen {
            theme::CARD()
        } else {
            Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if chosen {
                theme::BORDER_STRONG()
            } else {
                theme::BORDER()
            },
        ))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&pipeline.name)
                        .monospace()
                        .size(13.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (fill, color) = if word == "default" {
                        (theme::BLUE_FILL(), theme::BLUE_TEXT())
                    } else {
                        (theme::GREY_FILL(), theme::GREY_TEXT())
                    };
                    theme::pill(ui, word, fill, color);
                });
            });
            dots(ui, pipeline);
        });
    let click = ui.interact(
        shown.response.rect,
        ui.id().with(("pipeline-card", pipeline.name.as_str())),
        egui::Sense::click(),
    );
    theme::named(click, &pipeline.name).clicked()
}
