//! First run: a readiness checklist for an empty install, with the command
//! that fixes each missing piece.

use eframe::egui::{self, RichText};

use crate::accounts::{Level, rows};
use crate::app::App;
use crate::backend;
use crate::theme;

impl App {
    /// An install with no tickets and no chosen project folder.
    pub(crate) fn is_first_run(&self) -> bool {
        self.loaded
            && self.snapshot.tickets.is_empty()
            && backend::saved_workspace(&self.options.data_dir).is_none()
    }

    pub(crate) fn first_run_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Get ready").size(30.0).strong());
        ui.label(theme::muted("Three things. GitHub is optional."));
        ui.add_space(10.0);
        let accounts = self.accounts.as_ref().map(rows);
        let find = |name: &str| {
            accounts
                .as_ref()
                .and_then(|list| list.iter().find(|row| row.name == name))
        };
        let agent = find("Claude");
        let agent_ready = agent.is_some_and(|row| row.level == Level::Good)
            || self.agent == backend::AgentChoice::Practice;
        let agent_hint = agent.and_then(|row| row.hint);
        step_card(
            ui,
            "Agent",
            if agent_ready {
                "ready"
            } else if accounts.is_none() {
                "checking..."
            } else {
                "Claude is not signed in"
            },
            agent_ready,
            agent_hint,
        );
        let folder = backend::saved_workspace(&self.options.data_dir);
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
        let github = find("GitHub");
        let github_ready = github.is_some_and(|row| row.level == Level::Good);
        step_card(
            ui,
            "GitHub (optional)",
            if github_ready {
                "signed in"
            } else {
                "not signed in"
            },
            github_ready,
            github.and_then(|row| row.hint),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary("Check again")).clicked() {
                self.accounts = None;
                backend::fetch_accounts(self.context.clone(), self.sender.clone());
            }
            if ui.button("New ticket").clicked() {
                self.start_new();
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
