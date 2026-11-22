//! The box where work starts: one message box like a chat, with a single
//! attach button and a send button. There is no title field; the title is
//! the first line of the message.
//!
//! Files, images and folders arrive by dropping them on the window or, on
//! Windows and macOS, by the native pickers. They are only paths here; the
//! engine checks, copies and records them when the ticket starts.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use crate::{backend::Action, theme};

const TITLE_CHARS: usize = 60;

#[derive(Default)]
pub struct NewTicket {
    pub(crate) focused: bool,
    pub description: String,
    pub files: Vec<PathBuf>,
    /// The project folder the agent reads for this ticket.
    pub workspace: Option<PathBuf>,
    pub pipeline: Option<String>,
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
        !self.description.trim().is_empty() || !self.files.is_empty()
    }

    /// The title is the first line of the message, cut at a word near 60
    /// characters. With no words, the first attachment's name stands in.
    #[must_use]
    pub fn title(&self) -> String {
        let first = self
            .description
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("");
        if first.is_empty() {
            return self
                .files
                .first()
                .map_or_else(|| "New ticket".to_owned(), |path| chip_name(path));
        }
        if first.chars().count() <= TITLE_CHARS {
            return first.to_owned();
        }
        let cut: String = first.chars().take(TITLE_CHARS).collect();
        let kept = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
        format!("{}...", kept.trim_end_matches([',', ';', ':', '.']))
    }

    pub(crate) fn start_action(&self) -> Action {
        Action::Start {
            title: self.title(),
            description: self.description.clone(),
            attachments: self.files.clone(),
            workspace: self.workspace.clone(),
            // Without a project the ticket is ask-only whatever was picked.
            pipeline: self.workspace.as_ref().and(self.pipeline.clone()),
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
pub fn pick_project() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose the project folder")
        .pick_folder()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub const fn pick_project() -> Option<PathBuf> {
    None
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
                if theme::named(
                    ui.add(egui::Button::new(theme::muted("×")).frame(false))
                        .on_hover_text("Remove"),
                    format!("Remove {}", chip_name(path)),
                )
                .clicked()
                {
                    remove = true;
                }
            });
        });
    remove
}

/// Draw the composer. Returns the action to run when the owner sends.
///
/// Enter sends and Shift+Enter starts a new line, as in a chat.
pub fn show(ui: &mut egui::Ui, form: &mut NewTicket) -> Option<Action> {
    let mut start = None;
    let hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
    let border = if hovering {
        theme::BLUE()
    } else {
        theme::BORDER_STRONG()
    };
    let text_id = egui::Id::new("composer-text");
    // Take Enter before the text box turns it into a new line.
    let send_key = ui.memory(|memory| memory.has_focus(text_id))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    egui::Frame::new()
        .fill(theme::CARD())
        .stroke(egui::Stroke::new(1.0_f32, border))
        .corner_radius(egui::CornerRadius::same(22))
        .inner_margin(egui::Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let text = ui.add(
                egui::TextEdit::multiline(&mut form.description)
                    .id(text_id)
                    .hint_text("Describe what should change, or drop files here")
                    .frame(false)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if !form.focused {
                text.request_focus();
                form.focused = true;
            }
            let mut removed = None;
            if !form.files.is_empty() {
                ui.add_space(4.0);
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
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                attach_menu(ui, form);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ready = form.ready();
                    let send = egui::Button::new(RichText::new("↑").size(14.0).strong())
                        .fill(if ready { theme::BLUE() } else { theme::NODE() })
                        .corner_radius(egui::CornerRadius::same(99))
                        .min_size(egui::vec2(36.0, 36.0));
                    let clicked = theme::named(
                        ui.add_enabled(ready, send)
                            .on_hover_text("Send (Enter). Shift+Enter adds a line."),
                        "Send",
                    )
                    .clicked();
                    if (clicked || send_key) && ready {
                        start = Some(form.start_action());
                    }
                });
            });
        });
    project_row(ui, form);
    start
}

/// Which project the agent will read, under the box. Changing it picks another
/// folder; clearing it sends the ticket with no project.
fn project_row(ui: &mut egui::Ui, form: &mut NewTicket) {
    if !has_picker() {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (text, hover) = match &form.workspace {
            Some(path) => (
                format!("Project: {}", chip_name(path)),
                format!(
                    "{}\nThe agent reads this folder to ground its answers. It cannot change or run anything in it.",
                    path.display()
                ),
            ),
            None => (
                "Choose a project folder".to_owned(),
                "Without a project the agent knows only your words and attachments.".to_owned(),
            ),
        };
        let button = egui::Button::new(RichText::new(text).color(theme::TEXT_SOFT())).frame(false);
        if ui.add(button).on_hover_text(hover).clicked()
            && let Some(folder) = pick_project()
        {
            form.workspace = Some(folder);
        }
        if form.workspace.is_some()
            && theme::named(
                ui.add(egui::Button::new(theme::muted("×")).frame(false))
                    .on_hover_text("Do not use a project"),
                "Remove the project folder",
            )
            .clicked()
        {
            form.workspace = None;
        }
    });
}

/// One button for everything that can be attached.
fn attach_menu(ui: &mut egui::Ui, form: &mut NewTicket) {
    if !has_picker() {
        ui.label(theme::muted(
            "Drop files or a folder on the window to attach them",
        ));
        return;
    }
    let button = egui::Button::new(RichText::new("+").size(15.0))
        .fill(theme::NODE())
        .corner_radius(egui::CornerRadius::same(99))
        .min_size(egui::vec2(36.0, 36.0));
    let response = ui
        .add(button)
        .on_hover_text("Attach files, images or a folder");
    let response = theme::named(response, "Attach");
    egui::Popup::menu(&response).show(|ui| {
        if ui
            .button("Files or images")
            .on_hover_text("Their contents are kept as data and never run.")
            .clicked()
        {
            form.add(pick_files());
            ui.close();
        }
        if ui
            .button("Folder")
            .on_hover_text("The folder is recorded by its path, not copied.")
            .clicked()
        {
            form.add(pick_folder());
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: the title comes from the first non-empty line, is cut at a
    // word and never empty, so a ticket started with no title still has a
    // readable name in the sidebar.
    #[test]
    fn the_title_is_the_first_line_of_the_message() {
        let mut form = NewTicket::default();
        assert_eq!(form.title(), "New ticket");
        form.add([PathBuf::from("/x/spec.md")]);
        assert_eq!(form.title(), "spec.md");
        form.description = "\n  Fix the timezone bug\nand more".into();
        assert_eq!(form.title(), "Fix the timezone bug");
        form.description = "word ".repeat(30);
        let title = form.title();
        assert!(title.ends_with("..."), "{title}");
        assert!(title.chars().count() <= TITLE_CHARS + 3);
    }

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
