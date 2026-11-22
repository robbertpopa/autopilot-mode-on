//! The left navigation: new ticket, tickets grouped by project or status,
//! the spend meter, and at the foot one icon per account and the settings gear.
//! It only paints and switches pages; it starts nothing itself.

use autopilot_accounts::Accounts;
use autopilot_engine::{Badge, TicketView, dollars};
use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Stroke};

use crate::accounts::{Level, Row, rows};
use crate::app::{App, Mode, ticket_row};
use crate::theme;

/// Below this window width the nav shrinks to icons.
const COLLAPSE_BELOW: f32 = 900.0;
const WIDE: f32 = 224.0;
/// Height the spend card and the account row take at the foot of the nav.
const FOOT_HEIGHT: f32 = 140.0;
/// The spend card: two lines, the bar and padding.
const SPEND_HEIGHT: f32 = 76.0;
const NARROW: f32 = 64.0;

/// Group heading for tickets that have no project folder.
const NO_PROJECT: &str = "no project";

/// The project a ticket belongs to: the name of its folder.
#[must_use]
pub fn project_name(view: &TicketView) -> Option<String> {
    let path = view.workspace.as_deref()?;
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    (!name.is_empty()).then(|| name.to_owned())
}

/// Tickets in named groups, in first-seen order, with the ungrouped last.
/// Needs-you tickets come first inside each group.
fn groups(tickets: &[TicketView], by_status: bool) -> Vec<(String, Vec<&TicketView>)> {
    let mut groups: Vec<(String, Vec<&TicketView>)> = Vec::new();
    for view in tickets {
        let name = if by_status {
            status_group(view.badge).to_owned()
        } else {
            project_name(view).unwrap_or_else(|| NO_PROJECT.to_owned())
        };
        match groups.iter_mut().find(|(known, _)| *known == name) {
            Some((_, members)) => members.push(view),
            None => groups.push((name, vec![view])),
        }
    }
    for (_, members) in &mut groups {
        members.sort_by_key(|view| view.badge != Badge::NeedsYou);
    }
    if by_status {
        groups.sort_by_key(|(name, _)| status_rank(name));
    } else {
        groups.sort_by_key(|(name, _)| name == NO_PROJECT);
    }
    groups
}

const fn status_group(badge: Badge) -> &'static str {
    match badge {
        Badge::NeedsYou => "Needs you",
        Badge::Running => "Running",
        Badge::Failed | Badge::Interrupted => "Stopped",
        Badge::Done => "Done",
        Badge::Paused | Badge::Idle => "Idle",
    }
}

fn status_rank(name: &str) -> usize {
    ["Needs you", "Running", "Stopped", "Idle", "Done"]
        .iter()
        .position(|known| *known == name)
        .unwrap_or(usize::MAX)
}

/// The brand colour of a service, grey when it is not usable.
fn brand(name: &str, level: Level) -> Color32 {
    if level == Level::Missing {
        return theme::TEXT_MUTED();
    }
    match name {
        "Claude" => Color32::from_rgb(0xD9, 0x77, 0x57),
        "Codex" => Color32::from_rgb(0x10, 0xA3, 0x7F),
        "Azure" => Color32::from_rgb(0x2F, 0x8B, 0xE0),
        _ => theme::TEXT(),
    }
}

fn dot_color(level: Level) -> Color32 {
    match level {
        Level::Good => theme::GREEN(),
        Level::Attention => theme::AMBER(),
        Level::Missing => theme::TEXT_MUTED(),
    }
}

fn tooltip(row: &Row) -> String {
    let mut text = format!("{}: {}", row.name, row.lines.join(", "));
    if let Some(hint) = row.hint {
        text = format!("{text}\nfix: {hint}");
    }
    text
}

/// One service as a coloured disc with its initial and a status dot.
/// Returns true when clicked.
fn account_icon(ui: &mut egui::Ui, row: &Row, pending: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
    let color = if pending {
        theme::TEXT_MUTED()
    } else {
        brand(row.name, row.level)
    };
    let painter = ui.painter();
    painter.circle_stroke(rect.center(), 12.0, Stroke::new(1.5_f32, color));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        row.name.chars().next().unwrap_or('?'),
        egui::FontId::proportional(13.0),
        color,
    );
    let status = if pending {
        theme::TEXT_MUTED()
    } else {
        dot_color(row.level)
    };
    painter.circle_filled(rect.right_bottom() - egui::vec2(5.0, 5.0), 4.0, status);
    let text = if pending {
        format!("{}: checking...", row.name)
    } else {
        tooltip(row)
    };
    theme::named(response.on_hover_text(text), row.name).clicked()
}

/// A gear drawn with lines, because the default font has no gear glyph.
fn gear_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
    let painter = ui.painter();
    let color = theme::TEXT_SOFT();
    let center = rect.center();
    painter.circle_stroke(center, 6.0, Stroke::new(1.6_f32, color));
    for step in 0_u8..8 {
        let angle = f32::from(step) * std::f32::consts::FRAC_PI_4;
        let direction = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment(
            [center + direction * 8.0, center + direction * 11.0],
            Stroke::new(2.4_f32, color),
        );
    }
    theme::named(response.on_hover_text("Settings"), "Settings")
}

/// One row of the nav: left-aligned label, an optional amber count chip or a
/// key hint at the right, a raised fill when it is the open page.
fn nav_row(
    ui: &mut egui::Ui,
    name: &str,
    short: &str,
    selected: bool,
    narrow: bool,
    right: Option<(String, bool)>,
) -> bool {
    let size = egui::vec2(ui.available_width(), 30.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(7),
            if selected {
                theme::NODE()
            } else {
                theme::SELECTED()
            },
        );
    }
    let text_color = if selected {
        theme::TEXT()
    } else {
        theme::TEXT_SOFT()
    };
    let font = egui::FontId::proportional(13.0);
    if narrow {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            short,
            font,
            text_color,
        );
    } else {
        ui.painter().text(
            rect.left_center() + egui::vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            font,
            text_color,
        );
        if let Some((text, chip)) = right {
            let end = rect.right_center() - egui::vec2(8.0, 0.0);
            if chip {
                let ink = Color32::from_rgb(0x1A, 0x13, 0x05);
                let galley = ui
                    .painter()
                    .layout_no_wrap(text, egui::FontId::monospace(11.0), ink);
                let width = galley.size().x + 14.0;
                let pill = egui::Rect::from_min_size(
                    end - egui::vec2(width, 9.0),
                    egui::vec2(width, 18.0),
                );
                ui.painter()
                    .rect_filled(pill, egui::CornerRadius::same(5), theme::AMBER_SOLID());
                ui.painter()
                    .galley(pill.center() - galley.size() / 2.0, galley, ink);
            } else {
                ui.painter().text(
                    end,
                    egui::Align2::RIGHT_CENTER,
                    text,
                    egui::FontId::monospace(11.0),
                    theme::TEXT_MUTED(),
                );
            }
        }
    }
    theme::named(response, name).clicked()
}

impl App {
    pub(crate) fn nav(&mut self, ctx: &egui::Context) {
        let narrow = ctx.content_rect().width() < COLLAPSE_BELOW;
        egui::SidePanel::left("nav")
            .exact_width(if narrow { NARROW } else { WIDE })
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .inner_margin(Margin::symmetric(12, 16)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                self.nav_head(ui, narrow);
                ui.add_space(15.0);
                self.new_ticket_button(ui, narrow);
                ui.add_space(15.0);
                self.nav_links(ui, narrow);
                if !narrow {
                    ui.add_space(15.0);
                    // Leave room for the spend card and the account row, which
                    // are laid out from the bottom: without a cap the list
                    // would run under them.
                    let room = (ui.available_height() - FOOT_HEIGHT).max(60.0);
                    self.nav_tickets(ui, room);
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    self.nav_foot(ui, narrow);
                    if !narrow {
                        self.spend_meter(ui);
                    }
                });
            });
    }

    fn nav_head(&self, ui: &mut egui::Ui, narrow: bool) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if let Some(icon) = &self.icon {
                ui.add(
                    egui::Image::new(icon)
                        .fit_to_exact_size(egui::vec2(28.0, 28.0))
                        .corner_radius(7),
                );
            }
            if !narrow {
                ui.label(
                    RichText::new("autopilot-mode-on")
                        .monospace()
                        .size(13.0)
                        .strong(),
                );
            }
        });
    }

    fn new_ticket_button(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let size = egui::vec2(ui.available_width(), 36.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(7), theme::BLUE());
        let on = theme::ON_ACCENT();
        if narrow {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(18.0),
                on,
            );
        } else {
            ui.painter().text(
                rect.left_center() + egui::vec2(12.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "New ticket",
                egui::FontId::proportional(13.0),
                on,
            );
            let kbd = egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(28.0, 0.0),
                egui::vec2(26.0, 18.0),
            );
            ui.painter().rect_stroke(
                kbd,
                egui::CornerRadius::same(4),
                Stroke::new(1.0_f32, on),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                kbd.center(),
                egui::Align2::CENTER_CENTER,
                "^N",
                egui::FontId::monospace(11.0),
                on,
            );
        }
        if theme::named(response, "New ticket").clicked() {
            self.start_new();
        }
    }

    fn nav_links(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let count = self.today_count();
        let inbox = (count > 0).then(|| (count.to_string(), true));
        if nav_row(ui, "Inbox", "I", self.mode == Mode::Today, narrow, inbox) {
            self.mode = Mode::Today;
        }
        for (mode, name, short) in [
            (Mode::Board, "Board", "B"),
            (Mode::Projects, "Projects", "P"),
            (Mode::Pipelines, "Pipelines", "L"),
            (Mode::Health, "Health", "H"),
        ] {
            if nav_row(ui, name, short, self.mode == mode, narrow, None) {
                self.mode = mode;
            }
        }
        if nav_row(ui, "Help", "?", false, narrow, None) {
            self.help_open = !self.help_open;
        }
        if nav_row(ui, "Search", "S", false, narrow, Some(("^K".into(), false))) {
            self.search_open = true;
            self.search_query.clear();
            self.search_selected = 0;
        }
    }

    fn nav_tickets(&mut self, ui: &mut egui::Ui, room: f32) {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(
                RichText::new("TICKETS")
                    .monospace()
                    .size(11.0)
                    .strong()
                    .color(theme::TEXT_MUTED()),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if self.group_by_status {
                    "by status"
                } else {
                    "by project"
                };
                if ui
                    .small_button(label)
                    .on_hover_text("Group tickets by project or by status")
                    .clicked()
                {
                    self.group_by_status = !self.group_by_status;
                }
            });
        });
        if self.snapshot.tickets.is_empty() {
            ui.label(theme::muted("No tickets yet."));
            return;
        }
        let grouped = groups(&self.snapshot.tickets, self.group_by_status);
        let mut open = None;
        let mut toggled = None;
        egui::ScrollArea::vertical()
            .max_height(room)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (name, members) in &grouped {
                    let closed = self.closed_groups.contains(name);
                    let needs = members.iter().any(|view| view.badge == Badge::NeedsYou);
                    ui.horizontal(|ui| {
                        let arrow = if closed { "▶" } else { "▼" };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{arrow} {name}  {}", members.len()))
                                        .monospace()
                                        .size(12.0)
                                        .color(theme::TEXT_SOFT()),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            toggled = Some(name.clone());
                        }
                        if needs {
                            ui.label(RichText::new("needs you").size(11.0).color(theme::AMBER()));
                        }
                    });
                    if closed {
                        continue;
                    }
                    for view in members {
                        if ticket_row(ui, view, self.selected.as_ref() == Some(&view.id)) {
                            open = Some(view.id.clone());
                        }
                    }
                }
            });
        if let Some(name) = toggled
            && !self.closed_groups.remove(&name)
        {
            self.closed_groups.insert(name);
        }
        if let Some(id) = open {
            self.open(&id);
        }
    }

    /// Spend today, API (exact) and subscription (estimate) kept apart.
    fn spend_meter(&self, ui: &mut egui::Ui) {
        let limit = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.budgets.daily_microusd);
        let spent = self.snapshot.today_api.as_microusd();
        let small = |text: String, muted: bool| {
            RichText::new(text).monospace().size(11.0).color(if muted {
                theme::TEXT_MUTED()
            } else {
                theme::TEXT()
            })
        };
        // A fixed-height box: this runs inside the nav's bottom-up layout,
        // where a frame would otherwise stretch to the whole free height.
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), SPEND_HEIGHT),
            Layout::top_down(Align::Min),
            |ui| {
                Frame::new()
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(small("spend/day".into(), true));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let cap = limit.map_or_else(
                                        || "no cap".to_owned(),
                                        |limit| {
                                            dollars(autopilot_core::Money::from_microusd(
                                                limit.get(),
                                            ))
                                        },
                                    );
                                    ui.label(small(cap, true));
                                });
                            });
                            let (bar, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 4.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                bar,
                                egui::CornerRadius::same(2),
                                theme::BORDER(),
                            );
                            if let Some(limit) = limit {
                                let permille = u16::try_from(
                                    (spent.saturating_mul(1000) / limit.get()).min(1000),
                                )
                                .unwrap_or(1000);
                                let mut filled = bar;
                                filled.set_width(bar.width() * f32::from(permille) / 1000.0);
                                ui.painter().rect_filled(
                                    filled,
                                    egui::CornerRadius::same(2),
                                    theme::GREEN_SOLID(),
                                );
                            }
                            ui.horizontal(|ui| {
                                ui.label(small(
                                    format!("api {}", dollars(self.snapshot.today_api)),
                                    false,
                                ));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(small(
                                        format!(
                                            "sub ~{}",
                                            dollars(self.snapshot.today_subscription)
                                        ),
                                        true,
                                    ));
                                });
                            });
                        });
                    });
            },
        );
    }

    fn nav_foot(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let accounts: Option<Accounts> = self.accounts.clone();
        if narrow {
            if gear_button(ui).clicked() {
                self.mode = Mode::Asking;
            }
            self.account_icons(ui, accounts.as_ref());
            return;
        }
        // A horizontal row takes only its own height, so the spend card sits
        // directly above it instead of being pushed up the panel.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            self.account_icons(ui, accounts.as_ref());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if gear_button(ui).clicked() {
                    self.mode = Mode::Asking;
                }
            });
        });
    }

    fn account_icons(&mut self, ui: &mut egui::Ui, accounts: Option<&Accounts>) {
        let shown = accounts.map(rows);
        // `rows` lists GitHub, Claude, Codex, Azure; the nav shows Claude first.
        let order = ["Claude", "GitHub", "Azure", "Codex"];
        for name in order {
            let row = shown
                .as_ref()
                .and_then(|list| list.iter().find(|row| row.name == name))
                .cloned()
                .unwrap_or(Row {
                    name,
                    level: Level::Missing,
                    lines: vec![],
                    hint: None,
                });
            if account_icon(ui, &row, accounts.is_none()) {
                self.mode = Mode::Accounts;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn views() -> Vec<TicketView> {
        let directory = tempfile::tempdir().unwrap();
        crate::demo::seed(directory.path(), 1_700_000_000_000).unwrap();
        let options = crate::backend::Options {
            data_dir: directory.path().to_path_buf(),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        crate::backend::read(&options).tickets
    }

    // Invariant: a ticket's project is the last folder of its workspace path,
    // for both separators, and an empty or missing path is "no project".
    #[test]
    fn the_project_is_the_last_folder_name() {
        let mut view = views().remove(0);
        view.workspace = Some(r"D:\work\quokka\".into());
        assert_eq!(project_name(&view).as_deref(), Some("quokka"));
        view.workspace = Some("/home/me/quokka".into());
        assert_eq!(project_name(&view).as_deref(), Some("quokka"));
        view.workspace = Some(String::new());
        assert_eq!(project_name(&view), None);
        view.workspace = None;
        assert_eq!(project_name(&view), None);
    }

    // Invariant: "no project" is listed last and needs-you tickets lead
    // their group. Sorting by name alone would put it first or bury them.
    #[test]
    fn no_project_is_last_and_needs_you_leads() {
        let mut all = views();
        assert!(all.len() >= 3, "the demo has several tickets");
        all.truncate(3);
        all[0].workspace = None;
        all[1].workspace = Some("/x/zeta".into());
        all[1].badge = Badge::Done;
        all[2].workspace = Some("/x/zeta".into());
        all[2].badge = Badge::NeedsYou;
        let grouped = groups(&all, false);
        assert_eq!(grouped[0].0, "zeta");
        assert_eq!(grouped[1].0, NO_PROJECT);
        assert_eq!(grouped[0].1[0].badge, Badge::NeedsYou);
    }
}
