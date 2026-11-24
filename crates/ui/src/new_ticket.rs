//! New ticket: a kind, the words, attachments and a project folder on the
//! left; on the right what will happen, read from the profile.

use autopilot_core::Step;
use autopilot_engine::{dollars, step_name};
use eframe::egui::{self, RichText};

use crate::app::{App, Mode};
use crate::backend;
use crate::composer;
use crate::theme;

/// What the ticket is about. It suggests a pipeline once named pipelines
/// exist (R7); until then it is not stored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Kind {
    Bug,
    #[default]
    Change,
    Ask,
}

/// Steps whose run depends on what Understand finds.
const fn known_after_understand(step: Step) -> bool {
    matches!(step, Step::Clarify | Step::Design)
}

/// Design and the pull request always ask, whatever the profile says.
const fn locked(step: Step) -> bool {
    matches!(step, Step::Design | Step::PullRequest)
}

impl App {
    pub(crate) fn new_ticket_screen(&mut self, ui: &mut egui::Ui) {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) && !self.busy() {
            self.mode = Mode::Watch;
            return;
        }
        ui.label(RichText::new("New ticket").size(18.0).strong());
        ui.add_space(8.0);
        // The folder last chosen stays chosen until it is changed.
        if self.home.workspace.is_none() {
            self.home.workspace.clone_from(&self.saved_workspace);
        }
        let before = self.home.workspace.clone();
        // Below this width the side card goes under the main column instead of being cut off.
        let narrow = ui.available_width() < 820.0;
        let aside = if narrow {
            ui.available_width()
        } else {
            340.0_f32.min(ui.available_width() * 0.4)
        };
        let main = if narrow {
            ui.available_width()
        } else {
            ui.available_width() - aside - 24.0
        };
        let mut start = None;
        let columns = egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true);
        ui.with_layout(columns, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(16.0, 12.0);
            ui.allocate_ui_with_layout(
                egui::vec2(main, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(main);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.horizontal_wrapped(|ui| {
                        self.pipeline_row(ui);
                        theme::segmented(
                            ui,
                            &mut self.kind,
                            &[
                                (Kind::Bug, "Something is broken"),
                                (Kind::Change, "Change or add"),
                                (Kind::Ask, "Just ask"),
                            ],
                        );
                    });
                    start = composer::show(ui, &mut self.home);
                    if self.agent == backend::AgentChoice::Practice {
                        ui.label(theme::muted(
                            "Offline demo agent: the answers are fixed samples. Switch to Claude in Settings > Accounts.",
                        ));
                    }
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(aside, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(aside);
                    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.what_will_happen(ui);
                    });
                },
            );
        });
        if self.home.workspace != before {
            backend::save_workspace(&self.options.data_dir, self.home.workspace.as_deref());
            self.saved_workspace.clone_from(&self.home.workspace);
        }
        if let Some(action) = start {
            self.home = composer::NewTicket::default();
            self.pipeline_chosen = false;
            self.mode = Mode::Watch;
            self.act(action);
        }
    }

    /// The pipeline a ticket of this kind starts with: the project's default
    /// for a change, bug-fix for a bug, ask-only to just ask.
    fn suggestion(&self) -> String {
        match self.kind {
            Kind::Bug => crate::pipelines::suggested(Kind::Bug).to_owned(),
            Kind::Ask => crate::pipelines::suggested(Kind::Ask).to_owned(),
            Kind::Change => self
                .home
                .workspace
                .as_ref()
                .and_then(|folder| {
                    let path = folder.to_string_lossy();
                    self.profile
                        .as_ref()
                        .ok()
                        .and_then(|profile| profile.project(&path))
                        .and_then(|project| project.default_pipeline.clone())
                })
                .unwrap_or_else(|| crate::pipelines::suggested(Kind::Change).to_owned()),
        }
    }

    /// The pipeline select. With no project folder the ticket is ask-only and
    /// the select is locked; otherwise it shows the suggestion until the owner
    /// picks one.
    fn pipeline_row(&mut self, ui: &mut egui::Ui) {
        let Ok(profile) = &self.profile else {
            return;
        };
        let names: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
        let locked = self.home.workspace.is_none();
        if !self.pipeline_chosen {
            self.home.pipeline = Some(self.suggestion());
        }
        let current = if locked {
            autopilot_profile::ASK_ONLY.to_owned()
        } else {
            self.home.pipeline.clone().unwrap_or_default()
        };
        let mut chosen = current.clone();
        ui.horizontal(|ui| {
            ui.label(theme::muted("pipeline"));
            ui.add_enabled_ui(!locked, |ui| {
                egui::ComboBox::from_id_salt("new-ticket-pipeline")
                    .selected_text(&current)
                    .show_ui(ui, |ui| {
                        for name in &names {
                            ui.selectable_value(&mut chosen, name.clone(), name);
                        }
                    });
            });
            ui.label(theme::muted(if locked {
                "no project: ask-only"
            } else if self.pipeline_chosen {
                "chosen"
            } else {
                "suggested"
            }));
        });
        if !locked && chosen != current {
            self.home.pipeline = Some(chosen);
            self.pipeline_chosen = true;
        }
    }

    /// Whether the chosen project has a rules file, and how big it is. Never
    /// its text.
    fn rules_row(&self, ui: &mut egui::Ui) {
        let Some(folder) = &self.home.workspace else {
            ui.label(theme::muted("rules.md  no project"));
            return;
        };
        let path = autopilot_engine::canonical_folder(folder)
            .unwrap_or_else(|| folder.to_string_lossy().into_owned());
        let text = match autopilot_engine::rules_info(&self.options.data_dir, &path) {
            Ok(Some(info)) => format!(
                "rules.md  always on · not committed · {} bytes, ~{} tokens per step",
                info.bytes, info.tokens_estimate
            ),
            Ok(None) => "rules.md  none for this project (add one on Projects)".to_owned(),
            Err(error) => format!("rules.md  {error}"),
        };
        ui.label(theme::muted(text));
    }

    /// The steps and which of them will ask, read from the profile.
    /// How Design gets to know the application: Claude reading the project
    /// (read-only), or the ticket's words alone.
    pub(crate) fn design_mode_picker(&self, ui: &mut egui::Ui) {
        // Without Claude there is no choice to make: the practice agent draws
        // fixed text.
        if self.agent != backend::AgentChoice::Claude {
            return;
        }
        let reads = backend::design_reads_project(&self.options.data_dir);
        let mut choice = reads;
        ui.add_space(6.0);
        ui.label(theme::muted("Design draws from"));
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut choice, true, "Read the project")
                .on_hover_text(
                    "Claude reads the application first (read-only): its screens, colours, fonts and names, so the mockups match the real pages. Costs more.",
                );
            ui.selectable_value(&mut choice, false, "Normal")
                .on_hover_text(
                    "A plain request from the ticket's words alone. Cheaper, but it cannot see the application.",
                );
        });
        if choice != reads {
            backend::save_design_reads_project(&self.options.data_dir, choice);
        }
    }

    fn what_will_happen(&self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("WHAT WILL HAPPEN")
                .monospace()
                .size(11.0)
                .strong()
                .color(theme::TEXT_MUTED()),
        );
        let Ok(profile) = &self.profile else {
            ui.label(theme::muted("The profile could not be read."));
            return;
        };
        for step in Step::ALL {
            let Some(settings) = profile.step(step) else {
                continue;
            };
            let mut tags = Vec::new();
            if known_after_understand(step) {
                tags.push("known after Understand");
            }
            if locked(step) {
                tags.push("ask · locked");
            } else if settings.ask_after {
                tags.push("ask");
            } else if step == Step::BuildCheck {
                tags.push("runs");
            }
            // Wrapped, so a long tag line goes under the name instead of widening
            // the card past the window.
            ui.horizontal_wrapped(|ui| {
                theme::cell(
                    ui,
                    110.0,
                    RichText::new(step_name(step)).monospace().size(12.0),
                );
                ui.label(theme::muted(tags.join(" · ")));
                if step == Step::BuildCheck {
                    ui.label(theme::mono(format!(
                        "{} rounds max",
                        settings.limits.rounds
                    )));
                }
            });
        }
        self.design_mode_picker(ui);
        ui.add_space(8.0);
        let budget = profile.budgets.per_ticket_microusd.map_or_else(
            || "no limit set".to_owned(),
            |limit| dollars(autopilot_core::Money::from_microusd(limit.get())),
        );
        ui.label(theme::muted(format!("budget per ticket  {budget}")));
        ui.add_enabled(false, egui::Label::new("when_unsure  ask"))
            .on_disabled_hover_text("recorded per ticket, but no step reads it yet");
        self.rules_row(ui);
    }
}
