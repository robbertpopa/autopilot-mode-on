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
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) && !self.busy {
            self.mode = Mode::Watch;
            return;
        }
        ui.label(RichText::new("New ticket").size(30.0).strong());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(theme::muted("kind"));
            theme::segmented(
                ui,
                &mut self.kind,
                &[
                    (Kind::Bug, "Something is broken"),
                    (Kind::Change, "Change or add"),
                    (Kind::Ask, "Just ask"),
                ],
            );
        })
        .response
        .on_hover_text("It will suggest a pipeline once named pipelines exist (R7)");
        ui.add_space(8.0);
        // The folder last chosen stays chosen until it is changed.
        if self.home.workspace.is_none() {
            self.home.workspace = backend::saved_workspace(&self.options.data_dir);
        }
        let before = self.home.workspace.clone();
        self.pipeline_row(ui);
        let start = composer::show(ui, &mut self.home);
        if self.home.workspace != before {
            backend::save_workspace(&self.options.data_dir, self.home.workspace.as_deref());
        }
        if self.agent == backend::AgentChoice::Practice {
            ui.add_space(6.0);
            ui.label(theme::muted(
                "Offline demo agent: the answers are fixed samples. Switch to Claude in Settings > Accounts.",
            ));
        }
        if let Some(action) = start {
            self.home = composer::NewTicket::default();
            self.pipeline_chosen = false;
            self.new_started = true;
            self.mode = Mode::Watch;
            self.act(action);
        }
        ui.add_space(14.0);
        self.what_will_happen(ui);
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

    /// The steps and which of them will ask, read from the profile.
    fn what_will_happen(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("What will happen").size(18.0).strong());
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
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 18.0], egui::Label::new(step_name(step)));
                ui.label(theme::muted(tags.join(" · ")));
                if step == Step::BuildCheck {
                    ui.label(theme::mono(format!(
                        "{} rounds max",
                        settings.limits.rounds
                    )));
                }
            });
        }
        ui.add_space(8.0);
        let budget = profile.budgets.per_ticket_microusd.map_or_else(
            || "no limit set".to_owned(),
            |limit| dollars(autopilot_core::Money::from_microusd(limit.get())),
        );
        ui.label(theme::muted(format!("budget per ticket  {budget}")));
        for (text, why) in [
            ("when_unsure  ask", "needs request R3: a per-ticket setting"),
            (
                "rules.md  always on · not committed",
                "needs request R13: the per-project rules file",
            ),
        ] {
            ui.add_enabled(false, egui::Label::new(text))
                .on_disabled_hover_text(why);
        }
    }
}
