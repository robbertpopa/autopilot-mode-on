//! First run: four full-screen steps (agents, tools, environment, look) shown
//! once, with the command that fixes each missing piece.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, StrokeKind,
    TextureHandle, pos2, vec2,
};

use crate::accounts::{Level, Row, rows};
use crate::app::App;
use crate::backend;
use crate::theme;

/// Environment variable the API steps read the dedicated key from. The app
/// never stores or shows the key.
fn api_key_present() -> bool {
    std::env::var_os(autopilot_agent_claude::API_KEY_SOURCE).is_some_and(|value| !value.is_empty())
}

const STEPS: [&str; 4] = ["Agents", "Tools", "Environment", "Look"];
/// The width of the column every step is laid out in.
const COLUMN: f32 = 680.0;

/// The service logos, in the order of `LOGO_BYTES`. Simple Icons artwork,
/// 64x64 RGBA, drawn small so they stay sharp on a high-density screen.
#[derive(Clone, Copy)]
enum Logo {
    Claude,
    Github,
    Git,
    Azure,
    Codex,
}

const LOGO_BYTES: [&[u8]; 5] = [
    include_bytes!("../assets/claude.rgba"),
    include_bytes!("../assets/github.rgba"),
    include_bytes!("../assets/git.rgba"),
    include_bytes!("../assets/azure.rgba"),
    include_bytes!("../assets/codex.rgba"),
];

pub(crate) fn load_logos(ctx: &egui::Context) -> Vec<TextureHandle> {
    LOGO_BYTES
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            let image = egui::ColorImage::from_rgba_unmultiplied([64, 64], bytes);
            ctx.load_texture(
                format!("welcome-logo-{index}"),
                image,
                egui::TextureOptions::LINEAR,
            )
        })
        .collect()
}

/// The place of a service in `LOGO_BYTES`, so the nav can show the same logos
/// as the welcome screens.
pub(crate) fn service_logo(name: &str) -> Option<usize> {
    match name {
        "Claude" => Some(Logo::Claude as usize),
        "GitHub" => Some(Logo::Github as usize),
        "Azure" => Some(Logo::Azure as usize),
        "Codex" => Some(Logo::Codex as usize),
        _ => None,
    }
}

/// A white tile that holds a logo, white in both themes because the dark
/// logos vanish on a dark card.
fn logo_tile(ui: &mut egui::Ui, texture: Option<&TextureHandle>) {
    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(11),
        Color32::WHITE,
        Stroke::new(1.0_f32, theme::BORDER()),
        StrokeKind::Inside,
    );
    if let Some(texture) = texture {
        let image = egui::Rect::from_center_size(rect.center(), vec2(22.0, 22.0));
        let whole = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter()
            .image(texture.id(), image, whole, Color32::WHITE);
    }
}

fn panel(selected: bool) -> Frame {
    Frame::new()
        .fill(theme::CARD())
        .stroke(Stroke::new(
            if selected { 2.0_f32 } else { 1.0_f32 },
            if selected {
                theme::BLUE()
            } else {
                theme::BORDER()
            },
        ))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(18))
}

fn chip(ui: &mut egui::Ui, text: &str, good: bool) {
    let (fill, color) = if good {
        (theme::GREEN_FILL(), theme::GREEN())
    } else {
        (theme::AMBER_FILL(), theme::AMBER())
    };
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.5).strong().color(color));
        });
}

/// Name and one muted line under it, beside a logo.
fn title_block(ui: &mut egui::Ui, name: &str, line: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(RichText::new(name).size(15.0).strong());
        ui.label(theme::muted(line));
    });
}

/// The command that fixes a missing piece, with a Copy button.
fn fix_line(ui: &mut egui::Ui, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(theme::muted("To fix, run:"));
        ui.label(theme::mono(hint));
        if ui.small_button("Copy").clicked() {
            ui.ctx().copy_text(hint.to_owned());
        }
    });
}

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

    fn account_row(&self, name: &str) -> Option<Row> {
        self.accounts
            .as_ref()
            .map(rows)
            .and_then(|list| list.into_iter().find(|row| row.name == name))
    }

    /// Whether the agent step may be left: Claude signed in with a
    /// subscription, or a dedicated API key set, or the offline demo chosen.
    fn claude_ready(&self) -> bool {
        self.account_row("Claude")
            .is_some_and(|row| row.level == Level::Good)
            || api_key_present()
            || self.key_saved
            || self.agent == backend::AgentChoice::Practice
    }

    /// Whether the step in view may be left with Next.
    fn step_ready(&self) -> bool {
        match self.welcome_step {
            0 => self.claude_ready(),
            // Tools are shown so nothing is a surprise; none blocks the setup.
            1 | 3 => true,
            _ => self.saved_workspace.is_some() || !crate::composer::has_picker(),
        }
    }

    fn logo(&self, logo: Logo) -> Option<&TextureHandle> {
        self.welcome_logos.get(logo as usize)
    }

    /// The whole window is this one screen until it is done: no navigation,
    /// no tickets, nothing else to click.
    pub(crate) fn welcome(&mut self, ctx: &egui::Context) {
        if self.welcome_logos.is_empty() {
            self.welcome_logos = load_logos(ctx);
        }
        egui::TopBottomPanel::bottom("welcome-footer")
            .frame(
                Frame::new()
                    .fill(theme::CARD())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .inner_margin(Margin::symmetric(24, 20)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width().min(COLUMN), 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(ui.available_width());
                            self.welcome_footer(ui);
                        },
                    );
                });
            });
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND())
                    .inner_margin(Margin::symmetric(24, 0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width().min(COLUMN), 0.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(ui.available_width());
                                self.welcome_body(ui);
                            },
                        );
                    });
                });
            });
    }

    fn welcome_footer(&mut self, ui: &mut egui::Ui) {
        let last = self.welcome_step + 1 == STEPS.len();
        let ready = self.step_ready();
        ui.horizontal(|ui| {
            let tall = |text: &str| egui::Button::new(text).min_size(vec2(0.0, 34.0));
            if self.welcome_step > 0 && ui.add(tall("Back")).clicked() {
                self.welcome_step -= 1;
            }
            if self.welcome_step < 2 && ui.add(tall("Check again")).clicked() {
                self.accounts = None;
                backend::fetch_accounts(self.context.clone(), self.sender.clone());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if last { "Finish" } else { "Next" };
                let next = ui
                    .add_enabled(ready, theme::primary(label))
                    .on_disabled_hover_text("Finish this step first")
                    .clicked();
                // The environment can be set up later, on Projects.
                let skip = self.welcome_step == 2
                    && ui
                        .add(
                            egui::Button::new(RichText::new("Skip for now").strong())
                                .frame(false)
                                .min_size(vec2(0.0, 34.0)),
                        )
                        .clicked();
                if last && next {
                    self.finish_welcome();
                    self.start_new();
                } else if next || skip {
                    self.welcome_step += 1;
                }
            });
        });
    }

    fn welcome_body(&mut self, ui: &mut egui::Ui) {
        ui.add_space(48.0);
        self.welcome_header(ui);
        ui.add_space(26.0);
        self.welcome_progress(ui);
        ui.add_space(26.0);
        if let Some((ok, text)) = self.notice.clone() {
            crate::app::banner(ui, &text, ok);
            ui.add_space(10.0);
        }
        let (title, line) = match self.welcome_step {
            0 => (
                "Pick your agent",
                "The agent does the work on each ticket. Connect at least one.",
            ),
            1 => (
                "Connect your tools",
                "Commits use your Git identity. Pull requests open through GitHub or Azure DevOps, depending on where the repository lives.",
            ),
            2 => (
                "Set up your environment",
                "The folder tickets work in. Optional: without one a ticket can only ask questions, and you can add it later on Projects.",
            ),
            _ => (
                "Pick a look",
                "System follows your computer. You can switch any time in Settings.",
            ),
        };
        ui.label(RichText::new(title).size(26.0).strong());
        ui.add_space(4.0);
        ui.label(theme::muted(line));
        ui.add_space(18.0);
        match self.welcome_step {
            0 => self.welcome_agents(ui),
            1 => self.welcome_tools(ui),
            2 => self.welcome_folder(ui),
            _ => self.welcome_look(ui),
        }
        ui.add_space(24.0);
    }

    fn welcome_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(icon) = &self.icon {
                ui.add(egui::Image::new(icon).fit_to_exact_size(vec2(36.0, 36.0)));
            }
            ui.label(
                RichText::new("autopilot-mode-on")
                    .monospace()
                    .size(16.0)
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::muted(format!(
                    "Setup · {} of {}",
                    self.welcome_step + 1,
                    STEPS.len()
                )));
            });
        });
    }

    /// Numbered circles joined by lines: green with a tick when done, blue
    /// when current.
    fn welcome_progress(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let segment = rect.width() / 4.0;
        for (index, name) in STEPS.iter().enumerate() {
            let left = rect.left() + segment * [0.0, 1.0, 2.0, 3.0][index];
            let center = pos2(left + 13.0, rect.center().y);
            let number = (index + 1).to_string();
            let (fill, glyph) = match index.cmp(&self.welcome_step) {
                std::cmp::Ordering::Less => (theme::GREEN_SOLID(), "✔".to_owned()),
                std::cmp::Ordering::Equal => (theme::BLUE(), number),
                std::cmp::Ordering::Greater => (theme::SELECTED(), number),
            };
            painter.circle_filled(center, 13.0, fill);
            let on_fill = if index > self.welcome_step {
                theme::TEXT_MUTED()
            } else {
                theme::ON_ACCENT()
            };
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::proportional(12.0),
                on_fill,
            );
            let text_color = if index == self.welcome_step {
                theme::TEXT()
            } else {
                theme::TEXT_MUTED()
            };
            let galley = painter.layout_no_wrap(
                (*name).to_owned(),
                egui::FontId::proportional(13.0),
                text_color,
            );
            let label_left = left + 13.0 * 2.0 + 8.0;
            let label_right = label_left + galley.size().x;
            painter.galley(
                pos2(label_left, rect.center().y - galley.size().y / 2.0),
                galley,
                text_color,
            );
            if index + 1 < STEPS.len() {
                let line = if index < self.welcome_step {
                    theme::GREEN_SOLID()
                } else {
                    theme::BORDER_STRONG()
                };
                painter.line_segment(
                    [
                        pos2(label_right + 10.0, rect.center().y),
                        pos2(left + segment - 10.0, rect.center().y),
                    ],
                    Stroke::new(2.0_f32, line),
                );
            }
        }
    }

    /// Step 1: the agents. Claude is the one that runs steps today.
    fn welcome_agents(&mut self, ui: &mut egui::Ui) {
        let claude = self.account_row("Claude");
        let signed_in = claude.as_ref().is_some_and(|row| row.level == Level::Good);
        let checking = self.accounts.is_none();
        let key_running = api_key_present();
        let key = key_running || self.key_saved;
        let ready = self.claude_ready();
        panel(ready).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                logo_tile(ui, self.logo(Logo::Claude));
                title_block(ui, "Claude", "Runs every step today");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ready {
                        chip(ui, "Ready", true);
                    } else if checking {
                        ui.label(theme::muted("checking..."));
                    } else {
                        chip(ui, "Not ready", false);
                    }
                });
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                theme::cell(ui, 110.0, "Subscription");
                if signed_in {
                    chip(ui, "Signed in", true);
                    if let Some(line) = claude.as_ref().and_then(|row| row.lines.get(1)) {
                        ui.label(theme::muted(line.clone()));
                    }
                } else if checking {
                    ui.label(theme::muted("checking..."));
                } else {
                    chip(ui, "Not signed in", false);
                }
            });
            if !signed_in && !checking {
                fix_line(ui, "claude auth login");
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                theme::cell(ui, 110.0, "API key");
                if key {
                    chip(ui, if key_running { "Found" } else { "Saved" }, true);
                    if !key_running {
                        // A running program never sees a variable set after it started.
                        ui.label(theme::muted("Restart the app so runs can use it."));
                    }
                } else {
                    chip(ui, "Not set", false);
                }
            });
            if !key {
                fix_line(ui, "setx AUTOPILOT_ANTHROPIC_API_KEY <your key>");
            }
            ui.add_space(4.0);
            ui.label(theme::muted(
                "The app never stores or shows the key. It reads it from that environment variable when an API step runs.",
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
        });
        ui.add_space(12.0);
        let codex = self.account_row("Codex");
        panel(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                logo_tile(ui, self.logo(Logo::Codex));
                title_block(ui, "Codex", "Optional · not used to run steps yet");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &codex {
                    None => {
                        ui.label(theme::muted("checking..."));
                    }
                    Some(row) if row.level == Level::Good => chip(ui, "Signed in", true),
                    Some(row) if row.level == Level::Missing => chip(ui, "Not installed", false),
                    Some(_) => chip(ui, "Not signed in", false),
                });
            });
            if let Some(row) = codex.as_ref().filter(|row| row.level != Level::Good)
                && let Some(hint) = row.hint
            {
                ui.add_space(8.0);
                fix_line(ui, hint);
            }
        });
    }

    /// Step 2: GitHub, the Git identity and Azure.
    fn welcome_tools(&mut self, ui: &mut egui::Ui) {
        let Some(accounts) = self.accounts.clone() else {
            ui.label(theme::muted("Checking GitHub and Azure..."));
            return;
        };
        let mut switch = None;
        panel(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (index, name) in ["GitHub", "Git", "Azure"].into_iter().enumerate() {
                if index > 0 {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                }
                if name == "Git" {
                    self.git_identity_row(ui);
                    continue;
                }
                let (logo, line) = if name == "GitHub" {
                    (Logo::Github, "Opens pull requests · needed to ship")
                } else {
                    (Logo::Azure, "Tickets and pipelines · optional")
                };
                let row = rows(&accounts).into_iter().find(|row| row.name == name);
                ui.horizontal(|ui| {
                    logo_tile(ui, self.logo(logo));
                    title_block(
                        ui,
                        if name == "Azure" {
                            "Azure DevOps"
                        } else {
                            name
                        },
                        line,
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &row {
                        Some(row) if row.level == Level::Good => chip(ui, "Connected", true),
                        Some(row) if row.level == Level::Missing => {
                            chip(ui, "Not installed", false);
                        }
                        _ => chip(ui, "Not signed in", false),
                    });
                });
                if let Some(row) = row.as_ref().filter(|row| row.level != Level::Good)
                    && let Some(hint) = row.hint
                {
                    ui.add_space(6.0);
                    fix_line(ui, hint);
                }
                if let Some(picked) = crate::accounts::switch_choices(ui, name, &accounts) {
                    switch = Some(picked);
                }
            }
        });
        if let Some(switch) = switch {
            self.switch_account(switch);
        }
    }

    /// The Git identity row: who commits are signed as, with a small editor.
    fn git_identity_row(&mut self, ui: &mut egui::Ui) {
        let identity = self.git_identity.clone();
        ui.horizontal(|ui| {
            logo_tile(ui, self.logo(Logo::Git));
            title_block(
                ui,
                "Git identity",
                "Who commits are signed as · needed to commit",
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match &identity {
                    None => {
                        ui.label(theme::muted("checking..."));
                    }
                    Some(who) if who.name.is_empty() || who.email.is_empty() => {
                        chip(ui, "Not set", false);
                    }
                    Some(_) => chip(ui, "Configured", true),
                }
                if self.git_edit.is_none() && ui.button("Change").clicked() {
                    let who = identity.clone().unwrap_or_default();
                    self.git_edit = Some((who.name, who.email));
                }
            });
        });
        if let Some(who) = identity.as_ref().filter(|who| !who.name.is_empty())
            && self.git_edit.is_none()
        {
            ui.add_space(4.0);
            ui.label(theme::muted(format!("{} <{}>", who.name, who.email)));
        }
        let mut finished = None;
        if let Some((name, email)) = self.git_edit.as_mut() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                theme::cell(ui, 60.0, "Name");
                ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            });
            ui.horizontal(|ui| {
                theme::cell(ui, 60.0, "Email");
                ui.add(egui::TextEdit::singleline(email).desired_width(f32::INFINITY));
            });
            ui.horizontal(|ui| {
                if ui.add(theme::primary("Save")).clicked() {
                    finished = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    finished = Some(false);
                }
            });
        }
        match finished {
            Some(true) => {
                if let Some((name, email)) = self.git_edit.take() {
                    backend::save_git_identity(
                        self.context.clone(),
                        self.sender.clone(),
                        name,
                        email,
                    );
                }
            }
            Some(false) => self.git_edit = None,
            None => {}
        }
    }

    /// Step 3: the environment, which is the project folder and is optional.
    fn welcome_folder(&mut self, ui: &mut egui::Ui) {
        let folder = self.saved_workspace.clone();
        panel(folder.is_some()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (tile, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                ui.painter()
                    .rect_filled(tile, CornerRadius::same(12), theme::BLUE_FILL());
                let body =
                    egui::Rect::from_center_size(tile.center() + vec2(0.0, 2.0), vec2(22.0, 15.0));
                ui.painter()
                    .rect_filled(body, CornerRadius::same(3), theme::BLUE());
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(body.left_top() - vec2(0.0, 4.0), vec2(9.0, 5.0)),
                    CornerRadius::same(2),
                    theme::BLUE(),
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if let Some(path) = &folder {
                        let name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).size(15.0).strong());
                            chip(ui, "Chosen", true);
                        });
                        // A long path must not widen the card past the column.
                        let shown = path.display().to_string();
                        ui.add(egui::Label::new(theme::mono(shown.clone())).truncate())
                            .on_hover_text(shown);
                    } else {
                        ui.label(RichText::new("No folder chosen").size(15.0).strong());
                        ui.label(theme::muted("Tickets will only be able to ask questions."));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !crate::composer::has_picker() {
                        return;
                    }
                    let label = if folder.is_some() {
                        "Change folder"
                    } else {
                        "Choose a folder"
                    };
                    if ui.button(label).clicked()
                        && let Some(chosen) = crate::composer::pick_project()
                    {
                        backend::save_workspace(&self.options.data_dir, Some(&chosen));
                        self.saved_workspace = Some(chosen.clone());
                        self.add_project(&chosen);
                    }
                });
            });
        });
        ui.add_space(12.0);
        if crate::composer::has_picker() {
            ui.label(theme::muted(
                "Each ticket runs in its own copy of this folder, so your files stay untouched until you approve a pull request.",
            ));
        } else {
            ui.label(theme::muted(
                "No folder picker on this system: choose the folder on New ticket.",
            ));
        }
    }

    /// Step 4: the look, three small previews.
    fn welcome_look(&mut self, ui: &mut egui::Ui) {
        let before = self.theme;
        let mut chosen = self.theme;
        let width = (ui.available_width() - 32.0) / 3.0;
        // Top-aligned so tiles of different content height start on one line.
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            for (value, label) in [
                (theme::ThemeChoice::System, "System"),
                (theme::ThemeChoice::Light, "Light"),
                (theme::ThemeChoice::Dark, "Dark"),
            ] {
                let selected = before == value;
                let response = ui
                    .allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                        panel(selected)
                            .inner_margin(Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                preview(ui, value);
                                ui.add_space(8.0);
                                ui.label(RichText::new(label).strong());
                            });
                    })
                    .response;
                if ui
                    .interact(response.rect, ui.id().with(label), Sense::click())
                    .clicked()
                {
                    chosen = value;
                }
            }
        });
        if chosen != before {
            self.choose_theme(chosen);
        }
    }
}

/// A tiny window: light, dark, or half of each for System.
fn preview(ui: &mut egui::Ui, choice: theme::ThemeChoice) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 84.0), Sense::hover());
    let light = (
        Color32::from_rgb(0xF4, 0xF5, 0xF7),
        Color32::WHITE,
        Color32::from_rgb(0xC3, 0xC8, 0xD1),
    );
    let dark = (
        Color32::from_rgb(0x14, 0x18, 0x1F),
        Color32::from_rgb(0x1E, 0x24, 0x2E),
        Color32::from_rgb(0x3A, 0x42, 0x50),
    );
    let painter = ui.painter_at(rect);
    let half = rect.width() / 2.0;
    let sides = match choice {
        theme::ThemeChoice::Light => [light, light],
        theme::ThemeChoice::Dark => [dark, dark],
        theme::ThemeChoice::System => [light, dark],
    };
    for (index, (background, card, line)) in sides.into_iter().enumerate() {
        let part = egui::Rect::from_min_size(
            pos2(rect.left() + half * [0.0, 1.0][index], rect.top()),
            vec2(half, rect.height()),
        );
        painter.rect_filled(part, CornerRadius::ZERO, background);
        let inner = part.shrink(10.0);
        painter.rect_filled(
            egui::Rect::from_min_size(inner.left_top(), vec2(inner.width() * 0.6, 7.0)),
            CornerRadius::same(3),
            line,
        );
        painter.rect_filled(
            egui::Rect::from_min_max(inner.left_top() + vec2(0.0, 13.0), inner.right_bottom()),
            CornerRadius::same(5),
            card,
        );
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, theme::BORDER()),
        StrokeKind::Inside,
    );
}
