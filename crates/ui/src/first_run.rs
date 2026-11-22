//! First run: a readiness checklist for an empty install, with the command
//! that fixes each missing piece.

use eframe::egui::{self, RichText};

use crate::accounts::{Level, rows};
use crate::app::App;
use crate::backend;
use crate::theme;

impl App {
    /// Shown once, on the very first start: an install with no tickets, no
    /// chosen folder and a first screen not yet dealt with.
    pub(crate) fn is_first_run(&self) -> bool {
        self.loaded
            && !self.welcomed
            && self.snapshot.tickets.is_empty()
            && self.saved_workspace.is_none()
    }

    /// Remember that the first screen is done, so it never comes back.
    fn finish_welcome(&mut self) {
        self.welcomed = true;
        backend::mark_welcomed(&self.options.data_dir);
    }

    pub(crate) fn first_run_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Welcome").size(18.0).strong());
        ui.label(theme::muted(
            "Pick how it works. Sign-in stays in each tool's own command; no password or token is kept here.",
        ));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(theme::muted("agent"));
            let before = self.agent;
            let mut options = vec![(backend::AgentChoice::Claude, "Claude")];
            if self.sample_data || self.agent == backend::AgentChoice::Practice {
                options.push((backend::AgentChoice::Practice, "Offline demo"));
            }
            theme::segmented(ui, &mut self.agent, &options);
            if self.agent != before {
                self.agent.save(&self.options.data_dir);
            }
            ui.add_space(12.0);
            ui.label(theme::muted("theme"));
            let before = self.theme;
            let mut chosen = self.theme;
            theme::segmented(
                ui,
                &mut chosen,
                &[
                    (theme::ThemeChoice::System, "System"),
                    (theme::ThemeChoice::Light, "Light"),
                    (theme::ThemeChoice::Dark, "Dark"),
                ],
            );
            if chosen != before {
                self.choose_theme(chosen);
            }
        });
        ui.add_space(10.0);
        match self.accounts.clone() {
            None => {
                ui.label(theme::muted("Checking GitHub, Claude, Codex and Azure..."));
            }
            Some(accounts) => {
                for row in rows(&accounts) {
                    let ready = row.level == Level::Good;
                    let state = if ready {
                        row.lines.first().cloned().unwrap_or_default()
                    } else if row.level == Level::Missing {
                        "not installed".to_owned()
                    } else {
                        "not signed in".to_owned()
                    };
                    step_card(ui, row.name, &state, ready, row.hint);
                }
            }
        }
        let folder = self.saved_workspace.clone();
        step_card(
            ui,
            "Project folder",
            &folder.as_ref().map_or_else(
                || "none chosen: pick one on New ticket".to_owned(),
                |path| path.display().to_string(),
            ),
            folder.is_some(),
            None,
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary("Start")).clicked() {
                self.finish_welcome();
                self.start_new();
            }
            if ui.button("Check again").clicked() {
                self.accounts = None;
                backend::fetch_accounts(self.context.clone(), self.sender.clone());
            }
            if ui.button("Skip").clicked() {
                self.finish_welcome();
            }
        });
    }
}

fn step_card(ui: &mut egui::Ui, name: &str, state: &str, ready: bool, hint: Option<&str>) {
    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
        ui.set_width(ui.available_width() - 38.0);
        ui.horizontal(|ui| {
            let mark = if ready { "✔" } else { "×" };
            let color = if ready {
                theme::GREEN()
            } else {
                theme::AMBER()
            };
            ui.label(RichText::new(mark).color(color));
            ui.label(RichText::new(name).strong());
            ui.label(theme::muted(state));
        });
        if let (false, Some(hint)) = (ready, hint) {
            ui.horizontal(|ui| {
                ui.label(theme::muted("To fix, run:"));
                ui.label(theme::mono(hint));
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(hint.to_owned());
                }
            });
        }
    });
    ui.add_space(6.0);
}
