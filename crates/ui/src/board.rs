//! Board: every ticket in a column by what it is doing now.

use autopilot_engine::{Badge, TicketView};
use eframe::egui::{self, RichText};

use crate::app::{App, badge_colors};
use crate::nav::project_name;
use crate::theme;

/// The column a badge belongs to. A paused ticket is still work in hand.
#[must_use]
pub const fn column(badge: Badge) -> &'static str {
    match badge {
        Badge::NeedsYou | Badge::Failed | Badge::Interrupted => "Needs you",
        Badge::Running | Badge::Paused | Badge::Idle => "Working",
        Badge::Done => "Done",
    }
}

const COLUMNS: [&str; 3] = ["Needs you", "Working", "Done"];

impl App {
    pub(crate) fn board_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Board").size(30.0).strong());
        ui.add_space(10.0);
        let tickets: Vec<TicketView> = self.snapshot.tickets.clone();
        let mut open = None;
        ui.columns(5, |columns| {
            for (index, name) in COLUMNS.iter().enumerate() {
                let ui = &mut columns[index];
                ui.label(RichText::new(*name).strong());
                for view in tickets.iter().filter(|view| column(view.badge) == *name) {
                    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                        ui.set_width(ui.available_width() - 34.0);
                        if ui.link(&view.title).clicked() {
                            open = Some(view.id.clone());
                        }
                        let (text, fill, color) = badge_colors(view.badge, view);
                        theme::pill(ui, &text, fill, color);
                        if let Some(project) = project_name(view) {
                            ui.label(theme::muted(project));
                        }
                        if let Some(round) = view.rounds.last() {
                            ui.label(theme::muted(format!("round {}", round.round)));
                        }
                    });
                }
            }
            columns[3].label(RichText::new("Waiting in line").strong());
            columns[3].label(theme::muted("needs request R4: a queued state"));
            columns[4].label(RichText::new("PR open").strong());
            columns[4].label(theme::muted("after real pull requests (S6)"));
        });
        if let Some(id) = open {
            self.open(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: a ticket that waits on the owner or stopped is in "Needs
    // you", running and paused work is in "Working", and only Done is Done.
    // Moving a badge between columns changes what the owner is told to do.
    #[test]
    fn badges_land_in_the_right_columns() {
        assert_eq!(column(Badge::NeedsYou), "Needs you");
        assert_eq!(column(Badge::Failed), "Needs you");
        assert_eq!(column(Badge::Interrupted), "Needs you");
        assert_eq!(column(Badge::Running), "Working");
        assert_eq!(column(Badge::Paused), "Working");
        assert_eq!(column(Badge::Done), "Done");
    }
}
