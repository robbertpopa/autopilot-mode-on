//! Settings pages that are new in the redesign: when it asks you, checks and
//! look. Edits to `ask_after` collect here and are written on Save, after the
//! owner has seen the diff.

use autopilot_core::Step;
use autopilot_engine::{StepField, step_name};
use autopilot_profile::WhenUnsure;
use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;
use crate::theme::ThemeChoice;

/// Design and the pull request always ask, so their switch is not editable.
#[must_use]
pub const fn locked(step: Step) -> bool {
    matches!(step, Step::Design | Step::PullRequest)
}

impl App {
    fn ask_value(&self, step: Step) -> bool {
        if locked(step) {
            return true;
        }
        self.ask_edits.get(&step).copied().unwrap_or_else(|| {
            self.profile
                .as_ref()
                .ok()
                .and_then(|profile| profile.step(step))
                .is_some_and(|settings| settings.ask_after)
        })
    }

    fn ask_saved(&self, step: Step) -> bool {
        self.profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.step(step))
            .is_some_and(|settings| settings.ask_after)
    }

    /// The edits that differ from what the profile holds now.
    pub(crate) fn ask_changes(&self) -> Vec<(Step, bool)> {
        self.ask_edits
            .iter()
            .filter(|(step, value)| !locked(**step) && self.ask_saved(**step) != **value)
            .map(|(step, value)| (*step, *value))
            .collect()
    }

    /// Write every collected change to `profile.toml`, then forget them.
    pub(crate) fn save_asking(&mut self) {
        for (step, value) in self.ask_changes() {
            self.apply_edit(step, &StepField::AskAfter(value));
            if self.profile_problem.is_some() {
                return;
            }
        }
        self.ask_edits.clear();
    }

    pub(crate) fn asking_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(RichText::new("When it asks you").size(30.0).strong());
        ui.label(theme::muted("pipeline · ask_after"));
        ui.add_space(10.0);
        for step in Step::ALL {
            ui.horizontal(|ui| {
                ui.add_sized([170.0, 22.0], egui::Label::new(step_name(step)));
                if locked(step) {
                    ui.add_enabled(
                        false,
                        egui::Checkbox::new(&mut true.clone(), "ask · locked"),
                    )
                    .on_disabled_hover_text("Design and the pull request always ask");
                } else {
                    let mut value = self.ask_value(step);
                    if ui.checkbox(&mut value, "ask after this step").changed() {
                        self.ask_edits.insert(step, value);
                    }
                }
            });
        }
        if let Some((step, text)) = &self.profile_problem {
            ui.label(RichText::new(format!("{}: {text}", step_name(*step))).color(theme::RED()));
        }
        self.save_bar(ui);
        ui.add_space(14.0);
        self.when_unsure_cards(ui);
        ui.add_space(14.0);
        self.forced_rules(ui);
    }

    /// The diff first, then Save. Nothing is written before.
    fn save_bar(&mut self, ui: &mut egui::Ui) {
        let changes = self.ask_changes();
        if changes.is_empty() {
            return;
        }
        ui.add_space(10.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 38.0);
            ui.label(RichText::new("Unsaved changes").strong());
            for (step, value) in &changes {
                ui.label(theme::mono(format!(
                    "{}  ask_after  {} → {}",
                    step_name(*step),
                    !value,
                    value
                )));
            }
            ui.horizontal(|ui| {
                if ui.add(theme::primary("Save")).clicked() {
                    self.save_asking();
                }
                if ui.button("Discard").clicked() {
                    self.ask_edits.clear();
                }
            });
        });
    }

    /// The project's `when_unsure`, shown as two cards. Editing it per ticket
    /// needs request R3; the project value is a line in `profile.toml`.
    fn when_unsure_cards(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("when_unsure").size(18.0).strong());
        let current = self
            .profile
            .as_ref()
            .ok()
            .map(|profile| profile.when_unsure);
        ui.horizontal(|ui| {
            for (value, title, text) in [
                (WhenUnsure::Ask, "ask", "Stop and ask you."),
                (
                    WhenUnsure::DecideAndTell,
                    "decide",
                    "Pick the recommended option and say so.",
                ),
            ] {
                let chosen = current == Some(value);
                theme::card(
                    if chosen {
                        theme::SELECTED()
                    } else {
                        theme::CARD()
                    },
                    if chosen {
                        theme::BLUE()
                    } else {
                        theme::BORDER()
                    },
                )
                .show(ui, |ui| {
                    ui.set_width(220.0);
                    ui.label(RichText::new(title).strong());
                    ui.label(theme::muted(text));
                });
            }
        });
        ui.label(theme::muted(
            "Change it in profile.toml (when_unsure). A per-ticket setting needs request R3.",
        ));
    }

    /// What forces an ask whatever the switches say, read from the profile.
    fn forced_rules(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Always asks").size(18.0).strong());
        ui.label("Design and the pull request, every time.");
        let Ok(profile) = &self.profile else {
            return;
        };
        if profile.protected_paths.is_empty() {
            ui.label(theme::muted("protected_paths: none set"));
        } else {
            ui.label(theme::muted(format!(
                "A change to protected_paths asks: {}",
                profile.protected_paths.join(", ")
            )));
        }
        ui.label(theme::muted(
            "A spend limit that would be crossed asks. Raise limits under Cost.",
        ));
    }

    pub(crate) fn checks_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(RichText::new("Checks").size(30.0).strong());
        ui.label(theme::muted(
            "Commands run after each Build round, from profile.toml.",
        ));
        ui.add_space(10.0);
        let Ok(profile) = &self.profile else {
            ui.label(theme::muted("The profile could not be read."));
            return;
        };
        if profile.checks.is_empty() {
            ui.label(theme::muted("No checks are set."));
        }
        for (name, command) in &profile.checks {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [120.0, 20.0],
                    egui::Label::new(RichText::new(name).strong()),
                );
                ui.label(theme::mono(format!(
                    "{} {}",
                    command.executable,
                    command.args.join(" ")
                )));
            });
        }
    }

    pub(crate) fn look_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(RichText::new("Look").size(30.0).strong());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Theme");
            let before = self.theme;
            let mut chosen = self.theme;
            theme::segmented(
                ui,
                &mut chosen,
                &[
                    (ThemeChoice::System, "System"),
                    (ThemeChoice::Light, "Light"),
                    (ThemeChoice::Dark, "Dark"),
                ],
            );
            if chosen != before {
                self.choose_theme(chosen);
            }
        });
        ui.add_space(8.0);
        if ui
            .checkbox(
                &mut self.notify,
                "Show a desktop notification when a ticket needs me, fails or finishes",
            )
            .changed()
        {
            crate::backend::save_notifications(&self.options.data_dir, self.notify);
        }
    }
}
