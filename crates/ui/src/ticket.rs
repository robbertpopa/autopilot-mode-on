//! The ticket page: one story, newest last, a composer under it, and a right
//! panel with the vertical stepper. A click on a story entry or a step opens
//! the drawer (`step.rs`) in the same panel.

use autopilot_core::{Step, TicketId};
use autopilot_engine::{TicketView, dollars, step_name};
use autopilot_pipeline::StepState;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Shape, Stroke};

use crate::app::{App, Screen};
use crate::backend::Action;
use crate::run::state_text;
use crate::theme;

const CODE_WHY: &str =
    "code changes follow the approved plan: press Build on the ticket once the plan is approved";
const ASK_WHY: &str = "coming soon: a side question to the agent is not built yet";
const UNSURE_WHY: &str =
    "recorded per ticket, but no step reads it yet, so a switch here would change nothing";

/// The mark and colour for a step in the stepper and the story.
fn mark(state: &StepState) -> (&'static str, Color32) {
    match state {
        StepState::Passed => ("✔", theme::GREEN()),
        StepState::Running => ("▶", theme::BLUE_TEXT()),
        StepState::Asking(_) => ("?", theme::AMBER()),
        StepState::Failed(_) | StepState::Interrupted(_) => ("×", theme::RED()),
        StepState::Paused => ("⏸", theme::GREY_TEXT()),
        StepState::Skipped(_) | StepState::Idle => ("-", theme::TEXT_MUTED()),
    }
}

/// What the owner asked, in full. The title is cut to one line, so without
/// this the original request cannot be read again.
fn prompt_card(ui: &mut egui::Ui, view: &TicketView) {
    if view.description.trim().is_empty() {
        return;
    }
    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(RichText::new("YOUR PROMPT").monospace().size(11.0).strong())
            .id_salt(("prompt", view.id.as_str()))
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(view.description.as_str()).selectable(true));
                    });
            });
    });
    ui.add_space(10.0);
}

/// The step the ticket is on: the first one that has not finished or been
/// skipped. `None` when everything is done.
pub(crate) fn current_step(view: &TicketView) -> Option<&autopilot_engine::StepView> {
    view.steps
        .iter()
        .find(|card| !matches!(card.state, StepState::Passed | StepState::Skipped(_)))
}

/// Minutes and seconds, `2:05`.
fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// One line that says what the ticket is doing right now. Only a running
/// ticket gets a spinner, so motion always means work is happening.
pub(crate) fn live_status(ui: &mut egui::Ui, view: &TicketView) {
    live_status_at(ui, view, crate::now_ms());
}

/// Seconds as `3s`, or `2:05` from a minute on.
fn ago(ms: u64) -> String {
    if ms < 60_000 {
        format!("{}s", ms / 1000)
    } else {
        clock(ms)
    }
}

/// The sub-steps of a step, ticked as they happen. They stay after it ends.
pub(crate) fn sub_step_list(ui: &mut egui::Ui, view: &TicketView, step: Step) {
    use autopilot_engine::SubState;
    for item in autopilot_engine::sub_steps(view, step) {
        let (mark, color, note) = match &item.state {
            SubState::Done => ("✔", theme::GREEN(), None),
            SubState::Active => ("▶", theme::BLUE_TEXT(), Some("in progress".to_owned())),
            SubState::Pending => ("○", theme::TEXT_MUTED(), None),
            SubState::Failed => ("×", theme::RED(), None),
            SubState::Stopped(reason) => ("■", theme::RED(), Some(format!("stopped: {reason}"))),
            SubState::Skipped(reason) => {
                ("–", theme::TEXT_MUTED(), Some(format!("skipped: {reason}")))
            }
        };
        // A plain row, not a wrapped one: in a wrapped row the mark and the
        // text started on the same spot and were drawn over each other.
        ui.horizontal(|ui| {
            ui.label(RichText::new(mark).color(color));
            ui.add(egui::Label::new(theme::muted(item.text)).wrap());
            if let Some(note) = note {
                ui.add(egui::Label::new(RichText::new(note).color(color)).wrap());
            }
        });
    }
}

pub(crate) fn live_status_at(ui: &mut egui::Ui, view: &TicketView, now_ms: u64) {
    let Some(card) = current_step(view) else {
        return;
    };
    let name = step_name(card.step);
    let (text, color) = match &card.state {
        StepState::Running if crate::run::pull_request_approved(view) => (
            "Pull request approved: waiting for you to open it".to_owned(),
            theme::BLUE_TEXT(),
        ),
        StepState::Running => {
            let since = now_ms.saturating_sub(card.started_ms.unwrap_or(view.started_ms));
            match autopilot_engine::live_status(view, card.step, now_ms) {
                Some(live) => {
                    let beat = match live.liveness {
                        autopilot_engine::Liveness::Active => {
                            format!("last activity {} ago", ago(live.idle_ms))
                        }
                        autopilot_engine::Liveness::Quiet => {
                            format!("quiet for {}", ago(live.idle_ms))
                        }
                        autopilot_engine::Liveness::Stalled => {
                            format!("no activity for {}: it may be stuck", ago(live.idle_ms))
                        }
                    };
                    let color = match live.liveness {
                        autopilot_engine::Liveness::Active => theme::BLUE_TEXT(),
                        autopilot_engine::Liveness::Quiet => theme::AMBER(),
                        autopilot_engine::Liveness::Stalled => theme::RED(),
                    };
                    let cost = match &live.progress {
                        Some(progress) => format!("{} · {progress}", dollars(live.spent)),
                        None => dollars(live.spent),
                    };
                    (
                        format!(
                            "Running {name} · {} · {beat} · {cost} · {} since start",
                            live.phase,
                            clock(since)
                        ),
                        color,
                    )
                }
                None => (
                    format!("Running {name} · {} since start", clock(since)),
                    theme::BLUE_TEXT(),
                ),
            }
        }
        StepState::Asking(_) => (format!("{name} is waiting for you"), theme::AMBER()),
        StepState::Paused => (format!("Paused at {name}"), theme::GREY_TEXT()),
        StepState::Failed(_) | StepState::Interrupted(_) => {
            (format!("Stopped at {name}"), theme::RED())
        }
        _ => return,
    };
    let running =
        matches!(card.state, StepState::Running) && !crate::run::pull_request_approved(view);
    ui.horizontal(|ui| {
        if running {
            ui.add(egui::Spinner::new().size(14.0).color(color));
            // The clock must keep moving while nothing else changes.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        // Wraps: the heartbeat line is long, and a horizontal row would widen
        // the whole page past the window.
        ui.add(egui::Label::new(RichText::new(text).color(color).strong()).wrap());
    });
    if running {
        if let Some(live) = autopilot_engine::live_status(view, card.step, now_ms)
            && let Some(detail) = live.detail
        {
            ui.label(theme::muted(format!("      {detail}")));
        }
        sub_step_list(ui, view, card.step);
    }
}

/// Steps that count towards progress, finished first. Skipped steps are out.
fn progress(view: &TicketView) -> (usize, usize) {
    let total = view
        .steps
        .iter()
        .filter(|card| !matches!(card.state, StepState::Skipped(_)))
        .count();
    let done = view
        .steps
        .iter()
        .filter(|card| matches!(card.state, StepState::Passed))
        .count();
    (done, total)
}

/// "2 of 7 steps" and a thin bar above the stepper.
fn progress_bar(ui: &mut egui::Ui, view: &TicketView) {
    let (done, total) = progress(view);
    ui.label(RichText::new(format!("{done} of {total} steps")).strong());
    #[allow(clippy::cast_precision_loss, reason = "a handful of steps")]
    let fraction = if total == 0 {
        0.0
    } else {
        done as f32 / total as f32
    };
    ui.add(
        egui::ProgressBar::new(fraction)
            .desired_height(6.0)
            .fill(theme::GREEN()),
    );
    ui.add_space(8.0);
}

/// The dot for one step: filled when finished, a pulsing ring while it runs,
/// hollow when it has not started. Colour is never the only signal: the row
/// also carries the state as text.
fn draw_dot(ui: &egui::Ui, center: egui::Pos2, state: &StepState) {
    let painter = ui.painter();
    let radius = 7.0_f32;
    match state {
        StepState::Passed => {
            painter.circle_filled(center, radius, theme::GREEN());
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "✔",
                egui::FontId::proportional(9.0),
                Color32::WHITE,
            );
        }
        StepState::Running => {
            #[allow(clippy::cast_possible_truncation, reason = "a value between 0 and 1")]
            let phase = (ui.input(|input| input.time) % 1.2 / 1.2) as f32;
            painter.circle_stroke(
                center,
                radius + 5.0 * phase,
                Stroke::new(2.0_f32, theme::BLUE_TEXT().gamma_multiply(1.0 - phase)),
            );
            painter.circle_filled(center, radius, theme::BLUE_TEXT());
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(40));
        }
        StepState::Asking(_) => {
            painter.circle_filled(center, radius, theme::AMBER());
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
        StepState::Failed(_) | StepState::Interrupted(_) => {
            painter.circle_filled(center, radius, theme::RED());
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "×",
                egui::FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        StepState::Paused => {
            painter.circle_filled(center, radius, theme::GREY_TEXT());
        }
        StepState::Skipped(_) | StepState::Idle => {
            painter.circle_stroke(
                center,
                radius - 1.0,
                Stroke::new(1.5_f32, theme::TEXT_MUTED()),
            );
        }
    }
}

/// One row of the vertical stepper, joined to its neighbours by a line that
/// is green up to the last finished step.
fn stepper_row(
    ui: &mut egui::Ui,
    card: &autopilot_engine::StepView,
    tag: &str,
    current: bool,
    (first, last): (bool, bool),
    approved: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    let painter = ui.painter();
    if current {
        painter.rect_filled(rect, egui::CornerRadius::same(6), theme::SELECTED());
    }
    let x = rect.left() + 14.0;
    let y = rect.center().y;
    let done = matches!(card.state, StepState::Passed);
    let line = Stroke::new(
        2.0_f32,
        if done {
            theme::GREEN()
        } else {
            theme::BORDER()
        },
    );
    if !first {
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, y - 8.0)], line);
    }
    if !last {
        painter.line_segment([egui::pos2(x, y + 8.0), egui::pos2(x, rect.bottom())], line);
    }
    draw_dot(ui, egui::pos2(x, y), &card.state);
    let skipped = matches!(card.state, StepState::Skipped(_));
    let text_color = if skipped {
        theme::TEXT_MUTED()
    } else {
        ui.visuals().text_color()
    };
    let font = if current {
        egui::FontId::proportional(14.0)
    } else {
        egui::FontId::proportional(13.5)
    };
    let name_width = ui
        .painter()
        .layout_no_wrap(step_name(card.step).to_owned(), font.clone(), text_color)
        .size()
        .x;
    ui.painter().text(
        egui::pos2(rect.left() + 32.0, y),
        egui::Align2::LEFT_CENTER,
        step_name(card.step),
        font,
        text_color,
    );
    let right = if approved && card.state == StepState::Running {
        "approved"
    } else if current && !matches!(card.state, StepState::Idle) {
        short_state(&card.state)
    } else {
        tag
    };
    // Whatever does not fit beside the name is cut to a shorter word, so the
    // state never prints over the step's name.
    let room = rect.width() - 32.0 - name_width - 16.0;
    let right = fit_state(ui, right, room);
    ui.painter().text(
        egui::pos2(rect.right() - 8.0, y),
        egui::Align2::RIGHT_CENTER,
        right,
        egui::FontId::monospace(11.0),
        mark(&card.state).1,
    );
    response
}

/// The state text that fits in `room` points: the full words, a shorter word,
/// or nothing.
fn fit_state<'a>(ui: &egui::Ui, text: &'a str, room: f32) -> &'a str {
    let width = |text: &str| {
        ui.painter()
            .layout_no_wrap(
                text.to_owned(),
                egui::FontId::monospace(11.0),
                Color32::WHITE,
            )
            .size()
            .x
    };
    let shorter = match text {
        "waiting for you" => "waiting",
        "interrupted" => "stopped",
        "ask · locked" => "locked",
        other => other,
    };
    [text, shorter]
        .into_iter()
        .find(|candidate| width(candidate) <= room)
        .unwrap_or("")
}

/// The state as one or two words, for the current row.
fn short_state(state: &StepState) -> &'static str {
    match state {
        StepState::Running => "running",
        StepState::Asking(_) => "waiting for you",
        StepState::Paused => "paused",
        StepState::Failed(_) => "failed",
        StepState::Interrupted(_) => "interrupted",
        StepState::Passed => "done",
        StepState::Skipped(_) => "skipped",
        StepState::Idle => "",
    }
}

/// A dashed outline, because a skipped step is drawn dashed with its reason.
fn dashed(ui: &egui::Ui, rect: egui::Rect) {
    let points = vec![
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    let stroke = Stroke::new(1.0_f32, theme::TEXT_MUTED());
    ui.painter()
        .extend(Shape::dashed_line(&points, stroke, 5.0, 4.0));
}

/// The worktree chip text. A worktree exists only once Build has run.
fn worktree_chip(view: &TicketView) -> &'static str {
    if view.rounds.is_empty() {
        "no worktree yet"
    } else {
        "worktree · isolated"
    }
}

impl App {
    /// The ticket page: top bar, story, composer.
    pub(crate) fn ticket_page(&mut self, ui: &mut egui::Ui, id: &TicketId) {
        let Some(view) = self.ticket(id).cloned() else {
            return;
        };
        self.run_header(ui, &view);
        if view.archived {
            ui.horizontal(|ui| {
                ui.label(theme::muted("Archived: hidden from the lists."));
                if ui.button("Restore").clicked() {
                    self.act(Action::Restore(view.id.clone()));
                }
            });
        }
        ui.horizontal(|ui| {
            let project = crate::nav::project_name(&view);
            theme::pill(
                ui,
                project.as_deref().unwrap_or("no project"),
                theme::GREY_FILL(),
                theme::GREY_TEXT(),
            );
            theme::pill(
                ui,
                worktree_chip(&view),
                theme::GREY_FILL(),
                theme::GREY_TEXT(),
            );
        });
        ui.add_space(12.0);
        // A step opened from the stepper shows in this column; the stepper stays
        // where it was, so the window does not change shape.
        if let Screen::Step(step) = self.screen {
            self.drawer(ui, &view, step);
            return;
        }
        prompt_card(ui, &view);
        self.next_card(ui, &view);
        self.overlap_notice(ui, &view);
        queue_notice(ui, &view);
        Self::orchestrator_card(ui, &view);
        self.advisor_box(ui, &view);
        self.story(ui, &view);
        ui.add_space(6.0);
        self.build_panel(ui, &view);
        // The pull request feedback card (stream 1) is drawn here, below the
        // checks it follows from.
        self.files_panel(ui, &view);
    }

    /// Another open ticket in this project that predicts the same files. It
    /// only tells: starting, holding or merging is the owner's call.
    fn overlap_notice(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let key = (view.id.clone(), view.last_seq);
        let fresh = self
            .overlaps
            .as_ref()
            .is_some_and(|(id, seq, _)| *id == key.0 && *seq == key.1);
        if !fresh {
            let found = autopilot_engine::find_overlaps(&self.options.data_dir, &view.id)
                .unwrap_or_default();
            self.overlaps = Some((key.0, key.1, found));
        }
        let Some((_, _, found)) = self.overlaps.clone() else {
            return;
        };
        let mut open = None;
        for overlap in found {
            crate::app::banner(
                ui,
                &format!(
                    "{} also changes {}. Finish one first, or open it to compare.",
                    overlap.title, overlap.path
                ),
                false,
            );
            if ui.link(format!("Open {}", overlap.title)).clicked() {
                open = Some(overlap.other);
            }
        }
        if let Some(id) = open {
            self.open(&id);
        }
    }

    /// One entry per step that has started, in order, so the newest is last.
    /// Skipped steps are dashed and say why.
    fn story(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let mut open = None;
        for card in &view.steps {
            // A step that has not started is listed only when it is next, with
            // its plan, so the owner sees what it will do before it does it.
            if matches!(card.state, StepState::Idle)
                && current_step(view).is_none_or(|next| next.step != card.step)
            {
                continue;
            }
            // Build shows its rounds in the build panel below.
            if card.step == Step::BuildCheck && !view.rounds.is_empty() {
                continue;
            }
            let skipped = matches!(card.state, StepState::Skipped(_));
            let (glyph, color) = mark(&card.state);
            let shown = Frame::new()
                .fill(if skipped {
                    Color32::TRANSPARENT
                } else {
                    theme::CARD()
                })
                .stroke(if skipped {
                    Stroke::NONE
                } else {
                    Stroke::new(1.0_f32, theme::BORDER())
                })
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 24.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(glyph).color(color));
                        ui.label(RichText::new(step_name(card.step)).strong());
                        let approved = card.step == Step::PullRequest
                            && crate::run::pull_request_approved(view);
                        ui.label(theme::muted(if approved {
                            "approved, not opened yet".to_owned()
                        } else if matches!(card.state, StepState::Idle) {
                            "waiting".to_owned()
                        } else {
                            state_text(&card.state)
                        }));
                    });
                    if let Some(short) = &card.in_short {
                        ui.label(short);
                    }
                    // The step's plan, ticked from the log; open for the step
                    // that is on now, closed for the rest.
                    if !skipped {
                        egui::CollapsingHeader::new(theme::muted("Plan"))
                            .id_salt((view.id.as_str(), card.step.as_str(), "plan"))
                            .default_open(
                                card.state.is_active()
                                    || current_step(view).is_some_and(|now| now.step == card.step),
                            )
                            .show(ui, |ui| sub_step_list(ui, view, card.step));
                    }
                });
            let rect = shown.response.rect;
            if skipped {
                dashed(ui, rect);
            }
            let click = ui.interact(
                rect,
                ui.id().with(("story", card.step.as_str())),
                egui::Sense::click(),
            );
            if theme::named(click, step_name(card.step)).clicked() {
                open = Some(card.step);
            }
            ui.add_space(6.0);
        }
        if view.badge == autopilot_engine::Badge::Done
            && let Ok(profile) = &self.profile
        {
            crate::health::review_card(ui, view, profile.review.peak_context_permille);
        }
        if let Some(step) = open {
            self.screen = Screen::Step(step);
        }
    }

    /// Mode row and the plain-words change box. Modes that need a real agent
    /// are greyed with their reason.
    pub(crate) fn composer_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (label, why) in [("code", CODE_WHY), ("ask", ASK_WHY)] {
                ui.add_enabled(false, egui::Button::new(label))
                    .on_disabled_hover_text(why);
            }
            let selected = egui::Button::new("pipeline").fill(theme::SELECTED());
            ui.add(selected)
                .on_hover_text("Change how this ticket runs, in plain words");
            ui.add_enabled(false, egui::Button::new("/btw"))
                .on_disabled_hover_text(ASK_WHY);
            ui.add_enabled(false, egui::Button::new("when_unsure ask"))
                .on_disabled_hover_text(UNSURE_WHY);
            if ui.link("approvals").clicked() {
                self.mode = crate::app::Mode::Design;
            }
        });
        self.change_box(ui);
    }

    /// The pipeline of this ticket. A change applies to steps that have not
    /// started; the engine refuses one that would switch off a step that ran.
    fn ticket_pipeline_select(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let Ok(profile) = &self.profile else {
            return;
        };
        let names: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
        let current = view.pipeline.as_ref().map_or_else(
            || autopilot_profile::DEFAULT_PIPELINE.to_owned(),
            |(name, _)| name.clone(),
        );
        let mut chosen = current.clone();
        egui::ComboBox::from_id_salt("ticket-pipeline")
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut chosen, name.clone(), name);
                }
            });
        if chosen != current {
            self.act(crate::backend::Action::ChoosePipeline {
                ticket: view.id.clone(),
                name: chosen,
            });
        }
    }

    /// The row that opens the ticket's copy of the project, once Build made it.
    fn open_worktree_row(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let worktree = autopilot_engine::worktree_of(&self.options.data_dir, &view.id);
        let row = egui::Button::new("Open the worktree")
            .frame(false)
            .min_size(egui::vec2(ui.available_width(), 28.0));
        if ui
            .add_enabled(worktree.is_some(), row)
            .on_disabled_hover_text(
                "There is no copy of the project for this ticket yet: it is made when Build starts.",
            )
            .clicked()
        {
            self.act(Action::OpenWorktree(view.id.clone()));
        }
    }

    /// The row that opens a terminal in the ticket's copy of the project.
    fn open_terminal_row(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let worktree = autopilot_engine::worktree_of(&self.options.data_dir, &view.id);
        let row = egui::Button::new("Open in a terminal")
            .frame(false)
            .min_size(egui::vec2(ui.available_width(), 28.0));
        if ui
            .add_enabled(worktree.is_some(), row)
            .on_disabled_hover_text(crate::backend::NO_WORKTREE)
            .clicked()
        {
            self.act(Action::OpenTerminal(view.id.clone()));
        }
    }

    /// The right panel without a drawer: pipeline, stepper, cost, actions.
    pub(crate) fn stepper_panel(&mut self, ui: &mut egui::Ui, view: &TicketView) {
        let mut open = None;
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("PIPELINE")
                    .monospace()
                    .size(11.0)
                    .strong()
                    .color(theme::TEXT_MUTED()),
            );
            self.ticket_pipeline_select(ui, view);
            ui.label(theme::muted("changes only steps not started"));
            ui.add_space(8.0);
            progress_bar(ui, view);
            // The step open in the main column is the one marked, so a click
            // is answered where the eye already is.
            let current = match self.screen {
                Screen::Step(open) => Some(open),
                Screen::Run => current_step(view).map(|card| card.step),
            };
            let last = view.steps.len().saturating_sub(1);
            for (index, card) in view.steps.iter().enumerate() {
                let ask = self
                    .profile
                    .as_ref()
                    .ok()
                    .and_then(|profile| profile.step(card.step))
                    .is_some_and(|settings| settings.ask_after);
                let locked = matches!(card.step, Step::Design | Step::PullRequest);
                let tag = if matches!(card.state, StepState::Skipped(_)) {
                    "skipped"
                } else if locked {
                    "ask · locked"
                } else if ask {
                    "ask"
                } else {
                    ""
                };
                let row = stepper_row(
                    ui,
                    card,
                    tag,
                    current == Some(card.step),
                    (index == 0, index == last),
                    card.step == Step::PullRequest && crate::run::pull_request_approved(view),
                );
                if theme::named(row, step_name(card.step)).clicked() {
                    open = Some(card.step);
                }
                if let StepState::Skipped(reason) = &card.state {
                    ui.label(theme::muted(format!("      {reason}")));
                }
            }
        });
        if let Some(step) = open {
            self.screen = Screen::Step(step);
        }
        ui.add_space(10.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("COST")
                    .monospace()
                    .size(11.0)
                    .strong()
                    .color(theme::TEXT_MUTED()),
            );
            ui.horizontal(|ui| {
                theme::cost_tag(ui, false);
                ui.label(dollars(view.api));
                theme::cost_tag(ui, true);
                ui.label(dollars(view.subscription));
            });
            ui.label(theme::muted("context  [not reported]"));
        });
        ui.add_space(10.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (text, why) in [
                (
                    "Re-run from here",
                    "open the step and use Re-run this step: Understand, Design and Plan can run again; to build again after a finished Build, reset it from the More menu",
                ),
                (
                    "Explain in plain words",
                    "open a step, then use Explain in its drawer",
                ),
            ] {
                ui.add_enabled(
                    false,
                    egui::Button::new(text)
                        .frame(false)
                        .min_size(egui::vec2(ui.available_width(), 28.0)),
                )
                .on_disabled_hover_text(why);
            }
            self.open_worktree_row(ui, view);
            self.open_terminal_row(ui, view);
        });
    }
}

/// Said while this ticket's Build waits for a free slot or for a ticket that
/// changes the same file; nothing is drawn once it runs.
fn queue_notice(ui: &mut egui::Ui, view: &TicketView) {
    if let Some(why) = crate::queue::BUILDS.waiting(&view.id) {
        crate::app::banner(ui, &format!("Build is queued: {why}."), false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(view: &TicketView, now_ms: u64) -> String {
        let context = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1200.0, 900.0),
            )),
            ..Default::default()
        };
        let output = context.run(input, |context| {
            egui::CentralPanel::default().show(context, |ui| {
                live_status_at(ui, view, now_ms);
                sub_step_list(ui, view, Step::BuildCheck);
            });
        });
        let mut text = String::new();
        for clipped in output.shapes {
            if let Shape::Text(shape) = clipped.shape {
                text.push_str(shape.galley.text());
                text.push('\n');
            }
        }
        text
    }

    fn view_of(name: &str) -> TicketView {
        let directory = tempfile::tempdir().unwrap();
        crate::demo::seed(directory.path(), 1_700_000_000_000).unwrap();
        crate::demo::seed_build_states(directory.path(), 1_700_000_000_000).unwrap();
        autopilot_engine::load_ticket(directory.path(), &TicketId::from(name)).unwrap()
    }

    // Invariant: a running build shows its phase and a heartbeat that turns
    // amber-worded "quiet" and then "stuck" as time passes with no event, and a
    // finished build keeps its sub-steps. A page that only said "Running" gave
    // no way to tell a thinking run from a dead one.
    #[test]
    fn a_running_step_shows_a_heartbeat_that_goes_quiet_and_stalled() {
        let view = view_of("demo-timezone");
        let active = drawn(&view, view.last_ms + 3_000);
        assert!(active.contains("Running Build"), "{active}");
        assert!(active.contains("last activity 3s ago"), "{active}");
        assert!(active.contains("round 2"), "{active}");
        assert!(active.contains("Round 1: build turn"), "{active}");
        let quiet = drawn(&view, view.last_ms + 40_000);
        assert!(quiet.contains("quiet for 40s"), "{quiet}");
        let stalled = drawn(&view, view.last_ms + 400_000);
        assert!(stalled.contains("it may be stuck"), "{stalled}");
        let done = drawn(&view_of("demo-passed"), 1_700_000_000_000);
        assert!(!done.contains("Running"), "{done}");
        assert!(
            done.contains("Round 1: tests"),
            "the sub-steps stay: {done}"
        );
    }

    // Invariant: a call that is still running shows its own turn and cost count
    // beside the recorded cost, so a long call does not look frozen. Breaks if
    // the window drops `LiveStatus::progress`.
    #[test]
    fn a_long_call_shows_its_running_turn_and_cost() {
        let mut view = view_of("demo-timezone");
        view.log.push(autopilot_engine::LogLine {
            step: Some(Step::BuildCheck),
            at_ms: view.last_ms + 1_000_000,
            text: "progress: turn 3, about $0.04 so far".to_owned(),
        });
        let text = drawn(&view, view.last_ms + 3_000);
        assert!(text.contains("turn 3, about $0.04 so far"), "{text}");
        let plain = drawn(&view_of("demo-timezone"), view.last_ms + 3_000);
        assert!(!plain.contains("so far"), "{plain}");
    }
}
