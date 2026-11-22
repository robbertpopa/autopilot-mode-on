//! Board: every ticket in a column by what it is doing now. Five columns, as
//! in the design: needs you, running, queued, pr open, done.

use autopilot_engine::{Badge, TicketView};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke};

use crate::app::App;
use crate::nav::project_name;
use crate::theme;

/// The column a badge belongs to. A paused ticket is still work in hand.
#[must_use]
pub const fn column(badge: Badge) -> &'static str {
    match badge {
        Badge::NeedsYou | Badge::Failed | Badge::Interrupted => "needs you",
        Badge::Running | Badge::Paused | Badge::Idle => "running",
        Badge::Done => "done",
    }
}

/// Column name, then the colour of its dot. `queued` has no tickets until the
/// scheduler has a caller, and `pr open` none until real pull requests (S6).
const COLUMNS: [&str; 5] = ["needs you", "running", "queued", "pr open", "done"];

fn dot(name: &str) -> Color32 {
    match name {
        "needs you" => theme::AMBER_SOLID(),
        "running" => theme::BLUE(),
        "pr open" => Color32::from_rgb(0xA7, 0x8B, 0xFA),
        "done" => theme::GREEN_SOLID(),
        _ => theme::TEXT_MUTED(),
    }
}

fn card_line(view: &TicketView) -> Color32 {
    match view.badge {
        Badge::NeedsYou => theme::AMBER(),
        Badge::Failed | Badge::Interrupted => theme::RED(),
        _ => theme::BORDER(),
    }
}

fn status_line(view: &TicketView) -> (String, Color32) {
    match view.badge {
        Badge::NeedsYou => ("approve".to_owned(), theme::AMBER()),
        Badge::Failed => ("stopped".to_owned(), theme::RED()),
        Badge::Interrupted => ("interrupted".to_owned(), theme::RED()),
        Badge::Running => match view.rounds.last() {
            Some(round) => (
                format!("build ⟲ check r{}", round.round),
                theme::BLUE_TEXT(),
            ),
            None => ("running".to_owned(), theme::BLUE_TEXT()),
        },
        other => (other.label().to_lowercase(), theme::TEXT_MUTED()),
    }
}

impl App {
    pub(crate) fn board_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Board").size(18.0).strong());
        ui.add_space(6.0);
        let tickets: Vec<TicketView> = self.snapshot.tickets.clone();
        let mut open = None;
        ui.columns(5, |columns| {
            for (index, name) in COLUMNS.iter().enumerate() {
                let ui = &mut columns[index];
                ui.spacing_mut().item_spacing.y = 6.0;
                let members: Vec<&TicketView> = tickets
                    .iter()
                    .filter(|view| column(view.badge) == *name)
                    .collect();
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.5, dot(name));
                    ui.label(
                        RichText::new(name.to_uppercase())
                            .monospace()
                            .size(11.0)
                            .strong()
                            .color(theme::TEXT()),
                    );
                    ui.label(
                        RichText::new(members.len().to_string())
                            .monospace()
                            .size(11.0)
                            .color(theme::TEXT_MUTED()),
                    );
                });
                if members.is_empty() {
                    Frame::new()
                        .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new("—")
                                    .monospace()
                                    .size(11.0)
                                    .color(theme::TEXT_MUTED()),
                            );
                        });
                }
                for view in members {
                    let shown = Frame::new()
                        .fill(theme::CARD())
                        .stroke(Stroke::new(1.0_f32, card_line(view)))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 6.0;
                            ui.label(RichText::new(&view.title).monospace().size(12.0).strong());
                            let (text, color) = status_line(view);
                            ui.label(RichText::new(text).monospace().size(12.0).color(color));
                            if let Some(project) = project_name(view) {
                                ui.label(
                                    RichText::new(project)
                                        .monospace()
                                        .size(12.0)
                                        .color(theme::TEXT_MUTED()),
                                );
                            }
                        });
                    let click = ui.interact(
                        shown.response.rect,
                        ui.id().with(("board", view.id.as_str())),
                        egui::Sense::click(),
                    );
                    if theme::named(click, &view.title).clicked() {
                        open = Some(view.id.clone());
                    }
                }
            }
        });
        if let Some(id) = open {
            self.open(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: a ticket that waits on the owner or stopped is in "needs
    // you", running and paused work is in "running", and only Done is Done.
    // Moving a badge between columns changes what the owner is told to do.
    #[test]
    fn badges_land_in_the_right_columns() {
        assert_eq!(column(Badge::NeedsYou), "needs you");
        assert_eq!(column(Badge::Failed), "needs you");
        assert_eq!(column(Badge::Interrupted), "needs you");
        assert_eq!(column(Badge::Running), "running");
        assert_eq!(column(Badge::Paused), "running");
        assert_eq!(column(Badge::Done), "done");
    }
}
