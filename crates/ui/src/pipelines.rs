//! Pipelines: which steps a ticket runs. The built-in presets are read-only;
//! a custom pipeline can be made from one and its steps switched on or off.
//! A pipeline never changes what a step asks: that stays with Settings and
//! the policy, so Design and the pull request are drawn as locked.

use autopilot_core::Step;
use autopilot_engine::{
    ProjectField, add_pipeline, edit_project, free_custom_slot, remove_custom_step,
    set_custom_step, step_name,
};
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
        // Six dots for the built-in steps, then one for each custom step the
        // pipeline runs.
        let steps = Step::EVERY
            .into_iter()
            .filter(|step| !step.is_custom() || pipeline.mode(*step) != StepMode::Off);
        for step in steps {
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
                    self.build_with_agent(ui, selected.as_ref());
                });
            });
        });
        if let Some(name) = pick {
            self.pipeline_selected = Some(name);
        }
    }

    /// Describe a workflow and an agent drafts a pipeline, or changes the one
    /// that is open. The engine checks the draft before it is shown; it is a
    /// dashed preview until the owner saves it, and an agent cannot unlock
    /// Design or the pull request, drop a forced ask or change a budget. A step
    /// it adds is shown in full, because its prompt becomes an instruction once
    /// saved.
    fn build_with_agent(&mut self, ui: &mut egui::Ui, open: Option<&Pipeline>) {
        ui.label(RichText::new("Build with the agent").size(14.0).strong());
        ui.label(theme::muted(
            "Say what you want. The agent proposes steps, including new ones, and nothing is saved until you press Save.",
        ));
        ui.add(
            egui::TextEdit::multiline(&mut self.draft_text)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text(
                    "e.g. add a security review before the pull request, or a small change that needs no questions",
                ),
        );
        let ready = !self.draft_text.trim().is_empty();
        let changeable = open.filter(|pipeline| !pipeline.builtin);
        ui.horizontal(|ui| {
            let new = ui.add_enabled(ready, theme::primary("Draft a new pipeline"));
            if new.clicked() {
                self.ask_agent(None);
            }
            let label = changeable.map_or_else(
                || "Change this pipeline".to_owned(),
                |pipeline| format!("Change {}", pipeline.name),
            );
            let button = ui.add_enabled(ready && changeable.is_some(), egui::Button::new(label));
            let button = if changeable.is_none() {
                button.on_disabled_hover_text(
                    "A built-in pipeline is read-only. Duplicate it, then change the copy.",
                )
            } else {
                button
            };
            if button.clicked() {
                self.ask_agent(changeable.map(|pipeline| pipeline.name.clone()));
            }
        });
        if let Some(draft) = self.draft.clone() {
            self.draft_preview(ui, &draft);
        }
        if let Some((ok, text)) = &self.pipeline_notice {
            let color = if *ok { theme::GREEN() } else { theme::RED() };
            ui.label(RichText::new(text).color(color));
        }
    }

    fn ask_agent(&mut self, editing: Option<String>) {
        match crate::backend::draft_pipeline(&self.options, &self.draft_text, editing.as_deref()) {
            Ok(draft) => {
                self.draft = Some(draft);
                self.draft_editing = editing;
                self.pipeline_notice = None;
            }
            Err(why) => {
                self.draft = None;
                self.pipeline_notice = Some((false, why));
            }
        }
    }

    fn draft_preview(&mut self, ui: &mut egui::Ui, draft: &autopilot_engine::PipelineDraft) {
        let shown = egui::Frame::new()
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                let title = if draft.replaces {
                    format!("change · {}", draft.name)
                } else {
                    format!("draft · {}", draft.name)
                };
                ui.label(RichText::new(title).strong());
                dots(ui, &draft.pipeline);
                for step in Step::ALL {
                    ui.horizontal(|ui| {
                        theme::cell(ui, 150.0, step_name(step));
                        ui.label(theme::muted(mode_word(draft.pipeline.mode(step))));
                        if let Some((_, why)) = draft.reasons.iter().find(|(each, _)| *each == step)
                        {
                            ui.label(why);
                        }
                    });
                }
                for (_, definition) in &draft.new_steps {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("new step · {}", definition.name)).strong());
                    ui.label(theme::muted("runs after the build · its prompt, in full:"));
                    ui.label(&definition.prompt);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary("Save to profile.toml")).clicked() {
                        self.save_agent_draft(draft);
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

    fn save_agent_draft(&mut self, draft: &autopilot_engine::PipelineDraft) {
        match autopilot_engine::save_draft(&self.options.profile, draft) {
            Ok(profile) => {
                self.profile = Ok(profile);
                self.pipeline_selected = Some(draft.name.clone());
                self.pipeline_notice = Some((true, format!("saved {}", draft.name)));
                self.draft = None;
                self.draft_text.clear();
            }
            Err(error) => self.pipeline_notice = Some((false, error.to_string())),
        }
    }

    /// The steps a pipeline page lists, in run order: the six built-in ones
    /// with every custom step the profile defines between Build and the pull
    /// request.
    fn listed_steps(&self) -> Vec<Step> {
        let Ok(profile) = &self.profile else {
            return Step::ALL.to_vec();
        };
        Step::EVERY
            .into_iter()
            .filter(|step| !step.is_custom() || profile.custom_step(*step).is_some())
            .collect()
    }

    fn pipeline_detail(&mut self, ui: &mut egui::Ui, pipeline: &Pipeline) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&pipeline.name)
                    .monospace()
                    .size(15.0)
                    .strong(),
            );
            theme::pill(
                ui,
                if pipeline.builtin {
                    "built-in"
                } else {
                    "custom"
                },
                theme::BLUE_FILL(),
                theme::BLUE_TEXT(),
            );
        });
        if pipeline.builtin {
            ui.label(theme::muted(
                "Built-in and read-only. Duplicate it to change the steps or add your own.",
            ));
        } else {
            ui.label(theme::muted(
                "Click a step to see and change what it does. Switch a step on or off with the buttons.",
            ));
        }
        ui.add_space(6.0);
        let steps = self.listed_steps();
        let mut modes: Vec<(Step, StepMode)> = steps
            .iter()
            .map(|step| (*step, pipeline.mode(*step)))
            .collect();
        let mut changed = false;
        for (number, (step, mode)) in modes.iter_mut().enumerate() {
            if self.step_row(ui, pipeline, number + 1, *step, mode) {
                changed = true;
            }
            ui.add_space(4.0);
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
        ui.add_space(6.0);
        self.add_step_form(ui, pipeline);
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

    /// One step as a card: its number, name and mode, and when opened what it
    /// does and every setting it has. Returns true when the mode was changed.
    fn step_row(
        &mut self,
        ui: &mut egui::Ui,
        pipeline: &Pipeline,
        number: usize,
        step: Step,
        mode: &mut StepMode,
    ) -> bool {
        let open = self.step_open == Some(step);
        let off = *mode == StepMode::Off;
        let mut changed = false;
        let mut toggle = false;
        let frame = egui::Frame::new()
            .fill(if open {
                theme::BLUE_FILL()
            } else {
                theme::NODE()
            })
            .stroke(if open {
                egui::Stroke::new(2.0_f32, theme::BLUE())
            } else {
                egui::Stroke::new(1.0_f32, theme::BORDER())
            })
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 9));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                let fill = if off {
                    theme::GREY_FILL()
                } else {
                    theme::BLUE()
                };
                ui.painter().circle_filled(rect.center(), 12.0, fill);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    number.to_string(),
                    egui::FontId::proportional(12.0),
                    if off {
                        theme::GREY_TEXT()
                    } else {
                        theme::ON_ACCENT()
                    },
                );
                let name = RichText::new(step_name(step)).strong().size(14.0);
                let name = if off {
                    name.color(theme::TEXT_MUTED())
                } else {
                    name
                };
                if ui
                    .add(egui::Label::new(name).sense(egui::Sense::click()))
                    .clicked()
                {
                    toggle = true;
                }
                if step.is_custom() {
                    theme::pill(ui, "yours", theme::AMBER_FILL(), theme::AMBER());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(if open { "Hide" } else { "Edit" })
                        .clicked()
                    {
                        toggle = true;
                    }
                    changed |= Self::mode_buttons(ui, pipeline, step, mode);
                });
            });
            if open {
                ui.add_space(6.0);
                self.step_body(ui, step);
            }
        });
        if toggle {
            self.step_open = if open { None } else { Some(step) };
            self.custom_edit = None;
        }
        changed
    }

    /// The on, if needed and off buttons of a step in a custom pipeline, or
    /// the plain word for a built-in one.
    fn mode_buttons(
        ui: &mut egui::Ui,
        pipeline: &Pipeline,
        step: Step,
        mode: &mut StepMode,
    ) -> bool {
        if pipeline.builtin {
            ui.label(theme::muted(mode_word(*mode)));
            return false;
        }
        let mut changed = false;
        // Shown right to left, so list them in reverse.
        let all = [
            (StepMode::Off, "off"),
            (StepMode::IfNeeded, "if needed"),
            (StepMode::On, "on"),
        ];
        for (value, label) in all {
            if step.is_custom() && value == StepMode::IfNeeded {
                continue;
            }
            if ui.selectable_label(*mode == value, label).clicked() && *mode != value {
                *mode = value;
                changed = true;
            }
        }
        changed
    }

    /// What a step does and all of its settings.
    fn step_body(&mut self, ui: &mut egui::Ui, step: Step) {
        let (tag, text) = crate::pipeline::blurb(step);
        ui.horizontal_wrapped(|ui| {
            theme::pill(ui, tag, theme::GREY_FILL(), theme::GREY_TEXT());
            ui.label(theme::muted(text));
        });
        if step.is_custom() {
            self.custom_editor(ui, step);
        }
        let settings = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.step(step))
            .cloned();
        let Some(settings) = settings else {
            return;
        };
        ui.add_space(4.0);
        let edit = self.settings_fields(ui, step, &settings);
        let locked = crate::settings::locked(step);
        let mut ask = settings.ask_after || locked;
        let label = if step == Step::PullRequest {
            "Ask me before opening it"
        } else {
            "Ask me after this step"
        };
        let toggled = ui
            .add_enabled(!locked, egui::Checkbox::new(&mut ask, label))
            .on_disabled_hover_text("Design and the pull request always ask")
            .changed();
        if let Some((problem_step, problem)) = &self.profile_problem
            && *problem_step == step
        {
            ui.label(RichText::new(problem).color(theme::AMBER()));
        }
        if let Some(field) = edit {
            self.apply_edit(step, &field);
        } else if toggled {
            self.apply_edit(step, &autopilot_engine::StepField::AskAfter(ask));
        }
    }

    /// The name and prompt of one of the owner's steps, with Save and Remove.
    fn custom_editor(&mut self, ui: &mut egui::Ui, step: Step) {
        let saved = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.custom_step(step))
            .cloned();
        let Some(saved) = saved else {
            return;
        };
        if self.custom_edit.as_ref().map(|(each, _, _)| *each) != Some(step) {
            self.custom_edit = Some((step, saved.name.clone(), saved.prompt.clone()));
        }
        let Some((_, name, prompt)) = self.custom_edit.as_mut() else {
            return;
        };
        ui.add_space(4.0);
        ui.label(theme::muted("Name"));
        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        ui.label(theme::muted(
            "Prompt: what the agent does at this step. It reads the ticket and cannot change files.",
        ));
        ui.add(
            egui::TextEdit::multiline(prompt)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        let dirty = *name != saved.name || *prompt != saved.prompt;
        let (name, prompt) = (name.clone(), prompt.clone());
        ui.horizontal(|ui| {
            if ui.add_enabled(dirty, theme::primary("Save step")).clicked() {
                match set_custom_step(&self.options.profile, step, &name, &prompt) {
                    Ok(profile) => {
                        self.profile = Ok(profile);
                        self.pipeline_notice = Some((true, format!("saved {name}")));
                    }
                    Err(error) => self.pipeline_notice = Some((false, error.to_string())),
                }
            }
            if ui.button("Remove step").clicked() {
                match remove_custom_step(&self.options.profile, step) {
                    Ok(profile) => {
                        self.profile = Ok(profile);
                        self.step_open = None;
                        self.custom_edit = None;
                        self.pipeline_notice = Some((true, format!("removed {}", saved.name)));
                    }
                    Err(error) => self.pipeline_notice = Some((false, error.to_string())),
                }
            }
        });
    }

    /// The "add a step" form. A built-in pipeline is copied first, so the new
    /// step lands in a pipeline that can change.
    fn add_step_form(&mut self, ui: &mut egui::Ui, pipeline: &Pipeline) {
        // Held outside `self` while it is edited, and put back unless the
        // owner added the step or cancelled.
        let Some((mut name, mut prompt)) = self.new_step.take() else {
            if ui.button("+ Add a step").clicked() {
                self.new_step = Some((String::new(), String::new()));
            }
            return;
        };
        let mut add = false;
        let mut cancel = false;
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0_f32, theme::BORDER_STRONG()))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.label(RichText::new("Add a step").strong());
                ui.label(theme::muted(
                    "It runs after Build and Check and before the pull request.",
                ));
                ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("Name, e.g. Security review")
                        .desired_width(f32::INFINITY),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut prompt)
                        .hint_text("What should the agent do at this step?")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                let ready = !name.trim().is_empty() && !prompt.trim().is_empty();
                ui.horizontal(|ui| {
                    add = ui.add_enabled(ready, theme::primary("Add step")).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if add {
            self.add_step_to(pipeline, &name, &prompt);
        } else if !cancel {
            self.new_step = Some((name, prompt));
        }
    }

    fn add_step_to(&mut self, pipeline: &Pipeline, name: &str, prompt: &str) {
        let Ok(profile) = &self.profile else {
            return;
        };
        let Some(slot) = free_custom_slot(profile) else {
            self.pipeline_notice = Some((
                false,
                format!("all {} custom steps are used", Step::CUSTOM_SLOTS),
            ));
            return;
        };
        // The new step goes into a copy when the open pipeline is built in.
        let target = if pipeline.builtin {
            self.duplicate_pipeline(&pipeline.name);
            self.pipeline_selected.clone()
        } else {
            Some(pipeline.name.clone())
        };
        let Some(target) = target else {
            return;
        };
        if let Err(error) = set_custom_step(&self.options.profile, slot, name, prompt) {
            self.pipeline_notice = Some((false, error.to_string()));
            return;
        }
        let Some(current) = autopilot_engine::load_profile(&self.options.profile)
            .ok()
            .and_then(|profile| profile.pipeline(&target))
        else {
            return;
        };
        let mut modes: Vec<(Step, StepMode)> = Step::ALL
            .iter()
            .map(|step| (*step, current.mode(*step)))
            .collect();
        modes.extend(
            current
                .custom_steps()
                .into_iter()
                .map(|step| (step, StepMode::On)),
        );
        modes.push((slot, StepMode::On));
        self.write_pipeline(&target, &modes, current.rounds);
        self.new_step = None;
        self.step_open = Some(slot);
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
            theme::BLUE_FILL()
        } else {
            Color32::TRANSPARENT
        })
        // A thicker blue edge is what tells the owner which pipeline the
        // detail on the right belongs to.
        .stroke(if chosen {
            egui::Stroke::new(2.0_f32, theme::BLUE())
        } else {
            egui::Stroke::new(1.0_f32, theme::BORDER())
        })
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
