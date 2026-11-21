//! The box where work starts: a title, the words, and what to attach.
//!
//! Files, images and folders arrive by dropping them on the window or, on
//! Windows and macOS, by the native pickers. They are only paths here; the
//! engine checks, copies and records them when the ticket starts.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use crate::{
    backend::{Action, AgentChoice},
    theme,
};

#[derive(Default)]
pub struct NewTicket {
    pub(crate) focused: bool,
    pub title: String,
    pub description: String,
    pub files: Vec<PathBuf>,
}

impl NewTicket {
    /// Add dropped or picked paths once each, up to the engine's limit.
    pub fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if self.files.len() >= autopilot_engine::MAX_ATTACHMENTS {
                break;
            }
            if !self.files.contains(&path) {
                self.files.push(path);
            }
        }
    }

    fn ready(&self) -> bool {
        !self.title.trim().is_empty()
    }

    fn action(&self) -> Action {
        Action::Start {
            title: self.title.clone(),
            description: self.description.clone(),
            attachments: self.files.clone(),
        }
    }
}

/// What the owner is picking from the native dialogs. Only Windows and macOS
/// have one: on Linux a dialog needs a desktop portal that is not tested.
#[must_use]
pub const fn has_picker() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

#[cfg(any(windows, target_os = "macos"))]
pub fn pick_files() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Attach files or images")
        .pick_files()
        .unwrap_or_default()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub const fn pick_files() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(any(windows, target_os = "macos"))]
pub fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Attach a folder")
        .pick_folder()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub const fn pick_folder() -> Option<PathBuf> {
    None
}

/// The label a chip shows for a path.
fn chip_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// One attached path as a small removable chip.
fn chip(ui: &mut egui::Ui, path: &Path) -> bool {
    let mut remove = false;
    let glyph = if path.is_dir() { "▤" } else { "▪" };
    egui::Frame::new()
        .fill(theme::NODE())
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(glyph).color(theme::TEXT_MUTED()));
                ui.label(RichText::new(chip_name(path)).size(13.0))
                    .on_hover_text(path.display().to_string());
                if ui
                    .add(egui::Button::new(theme::muted("×")).frame(false))
                    .on_hover_text("Remove")
                    .clicked()
                {
                    remove = true;
                }
            });
        });
    remove
}

/// Draw the composer. Returns the action to run when the owner starts.
pub fn show(
    ui: &mut egui::Ui,
    form: &mut NewTicket,
    hint: &str,
    agent: &mut AgentChoice,
) -> Option<Action> {
    let mut start = None;
    let hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
    let border = if hovering {
        theme::BLUE()
    } else {
        theme::BORDER_STRONG()
    };
    theme::card(theme::CARD(), border).show(ui, |ui| {
        ui.set_width(ui.available_width() - 38.0);
        let title = ui.add(
            egui::TextEdit::singleline(&mut form.title)
                .hint_text("A short title, e.g. Fix the timezone bug")
                .font(egui::TextStyle::Heading)
                .frame(false)
                .desired_width(f32::INFINITY),
        );
        if !form.focused {
            title.request_focus();
            form.focused = true;
        }
        ui.add(
            egui::TextEdit::multiline(&mut form.description)
                .hint_text("What should change? Drop files, images or a folder here. (optional)")
                .frame(false)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        let mut removed = None;
        if !form.files.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (index, path) in form.files.iter().enumerate() {
                    if chip(ui, path) {
                        removed = Some(index);
                    }
                }
            });
        }
        if let Some(index) = removed {
            form.files.remove(index);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(theme::muted("Agent"));
            theme::segmented(
                ui,
                agent,
                &[
                    (AgentChoice::Practice, "Practice"),
                    (AgentChoice::Claude, "Claude"),
                ],
            );
            if has_picker() {
                if ui
                    .button("Add files")
                    .on_hover_text(
                        "Files and images. Their contents are kept as data and never run.",
                    )
                    .clicked()
                {
                    form.add(pick_files());
                }
                if ui
                    .button("Add folder")
                    .on_hover_text("The folder is recorded by its path, not copied.")
                    .clicked()
                {
                    form.add(pick_folder());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ready = form.ready();
                let enter = ui
                    .input(|input| input.key_pressed(egui::Key::Enter) && input.modifiers.command);
                if ui.add_enabled(ready, theme::primary("Start")).clicked() || (enter && ready) {
                    start = Some(form.action());
                }
                ui.label(theme::muted(hint));
            });
        });
    });
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: the same path added twice (a drop and a pick of one file)
    // appears once, and the list stops at the engine's limit, so the start
    // action can never ask for more than the engine will accept.
    #[test]
    fn paths_are_kept_once_and_capped() {
        let mut form = NewTicket::default();
        form.add([PathBuf::from("/a"), PathBuf::from("/a")]);
        assert_eq!(form.files.len(), 1);
        form.add((0..100).map(|n| PathBuf::from(format!("/f{n}"))));
        assert_eq!(form.files.len(), autopilot_engine::MAX_ATTACHMENTS);
    }
}
