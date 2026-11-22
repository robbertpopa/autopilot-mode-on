//! Projects: one entry per workspace folder, from the profile and from the
//! folders tickets have used, with the project's pipeline, rules and limits.
//! A project is keyed by its full path; the folder name is only a label.

use autopilot_engine::{ProjectField, TicketView, edit_project};
use autopilot_profile::{leaf, normalize_path};
use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;

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
        let mut pick = None;
        for row in &list {
            let chosen = self.project_selected.as_deref() == Some(row.path.as_str());
            let mut text = format!("{}   {} tickets", row.name, row.tickets);
            if row.needs_you > 0 {
                text = format!("{text}   {} need you", row.needs_you);
            }
            if let Some(name) = &row.default_pipeline {
                text = format!("{text}   {name}");
            }
            if !row.registered {
                text = format!("{text}   not in profile");
            }
            let button = egui::Button::new(text)
                .fill(if chosen {
                    theme::SELECTED()
                } else {
                    egui::Color32::TRANSPARENT
                })
                .min_size(egui::vec2(ui.available_width(), 34.0));
            if theme::named(ui.add(button), &row.name)
                .on_hover_text(&row.path)
                .clicked()
            {
                pick = Some(row.path.clone());
            }
        }
        ui.label(theme::muted(format!(
            "no project   {without} tickets   ask-only: words and attachments only"
        )));
        if let Some(path) = pick {
            self.select_project(&path);
        }
        if let Some(row) = self
            .project_selected
            .as_deref()
            .and_then(|path| list.iter().find(|row| row.path == path))
        {
            ui.add_space(14.0);
            self.project_detail(ui, &row.clone());
        }
    }

    /// Choose a project and load its rules into the editor.
    pub(crate) fn select_project(&mut self, path: &str) {
        self.project_selected = Some(path.to_owned());
        self.revert_rules();
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
            }
            Err(error) => self.notice = Some((false, error.to_string())),
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
        ui.label(RichText::new(&row.name).size(15.0).strong());
        ui.label(theme::mono(&row.path));
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
        self.rules_editor(ui, &row.path);
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

    fn rules_editor(&mut self, ui: &mut egui::Ui, path: &str) {
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
        ui.label(theme::muted(format!(
            "{}",
            autopilot_engine::rules_path(&self.options.data_dir, path).display()
        )));
        ui.label(theme::muted(
            "Kept in the app data folder, not in the project. It guides the agent and cannot loosen what asks, what is protected or any budget.",
        ));
    }
}
