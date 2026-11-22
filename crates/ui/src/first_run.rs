//! First run: a readiness checklist for an empty install, with the command
//! that fixes each missing piece.

use eframe::egui::{self, RichText};

use crate::accounts::{Level, rows};
use crate::app::App;
use crate::backend;
use crate::theme;

/// Environment variable the API steps read the dedicated key from. The app
/// never stores or shows the key.
fn api_key_present() -> bool {
    std::env::var_os(autopilot_agent_claude::API_KEY_SOURCE).is_some_and(|value| !value.is_empty())
}

const STEPS: [&str; 4] = ["Claude", "Other tools", "Project folder", "Look"];

impl App {
    /// Shown once, on the very first start: an install with no tickets that
    /// has not been through the welcome and has no chosen folder yet.
    pub(crate) fn is_first_run(&self) -> bool {
        self.loaded && !self.welcomed && self.snapshot.tickets.is_empty()
    }

    /// Remember that the welcome is done, so it never comes back.
    fn finish_welcome(&mut self) {
        self.welcomed = true;
        backend::mark_welcomed(&self.options.data_dir);
    }

    /// Whether the Claude step may be left: signed in with a subscription, or
    /// a dedicated API key is in the environment.
    fn claude_ready(&self) -> bool {
        let signed_in = self
            .accounts
            .as_ref()
            .map(rows)
            .and_then(|list| list.into_iter().find(|row| row.name == "Claude"))
            .is_some_and(|row| row.level == Level::Good);
        signed_in || api_key_present() || self.agent == backend::AgentChoice::Practice
    }

    /// The whole window is this one screen until it is done: no navigation,
    /// no tickets, nothing else to click.
    pub(crate) fn welcome(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND())
                    .inner_margin(egui::Margin::symmetric(32, 24)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width().min(640.0), 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(ui.available_width());
                            self.welcome_body(ui);
                        },
                    );
                });
            });
    }

    fn welcome_body(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.label(
            RichText::new("autopilot-mode-on")
                .monospace()
                .size(18.0)
                .strong(),
        );
        ui.label(theme::muted(format!(
            "Setup · step {} of {}",
            self.welcome_step + 1,
            STEPS.len()
        )));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (index, name) in STEPS.iter().enumerate() {
                let color = match index.cmp(&self.welcome_step) {
                    std::cmp::Ordering::Equal => theme::TEXT(),
                    std::cmp::Ordering::Less => theme::GREEN(),
                    std::cmp::Ordering::Greater => theme::TEXT_MUTED(),
                };
                ui.label(RichText::new(*name).monospace().size(12.0).color(color));
            }
        });
        ui.add_space(14.0);
        let ready = match self.welcome_step {
            0 => self.welcome_claude(ui),
            1 => self.welcome_tools(ui),
            2 => self.welcome_folder(ui),
            _ => self.welcome_look(ui),
        };
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if self.welcome_step > 0 && ui.button("Back").clicked() {
                self.welcome_step -= 1;
            }
            let last = self.welcome_step + 1 == STEPS.len();
            let label = if last { "Finish" } else { "Next" };
            if ui
                .add_enabled(ready, theme::primary(label))
                .on_disabled_hover_text("Finish this step first")
                .clicked()
            {
                if last {
                    self.finish_welcome();
                    self.start_new();
                } else {
                    self.welcome_step += 1;
                }
            }
            if self.welcome_step < 2 && ui.button("Check again").clicked() {
                self.accounts = None;
                backend::fetch_accounts(self.context.clone(), self.sender.clone());
            }
        });
    }

    /// Step 1: how Claude is paid for. One of the two must be ready.
    fn welcome_claude(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(RichText::new("How should Claude run?").size(14.0).strong());
        ui.label(theme::muted(
            "Use your Claude subscription (signed in through the claude command) or a dedicated API key. Either one is enough.",
        ));
        ui.add_space(8.0);
        let accounts = self.accounts.as_ref().map(rows);
        let claude = accounts
            .as_ref()
            .and_then(|list| list.iter().find(|row| row.name == "Claude"));
        let signed_in = claude.is_some_and(|row| row.level == Level::Good);
        let state = if signed_in {
            claude
                .and_then(|row| row.lines.first().cloned())
                .unwrap_or_else(|| "signed in".to_owned())
        } else if accounts.is_none() {
            "checking...".to_owned()
        } else {
            "not signed in".to_owned()
        };
        step_card(
            ui,
            "Subscription",
            &state,
            signed_in,
            claude.and_then(|row| row.hint),
        );
        let key = api_key_present();
        step_card(
            ui,
            "API key",
            if key {
                "found in the environment"
            } else {
                "not set"
            },
            key,
            Some("setx AUTOPILOT_ANTHROPIC_API_KEY <your key>"),
        );
        ui.label(theme::muted(
            "The app never stores or shows the key: it reads it from that environment variable when an API step runs. Set it once, restart the app, then press Check again.",
        ));
        if self.sample_data {
            ui.add_space(6.0);
            let before = self.agent;
            theme::segmented(
                ui,
                &mut self.agent,
                &[
                    (backend::AgentChoice::Claude, "Claude"),
                    (backend::AgentChoice::Practice, "Offline demo"),
                ],
            );
            if self.agent != before {
                self.agent.save(&self.options.data_dir);
            }
        }
        self.claude_ready()
    }

    /// Step 2: the other tools. All optional, shown so nothing is a surprise.
    fn welcome_tools(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(RichText::new("Other tools").size(14.0).strong());
        ui.label(theme::muted(
            "GitHub opens the pull requests. Codex and Azure are optional. You can sign in later.",
        ));
        ui.add_space(8.0);
        match self.accounts.clone() {
            None => {
                ui.label(theme::muted("Checking GitHub, Codex and Azure..."));
            }
            Some(accounts) => {
                for row in rows(&accounts)
                    .into_iter()
                    .filter(|row| row.name != "Claude")
                {
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
        true
    }

    /// Step 3: the project folder. Required where a folder picker exists.
    fn welcome_folder(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(
            RichText::new("Where does your code live?")
                .size(14.0)
                .strong(),
        );
        ui.label(theme::muted(
            "Pick the folder tickets work in. A ticket with no folder can only ask questions.",
        ));
        ui.add_space(8.0);
        let folder = self.saved_workspace.clone();
        step_card(
            ui,
            "Project folder",
            &folder.as_ref().map_or_else(
                || "none chosen".to_owned(),
                |path| path.display().to_string(),
            ),
            folder.is_some(),
            None,
        );
        if crate::composer::has_picker() {
            if ui.button("Choose a folder").clicked()
                && let Some(chosen) = crate::composer::pick_project()
            {
                backend::save_workspace(&self.options.data_dir, Some(&chosen));
                self.saved_workspace = Some(chosen.clone());
                self.add_project(&chosen);
            }
            folder.is_some()
        } else {
            ui.label(theme::muted(
                "No folder picker on this system: choose the folder on New ticket.",
            ));
            true
        }
    }

    /// Step 4: the look, then done.
    fn welcome_look(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(RichText::new("Pick a look").size(14.0).strong());
        ui.add_space(8.0);
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
        true
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
            // A long path must not widen the card past the column.
            ui.add(egui::Label::new(theme::muted(state)).truncate())
                .on_hover_text(state);
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
