//! Ctrl+K: one box that finds tickets, projects, pipelines, settings pages and
//! commands. The index is built in memory from what the window already holds
//! and is never written anywhere. It never holds rules.md text, a key or the
//! contents of an attachment (names only): ticket text comes from the
//! already-redacted `searchable` field and from titles and timeline lines.

use autopilot_core::TicketId;
use autopilot_engine::{Badge, TicketView};
use autopilot_profile::Profile;
use eframe::egui::{self, Color32, RichText};

use crate::app::{App, Mode};
use crate::backend::Action;
use crate::nav::project_name;
use crate::theme;

const MAX_HITS: usize = 30;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    NewTicket,
    Go(Mode),
    SwitchTheme,
    PauseOrResume,
    OpenWorktree,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    Ticket(TicketId),
    Project(String),
    Pipeline(String),
    Page(Mode),
    Command(Command),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hit {
    pub kind: &'static str,
    pub label: String,
    pub detail: String,
    pub target: Target,
    /// Why the hit cannot be used now, for a greyed command.
    pub disabled: Option<&'static str>,
}

/// Which kinds a prefix narrows the search to, and the text after it.
fn split_prefix(query: &str) -> (Option<&'static str>, &str) {
    let query = query.trim_start();
    for (prefix, kind) in [
        ("pl:", "pipeline"),
        ("t:", "ticket"),
        ("p:", "project"),
        (">", "command"),
    ] {
        if let Some(rest) = query.strip_prefix(prefix) {
            return (Some(kind), rest.trim_start());
        }
    }
    (None, query)
}

/// Whether every word of `needle` appears in `text`, ignoring case.
fn matches(text: &str, needle: &str) -> bool {
    let text = text.to_lowercase();
    needle
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

const PAGES: [(&str, Mode); 10] = [
    ("Today", Mode::Today),
    ("Board", Mode::Board),
    ("Projects", Mode::Projects),
    ("Pipelines", Mode::Pipelines),
    ("When it asks you", Mode::Asking),
    ("Pipeline steps", Mode::Design),
    ("Cost and limits", Mode::Cost),
    ("Checks", Mode::Checks),
    ("Accounts", Mode::Accounts),
    ("Look: theme, notifications", Mode::Look),
];

fn ticket_text(view: &TicketView) -> String {
    let mut text = view.title.clone();
    for card in &view.steps {
        if let Some(short) = &card.in_short {
            text.push(' ');
            text.push_str(short);
        }
    }
    for line in &view.timeline {
        text.push(' ');
        text.push_str(&line.text);
    }
    for file in &view.attachments {
        text.push(' ');
        text.push_str(&file.name);
    }
    for (_, searchable) in &view.searchable {
        text.push(' ');
        text.push_str(searchable);
    }
    text
}

/// The commands that match, each greyed with its reason when it cannot run.
fn command_hits(needle: &str, open_ticket: Option<&TicketView>) -> Vec<Hit> {
    let mut hits = Vec::new();
    let pause = open_ticket.map_or("Pause or resume the open ticket", |view| {
        if view.badge == Badge::Paused {
            "Resume the open ticket"
        } else {
            "Pause the open ticket"
        }
    });
    let mut commands = vec![
        ("New ticket", Command::NewTicket, None),
        ("Go to Board", Command::Go(Mode::Board), None),
        ("Go to Projects", Command::Go(Mode::Projects), None),
        ("Go to Pipelines", Command::Go(Mode::Pipelines), None),
        ("Go to Settings", Command::Go(Mode::Asking), None),
        ("Switch theme", Command::SwitchTheme, None),
        (
            pause,
            Command::PauseOrResume,
            open_ticket
                .is_none()
                .then_some("open a ticket that is running first"),
        ),
        (
            "Open the worktree",
            Command::OpenWorktree,
            open_ticket.is_none().then_some("open a ticket first"),
        ),
    ];
    commands.retain(|(label, _, _)| needle.is_empty() || matches(label, needle));
    for (label, command, disabled) in commands {
        hits.push(Hit {
            kind: "command",
            label: label.to_owned(),
            detail: String::new(),
            target: Target::Command(command),
            disabled,
        });
    }

    hits
}

/// Everything that matches `query`, best kinds first. An empty query lists the
/// commands and the newest tickets.
#[must_use]
pub fn search(
    query: &str,
    tickets: &[TicketView],
    profile: Option<&Profile>,
    open_ticket: Option<&TicketView>,
) -> Vec<Hit> {
    let (only, needle) = split_prefix(query);
    let want = |kind: &str| only.is_none_or(|only| only == kind);
    let mut hits = Vec::new();
    if want("command") {
        hits.extend(command_hits(needle, open_ticket));
    }
    if want("ticket") {
        for view in tickets {
            if needle.is_empty() || matches(&ticket_text(view), needle) {
                hits.push(Hit {
                    kind: "ticket",
                    label: view.title.clone(),
                    detail: format!(
                        "{} · {}",
                        project_name(view).unwrap_or_else(|| "no project".into()),
                        view.badge.label()
                    ),
                    target: Target::Ticket(view.id.clone()),
                    disabled: None,
                });
            }
        }
    }
    if want("project")
        && let Some(profile) = profile
    {
        for project in profile.projects.values() {
            if needle.is_empty() || matches(&format!("{} {}", project.name, project.path), needle) {
                hits.push(Hit {
                    kind: "project",
                    label: project.name.clone(),
                    detail: project.path.clone(),
                    target: Target::Project(project.path.clone()),
                    disabled: None,
                });
            }
        }
    }
    if want("pipeline")
        && let Some(profile) = profile
    {
        for pipeline in profile.pipelines() {
            if needle.is_empty() || matches(&pipeline.name, needle) {
                hits.push(Hit {
                    kind: "pipeline",
                    label: pipeline.name.clone(),
                    detail: if pipeline.builtin {
                        "built-in"
                    } else {
                        "custom"
                    }
                    .into(),
                    target: Target::Pipeline(pipeline.name),
                    disabled: None,
                });
            }
        }
    }
    if only.is_none() && !needle.is_empty() {
        for (label, mode) in PAGES {
            if matches(label, needle) {
                hits.push(Hit {
                    kind: "settings",
                    label: label.to_owned(),
                    detail: String::new(),
                    target: Target::Page(mode),
                    disabled: None,
                });
            }
        }
    }
    hits.truncate(MAX_HITS);
    hits
}

impl App {
    /// Ctrl+K opens and closes the overlay on any page, even from a text box.
    pub(crate) fn search_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.search_open = !self.search_open;
            self.search_query.clear();
            self.search_selected = 0;
        }
    }

    pub(crate) fn search_overlay(&mut self, ctx: &egui::Context) {
        if !self.search_open {
            return;
        }
        let hits = search(
            &self.search_query,
            &self.snapshot.tickets,
            self.profile.as_ref().ok(),
            self.current(),
        );
        let (down, up, enter, escape) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if escape {
            self.search_open = false;
            return;
        }
        if down {
            self.search_selected = (self.search_selected + 1).min(hits.len().saturating_sub(1));
        }
        if up {
            self.search_selected = self.search_selected.saturating_sub(1);
        }
        let mut chosen = enter
            .then(|| hits.get(self.search_selected).cloned())
            .flatten();
        egui::Window::new("Search")
            .title_bar(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
            .fixed_size(egui::vec2(560.0, 0.0))
            .show(ctx, |ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text(
                            "Search tickets, projects, pipelines, settings. t: p: pl: > narrow it",
                        )
                        .desired_width(f32::INFINITY),
                );
                edit.request_focus();
                if edit.changed() {
                    self.search_selected = 0;
                }
                if hits.is_empty() {
                    ui.label(theme::muted("Nothing matches."));
                }
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (at, hit) in hits.iter().enumerate() {
                            let color = if hit.disabled.is_some() {
                                theme::TEXT_MUTED()
                            } else {
                                theme::TEXT()
                            };
                            let fill = if at == self.search_selected {
                                theme::SELECTED()
                            } else {
                                Color32::TRANSPARENT
                            };
                            let text = format!("{}   {}   {}", hit.kind, hit.label, hit.detail);
                            let row = ui.add(
                                egui::Button::new(RichText::new(text).color(color))
                                    .fill(fill)
                                    .min_size(egui::vec2(ui.available_width(), 28.0)),
                            );
                            let row = match hit.disabled {
                                Some(why) => row.on_hover_text(why),
                                None => row,
                            };
                            if row.clicked() {
                                chosen = Some(hit.clone());
                            }
                        }
                    });
            });
        if let Some(hit) = chosen
            && hit.disabled.is_none()
        {
            self.search_open = false;
            self.open_hit(&hit.target);
        }
    }

    /// Act on a result. A command calls the same action a button would; the
    /// search starts nothing itself.
    pub(crate) fn open_hit(&mut self, target: &Target) {
        match target {
            Target::Ticket(id) => self.open(id),
            Target::Project(path) => {
                self.select_project(path);
                self.mode = Mode::Projects;
            }
            Target::Pipeline(name) => {
                self.pipeline_selected = Some(name.clone());
                self.mode = Mode::Pipelines;
            }
            Target::Page(mode) | Target::Command(Command::Go(mode)) => self.mode = *mode,
            Target::Command(Command::NewTicket) => self.start_new(),
            Target::Command(Command::SwitchTheme) => {
                let next = match self.theme {
                    theme::ThemeChoice::System => theme::ThemeChoice::Light,
                    theme::ThemeChoice::Light => theme::ThemeChoice::Dark,
                    theme::ThemeChoice::Dark => theme::ThemeChoice::System,
                };
                self.choose_theme(next);
            }
            Target::Command(Command::PauseOrResume) => {
                if let Some(view) = self.current().cloned() {
                    let action = if view.badge == Badge::Paused {
                        Action::Resume(view.id)
                    } else {
                        Action::Pause(view.id)
                    };
                    self.act(action);
                }
            }
            Target::Command(Command::OpenWorktree) => {
                if let Some(view) = self.current().cloned() {
                    self.act(Action::OpenWorktree(view.id));
                }
            }
        }
    }
}
