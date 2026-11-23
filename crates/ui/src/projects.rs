//! Projects: one entry per workspace folder, from the profile and from the
//! folders tickets have used, with the project's pipeline, rules and limits.
//! A project is keyed by its full path; the folder name is only a label.

use autopilot_engine::{ProjectField, TicketView, edit_project};
use autopilot_profile::{leaf, normalize_path};
use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;

/// What the page holds about the selected project's card: its saved text, the
/// last scan's notice, and what that scan offered. Read when a project is
/// chosen or scanned, never per frame.
#[derive(Default)]
pub(crate) struct CardState {
    pub(crate) text: Option<String>,
    pub(crate) notice: Option<(bool, String)>,
    pub(crate) commands: Vec<autopilot_engine::DetectedCommand>,
    pub(crate) tools: Vec<autopilot_engine::ToolSuggestion>,
}

/// One row of the list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub path: String,
    pub name: String,
    pub registered: bool,
    pub tickets: usize,
    pub needs_you: usize,
    pub default_pipeline: Option<String>,
}

/// The projects to list: every entry in the profile, then every folder a
/// ticket has used that the profile does not know yet. Tickets with no folder
/// are counted apart as "no project".
#[must_use]
pub fn rows(
    profile: Option<&autopilot_profile::Profile>,
    tickets: &[TicketView],
) -> (Vec<Row>, usize) {
    let mut rows: Vec<Row> = profile
        .map(|profile| {
            profile
                .projects
                .values()
                .map(|project| Row {
                    path: project.path.clone(),
                    name: project.name.clone(),
                    registered: true,
                    tickets: 0,
                    needs_you: 0,
                    default_pipeline: project.default_pipeline.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut without = 0;
    for view in tickets {
        let Some(folder) = &view.workspace else {
            without += 1;
            continue;
        };
        let path = normalize_path(folder);
        let found = rows.iter().position(|row| row.path == path);
        let at = if let Some(at) = found {
            at
        } else {
            rows.push(Row {
                name: leaf(&path),
                path,
                registered: false,
                tickets: 0,
                needs_you: 0,
                default_pipeline: None,
            });
            rows.len() - 1
        };
        rows[at].tickets += 1;
        if view.badge == autopilot_engine::Badge::NeedsYou {
            rows[at].needs_you += 1;
        }
    }
    (rows, without)
}

impl App {
    pub(crate) fn projects_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Projects").size(18.0).strong());
        ui.add_space(8.0);
        let (list, without) = rows(self.profile.as_ref().ok(), &self.snapshot.tickets);
        ui.horizontal(|ui| {
            if crate::composer::has_picker() {
                if ui.button("New project").clicked()
                    && let Some(folder) = crate::composer::pick_project()
                {
                    self.add_project(&folder);
                }
            } else {
                ui.label(theme::muted(
                    "New project needs the folder picker (Windows, macOS).",
                ));
            }
        });
        ui.add_space(8.0);
        if self.project_selected.is_none()
            && let Some(first) = list.first()
        {
            self.select_project(&first.path);
        }
        let mut pick = None;
        let width = (ui.available_width() * 0.28).clamp(240.0, 320.0);
        let selected_row = self
            .project_selected
            .as_deref()
            .and_then(|path| list.iter().find(|row| row.path == path))
            .cloned();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for row in &list {
                        let chosen = self.project_selected.as_deref() == Some(row.path.as_str());
                        if project_card(ui, row, chosen) {
                            pick = Some(row.path.clone());
                        }
                    }
                    egui::Frame::new()
                        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new("no project")
                                    .monospace()
                                    .size(13.0)
                                    .strong()
                                    .color(theme::TEXT_SOFT()),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{without} tickets · ask-only: words and attachments only"
                                ))
                                .monospace()
                                .size(11.0)
                                .color(theme::TEXT_MUTED()),
                            );
                        });
                },
            );
            ui.vertical(|ui| {
                if let Some(row) = &selected_row {
                    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.project_detail(ui, row);
                    });
                }
            });
        });
        if let Some(path) = pick {
            self.select_project(&path);
        }
    }

    /// Choose a project and load its rules into the editor.
    pub(crate) fn select_project(&mut self, path: &str) {
        self.project_selected = Some(path.to_owned());
        self.revert_rules();
        self.card.notice = None;
        self.card.text = autopilot_engine::read_card(&self.options.data_dir, path)
            .ok()
            .flatten();
        let scanned = autopilot_engine::scan_workspace(std::path::Path::new(path));
        self.card.tools = autopilot_engine::suggest_tools(&scanned);
        self.card.commands = scanned.commands;
    }

    pub(crate) fn revert_rules(&mut self) {
        let Some(path) = self.project_selected.clone() else {
            return;
        };
        self.rules_text = autopilot_engine::read_rules(&self.options.data_dir, &path)
            .ok()
            .flatten()
            .unwrap_or_default();
        self.rules_saved = self.rules_text.clone();
        self.rules_notice = None;
    }

    /// Register a folder in the profile, as the history spells it.
    pub(crate) fn add_project(&mut self, folder: &std::path::Path) {
        let path = autopilot_engine::canonical_folder(folder)
            .unwrap_or_else(|| folder.to_string_lossy().into_owned());
        match edit_project(&self.options.profile, &path, None) {
            Ok(profile) => {
                self.profile = Ok(profile);
                self.select_project(&normalize_path(&path));
                // A new project is read once, so its page opens with a card.
                self.scan_project();
            }
            Err(error) => self.notice = Some((false, error.to_string())),
        }
    }

    /// Read the selected project's folder into its card. A refusal says why
    /// and leaves the last card as it was.
    pub(crate) fn scan_project(&mut self) {
        let Some(path) = self.project_selected.clone() else {
            return;
        };
        match autopilot_engine::get_to_know(&self.options.data_dir, &path) {
            Ok((card, info)) => {
                self.card.text = autopilot_engine::read_card(&self.options.data_dir, &path)
                    .ok()
                    .flatten();
                let found = card.commands.len();
                self.card.tools = autopilot_engine::suggest_tools(&card);
                self.card.commands = card.commands;
                self.card.notice = Some((
                    true,
                    format!("scanned, {} bytes, {found} commands found", info.bytes),
                ));
            }
            Err(error) => self.card.notice = Some((false, error.to_string())),
        }
    }

    /// Save the editor text as the project's rules. A refusal says why and
    /// leaves the saved rules as they were.
    pub(crate) fn save_project_rules(&mut self) {
        let Some(path) = self.project_selected.clone() else {
            return;
        };
        match autopilot_engine::save_rules(&self.options.data_dir, &path, &self.rules_text) {
            Ok(info) => {
                self.rules_saved = self.rules_text.clone();
                self.rules_notice = Some((true, format!("saved, {} bytes", info.bytes)));
            }
            Err(error) => self.rules_notice = Some((false, error.to_string())),
        }
    }

    fn project_detail(&mut self, ui: &mut egui::Ui, row: &Row) {
        ui.label(RichText::new(&row.name).monospace().size(14.0).strong());
        ui.label(theme::mono(&row.path).size(11.0).color(theme::TEXT_MUTED()));
        if !row.registered && ui.button("Add to profile").clicked() {
            let folder = std::path::PathBuf::from(&row.path);
            self.add_project(&folder);
        }
        ui.add_space(8.0);
        self.pipeline_choice(ui, row);
        if let Ok(profile) = &self.profile {
            let scoped = profile.for_project(Some(&row.path));
            let project = profile.project(&row.path);
            ui.label(theme::muted(format!(
                "protected_paths  {}",
                if scoped.protected_paths.is_empty() {
                    "none".to_owned()
                } else {
                    scoped.protected_paths.join(", ")
                }
            )));
            let budget = |value: Option<std::num::NonZeroU64>| {
                value.map_or_else(
                    || "no limit".to_owned(),
                    |micro| {
                        autopilot_engine::dollars(autopilot_core::Money::from_microusd(micro.get()))
                    },
                )
            };
            ui.label(theme::muted(format!(
                "budget per ticket  {}   per day  {}",
                budget(scoped.budgets.per_ticket_microusd),
                budget(scoped.budgets.daily_microusd)
            )));
            if let Some(repo) = project.and_then(|project| project.github_repo.as_deref()) {
                ui.label(theme::muted(format!("github  {repo}")));
            }
            ui.label(theme::muted(
                "A project adds protected paths and lowers budgets. It never removes either.",
            ));
        }
        ui.add_space(12.0);
        self.card_section(ui, row);
        ui.add_space(12.0);
        self.rules_editor(ui);
    }

    /// Add one command the scan offered to `[checks]`. The engine checks it
    /// against a fresh scan and refuses a name the owner already uses.
    pub(crate) fn add_detected_check(&mut self, command: &str) {
        let Some(path) = self.project_selected.clone() else {
            return;
        };
        match autopilot_engine::add_detected_check(&self.options.profile, &path, command) {
            Ok(profile) => {
                self.profile = Ok(profile);
                self.card.notice = Some((true, format!("added `{command}` to [checks]")));
            }
            Err(error) => self.card.notice = Some((false, error.to_string())),
        }
    }

    /// Skills and MCP servers the scan suggests. Advice only: nothing here
    /// installs or enables anything.
    fn suggested_tools(&self, ui: &mut egui::Ui) {
        if self.card.tools.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::muted("Suggested tools"));
            theme::pill(ui, "advice only", theme::GREY_FILL(), theme::GREY_TEXT());
        });
        for tool in &self.card.tools {
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tool.name).strong());
                        theme::pill(ui, tool.kind, theme::BLUE_FILL(), theme::BLUE_TEXT());
                        if !tool.reviewed {
                            theme::pill(ui, "unreviewed", theme::AMBER_FILL(), theme::AMBER());
                        }
                    });
                    ui.label(theme::muted(tool.summary));
                    ui.label(theme::muted(format!(
                        "{} · {} · {} · {} (looked at {})",
                        tool.maintainer, tool.licence, tool.source, tool.pinned, tool.checked_on
                    )));
                    ui.label(theme::muted(format!("can: {}", tool.permissions)));
                });
        }
        ui.label(theme::muted(
            "Read the source and what it can do before you use one. Autopilot steps run without MCP servers, and this does not change that.",
        ));
    }

    fn detected_commands(&mut self, ui: &mut egui::Ui) {
        if self.card.commands.is_empty() {
            return;
        }
        let set: Vec<String> = self
            .profile
            .as_ref()
            .map(|profile| profile.checks.keys().cloned().collect())
            .unwrap_or_default();
        let mut chosen = None;
        ui.label(theme::muted("Detected commands, not yet checks:"));
        for found in &self.card.commands {
            ui.horizontal(|ui| {
                theme::cell(ui, 60.0, found.kind);
                ui.label(theme::mono(&found.command).size(12.0));
                if set.iter().any(|name| name == found.kind) {
                    theme::pill(ui, "check set", theme::GREY_FILL(), theme::GREY_TEXT());
                } else if ui.button("Add as check").clicked() {
                    chosen = Some(found.command.clone());
                }
            });
        }
        if let Some(command) = chosen {
            self.add_detected_check(&command);
        }
    }

    fn card_section(&mut self, ui: &mut egui::Ui, row: &Row) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("project card").strong());
            theme::pill(ui, "read-only scan", theme::GREY_FILL(), theme::GREY_TEXT());
            theme::pill(ui, "not committed", theme::GREY_FILL(), theme::GREY_TEXT());
        });
        match &self.card.text {
            Some(text) => {
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(theme::mono(text).size(12.0));
                    });
                ui.label(theme::muted(format!(
                    "~{} tokens   {} of {} bytes",
                    text.len().div_ceil(4),
                    text.len(),
                    autopilot_engine::MAX_CARD_BYTES
                )));
            }
            None => {
                ui.label(theme::muted(
                    "No card yet. Scan reads manifest names, not source.",
                ));
            }
        }
        self.detected_commands(ui);
        self.suggested_tools(ui);
        let label = if self.card.text.is_some() {
            "Scan again"
        } else {
            "Scan"
        };
        if row.registered && ui.button(label).clicked() {
            self.scan_project();
        }
        if let Some((ok, text)) = &self.card.notice {
            let color = if *ok { theme::GREEN() } else { theme::RED() };
            ui.label(RichText::new(text).color(color));
        }
        ui.label(theme::muted(
            "Kept outside the project. Commands are suggestions: nothing becomes a check until you add it to [checks].",
        ));
    }

    fn pipeline_choice(&mut self, ui: &mut egui::Ui, row: &Row) {
        let Ok(profile) = &self.profile else {
            return;
        };
        let names: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
        let current = row
            .default_pipeline
            .clone()
            .unwrap_or_else(|| autopilot_profile::DEFAULT_PIPELINE.to_owned());
        let mut chosen = current.clone();
        ui.horizontal(|ui| {
            ui.label("default pipeline");
            egui::ComboBox::from_id_salt("default-pipeline")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for name in &names {
                        ui.selectable_value(&mut chosen, name.clone(), name);
                    }
                });
        });
        if chosen != current {
            match edit_project(
                &self.options.profile,
                &row.path,
                Some(&ProjectField::DefaultPipeline(Some(chosen))),
            ) {
                Ok(profile) => self.profile = Ok(profile),
                Err(error) => self.notice = Some((false, error.to_string())),
            }
        }
    }

    fn rules_editor(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("rules.md").strong());
            theme::pill(
                ui,
                "always included",
                theme::GREY_FILL(),
                theme::GREY_TEXT(),
            );
            theme::pill(ui, "not committed", theme::GREY_FILL(), theme::GREY_TEXT());
            if self.rules_text != self.rules_saved {
                theme::pill(ui, "unsaved", theme::AMBER_FILL(), theme::AMBER());
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.rules_text)
                .desired_rows(8)
                .desired_width(f32::INFINITY)
                .hint_text("Style, wording, what to ask before touching. Guidance only."),
        );
        ui.label(theme::muted(format!(
            "~{} tokens per step   {} of {} bytes",
            self.rules_text.len().div_ceil(4),
            self.rules_text.len(),
            autopilot_engine::MAX_RULES_BYTES
        )));
        ui.horizontal(|ui| {
            if ui.add(theme::primary("Save")).clicked() {
                self.save_project_rules();
            }
            if ui.button("Revert").clicked() {
                self.revert_rules();
            }
        });
        if let Some((ok, text)) = &self.rules_notice {
            let color = if *ok { theme::GREEN() } else { theme::RED() };
            ui.label(RichText::new(text).color(color));
        }
        ui.label(theme::muted(
            "Kept outside the project. It guides the agent and cannot loosen what asks, what is protected or any budget.",
        ));
    }
}

/// One project in the list: name, ticket count, and chips for its default
/// pipeline, tickets that need the owner, or a project not yet in the profile.
fn project_card(ui: &mut egui::Ui, row: &Row, chosen: bool) -> bool {
    let shown = egui::Frame::new()
        .fill(if chosen {
            theme::BLUE_FILL()
        } else {
            egui::Color32::TRANSPARENT
        })
        // The same blue edge as the Pipelines list marks the chosen row.
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
                ui.label(RichText::new(&row.name).monospace().size(13.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} tickets", row.tickets))
                            .monospace()
                            .size(11.0)
                            .color(theme::TEXT_MUTED()),
                    );
                });
            });
            ui.horizontal_wrapped(|ui| {
                if let Some(name) = &row.default_pipeline {
                    theme::pill(ui, name, theme::BLUE_FILL(), theme::BLUE_TEXT());
                }
                if row.needs_you > 0 {
                    theme::pill(
                        ui,
                        &format!("{} need you", row.needs_you),
                        theme::AMBER_FILL(),
                        theme::AMBER(),
                    );
                }
                if !row.registered {
                    theme::pill(ui, "not in profile", theme::GREY_FILL(), theme::GREY_TEXT());
                }
            });
        });
    let click = ui.interact(
        shown.response.rect,
        ui.id().with(("project-card", row.path.as_str())),
        egui::Sense::click(),
    );
    theme::named(click, &row.name)
        .on_hover_text(&row.path)
        .clicked()
}
