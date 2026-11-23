//! Step detail: the tabs, the actions for one step and the change request.

use autopilot_core::Step;
use autopilot_engine::{StepField, TicketView, dollars, step_name};
use autopilot_pipeline::StepState;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke};

use crate::app::{App, Screen, Tab};
use crate::backend::Action;
use crate::run::{clock, state_text};
use crate::theme;

const TABS: [(Tab, &str); 4] = [
    (Tab::Summary, "Summary"),
    (Tab::Output, "Output"),
    (Tab::Log, "Log"),
    (Tab::Changes, "Changes"),
];

fn why_it_ran(step: Step) -> &'static str {
    match step {
        Step::Understand | Step::Plan => "It always runs.",
        Step::Clarify => "It runs only when Understand has open questions.",
        Step::Design => "It runs only when the ticket changes the user interface.",
        Step::BuildCheck => "Your approved plan started it. Build ⟲ Check always runs.",
        Step::PullRequest => "It runs when every check has passed.",
        Step::Custom(_) => "You added it to this pipeline. It runs after Build ⟲ Check.",
    }
}

impl App {
    #[allow(
        clippy::too_many_lines,
        reason = "one screen drawn top to bottom; the sections share the same card"
    )]
    pub(crate) fn drawer(&mut self, ui: &mut egui::Ui, view: &TicketView, step: Step) {
        let view = view.clone();
        let Some(card) = view.step(step).cloned() else {
            return;
        };
        ui.horizontal(|ui| {
            if ui.link("× Close (Esc)").clicked() {
                self.screen = Screen::Run;
            }
            ui.label(RichText::new(step_name(step)).size(18.0).strong());
            let (fill, color) = match card.state {
                StepState::Passed => (theme::GREEN_FILL(), theme::GREEN()),
                StepState::Failed(_) | StepState::Interrupted(_) => {
                    (theme::RED_FILL(), theme::RED())
                }
                StepState::Asking(_) => (theme::AMBER_FILL(), theme::AMBER()),
                StepState::Running => (theme::BLUE_FILL(), theme::BLUE_TEXT()),
                _ => (theme::GREY_FILL(), theme::GREY_TEXT()),
            };
            theme::pill(ui, &state_text(&card.state), fill, color);
        });
        ui.horizontal_wrapped(|ui| {
            for other in &view.steps {
                let (mark, fill) = match other.state {
                    StepState::Passed => ("✔ ", theme::GREEN_FILL()),
                    StepState::Running => ("▶ ", theme::BLUE_FILL()),
                    StepState::Skipped(_) => ("", Color32::TRANSPARENT),
                    _ => ("", theme::GREY_FILL()),
                };
                let text = format!("{mark}{}", step_name(other.step));
                let button = egui::Button::new(RichText::new(text).size(13.0))
                    .fill(fill)
                    .stroke(Stroke::new(
                        1.0_f32,
                        if other.step == step {
                            theme::BLUE()
                        } else {
                            theme::BORDER()
                        },
                    ))
                    .corner_radius(egui::CornerRadius::same(99));
                if ui.add(button).clicked() {
                    self.screen = Screen::Step(other.step);
                }
            }
        });
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("IN SHORT")
                        .size(13.0)
                        .strong()
                        .color(theme::TEXT_MUTED()),
                );
                theme::pill(
                    ui,
                    "made from the run · free",
                    theme::GREEN_FILL(),
                    theme::GREEN(),
                );
            });
            ui.label(
                RichText::new(
                    card.in_short
                        .clone()
                        .unwrap_or_else(|| state_text(&card.state)),
                )
                .size(17.0),
            );
            let explained = view
                .explained
                .iter()
                .find(|(kept, _)| *kept == step)
                .map(|(_, text)| text.clone());
            if let Some(text) = explained {
                ui.label(theme::muted("In plain words"));
                ui.label(text);
            } else {
                let has_result = view.outputs.iter().any(|(kept, _)| *kept == step);
                if ui
                    .add_enabled(
                        has_result && !self.busy,
                        egui::Button::new("Explain in plain words"),
                    )
                    .on_hover_text(
                        "Asks a small model once, with the step's result, and keeps the answer",
                    )
                    .on_disabled_hover_text("This step has no result to explain yet")
                    .clicked()
                {
                    self.act(Action::Explain {
                        ticket: view.id.clone(),
                        step,
                    });
                }
            }
        });
        ui.add_space(6.0);
        let mut tab = self.tab;
        theme::segmented(ui, &mut tab, &TABS);
        self.tab = tab;
        ui.add_space(6.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(240.0);
            self.tab_body(ui, &view, step);
        });
        ui.add_space(8.0);
        self.step_side(ui, &view, step);
    }

    fn tab_body(&mut self, ui: &mut egui::Ui, view: &TicketView, step: Step) {
        let none = |ui: &mut egui::Ui, text: &str| {
            ui.label(theme::muted(text));
        };
        match self.tab {
            Tab::Summary => {
                self.overview(ui, view, step);
                ui.add_space(8.0);
                ui.label(RichText::new("Cost").strong());
                Self::cost_lines(ui, view, step);
                ui.add_space(8.0);
                ui.label(RichText::new("Settings").strong());
                self.settings_tab(ui, step);
            }
            Tab::Output => match view.outputs.iter().find(|(kept, _)| *kept == step) {
                Some((_, text)) => {
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            ui.label(theme::mono(text));
                        });
                }
                None => none(ui, "This step has not produced a result."),
            },
            Tab::Log => {
                none(
                    ui,
                    "Input and prompt are not recorded yet. The console follows.",
                );
                let lines: Vec<_> = view
                    .log
                    .iter()
                    .filter(|line| line.step == Some(step))
                    .collect();
                if lines.is_empty() {
                    none(ui, "The agent console is empty for this step.");
                }
                for line in lines {
                    ui.label(theme::mono(format!(
                        "{}   {}",
                        clock(line.at_ms),
                        line.text
                    )));
                }
            }
            Tab::Changes => none(ui, "The diff of this step is not recorded yet."),
        }
    }

    fn cost_lines(ui: &mut egui::Ui, view: &TicketView, step: Step) {
        let lines: Vec<_> = view
            .costs
            .iter()
            .filter(|line| line.step == Some(step))
            .collect();
        if lines.is_empty() {
            ui.label(theme::muted("No cost was recorded for this step."));
        }
        for line in lines {
            ui.horizontal(|ui| {
                theme::cost_tag(ui, line.estimated);
                ui.label(theme::mono(format!(
                    "{}   {}",
                    clock(line.at_ms),
                    dollars(line.amount)
                )));
            });
        }
    }

    fn overview(&self, ui: &mut egui::Ui, view: &TicketView, step: Step) {
        let ask = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.step(step))
            .is_some_and(|settings| settings.ask_after);
        // Stacked, not two columns: the drawer is too narrow for two readable
        // columns, and their text ran together.
        ui.label(theme::muted("Why it ran"));
        ui.label(why_it_ran(step));
        ui.add_space(4.0);
        ui.label(theme::muted("Asks you"));
        ui.label(if ask {
            "Yes: it waits for your approval when it finishes."
        } else {
            "Not unless a rule forces it, or you tick the box."
        });
        if step == Step::BuildCheck {
            ui.add_space(8.0);
            ui.label(RichText::new("Rounds").strong());
            if view.rounds.is_empty() {
                ui.label(theme::muted("No round has started."));
            }
            for round in &view.rounds {
                ui.horizontal(|ui| {
                    ui.label(format!("Round {}", round.round));
                    ui.label(theme::mono(format!(
                        "{} of {} checks passed",
                        round.passed(),
                        round.checks.len()
                    )));
                    if let Some(failed) = round.failure() {
                        ui.label(
                            RichText::new(format!("failed at {}: {}", failed.name, failed.note))
                                .color(theme::AMBER()),
                        );
                    }
                });
            }
        }
        if step == Step::Plan
            && let Some((_, text)) = view.outputs.iter().find(|(kept, _)| *kept == Step::Plan)
            && let Ok(plan) = serde_json_criteria(text)
        {
            ui.add_space(8.0);
            ui.label(RichText::new("Plan criteria").strong());
            for criterion in plan {
                ui.label(format!("○ {criterion}"));
            }
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui, step: Step) {
        let Some(settings) = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.step(step))
            .cloned()
        else {
            ui.label(theme::muted("The profile could not be read."));
            return;
        };
        let rows = [
            ("auth", format!("{:?}", settings.auth).to_lowercase()),
            (
                "model",
                settings.model.clone().unwrap_or_else(|| "default".into()),
            ),
            (
                "effort",
                settings.effort.clone().unwrap_or_else(|| "default".into()),
            ),
            ("turns", settings.limits.turns.to_string()),
            ("refine rounds", settings.limits.rounds.to_string()),
        ];
        for (name, value) in rows {
            ui.horizontal(|ui| {
                ui.label(theme::muted(name));
                ui.label(theme::mono(value));
            });
        }
    }

    #[allow(clippy::too_many_lines, reason = "one card per group of actions")]
    pub(crate) fn step_side(&mut self, ui: &mut egui::Ui, view: &TicketView, step: Step) {
        ui.label(
            RichText::new("ACTIONS FOR THIS STEP")
                .size(13.0)
                .strong()
                .color(theme::TEXT_MUTED()),
        );
        let card = view.step(step).map(|card| card.state.clone());
        // A card's own margins are 28 wide, so a button of this width fills one.
        let width = ui.available_width() - 28.0;
        let button = |text: &str| egui::Button::new(text).min_size(egui::vec2(width, 40.0));
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::muted("Run"));
            // Only Understand runs for real today: the other steps need a
            // real agent, which stays refused until S6.
            let settled = matches!(
                card,
                Some(
                    StepState::Passed
                        | StepState::Failed(_)
                        | StepState::Interrupted(_)
                        | StepState::Skipped(_)
                )
            );
            let busy = view.steps.iter().any(|other| other.state.is_active());
            let rerunnable = matches!(step, Step::Understand | Step::Design | Step::Plan);
            if rerunnable && settled && !busy {
                let rerun = ui.add(button("Re-run this step")).on_hover_text(
                    "Runs the step again with the same words, answers and files. Later steps that ran are reset with it.",
                );
                if rerun.clicked() {
                    self.act(match step {
                        Step::Understand => Action::RerunUnderstand(view.id.clone()),
                        Step::Design => Action::RerunDesign(view.id.clone()),
                        _ => Action::RerunPlan(view.id.clone()),
                    });
                }
            } else {
                let why = if rerunnable {
                    "The step has not finished, or another step is running or waiting for you"
                } else {
                    "Coming soon: this step cannot run until code can be written safely (containment, S6)"
                };
                ui.add_enabled(false, button("Re-run this step"))
                    .on_disabled_hover_text(why);
            }
            if step == Step::Design
                && ui
                    .add(button("Open the mockups"))
                    .on_hover_text("Shows the saved pages in the file manager. They hold no script and load nothing.")
                    .clicked()
            {
                self.act(Action::OpenDesign(view.id.clone()));
            }
            if matches!(card, Some(StepState::Skipped(_))) {
                ui.add_enabled(false, button("Run it anyway"))
                    .on_disabled_hover_text(
                        "Not available: a skipped step had nothing to do. Re-run an earlier step instead.",
                    );
            }
        });
        ui.add_space(8.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::muted("Control"));
            let paused = view.badge == autopilot_engine::Badge::Paused;
            if ui
                .add(button(if paused {
                    "Resume"
                } else {
                    "Pause after this step"
                }))
                .clicked()
            {
                self.act(if paused {
                    Action::Resume(view.id.clone())
                } else {
                    Action::Pause(view.id.clone())
                });
            }
            let halt = egui::Button::new(RichText::new("Stop this step").color(theme::RED()))
                .min_size(egui::vec2(width, 40.0));
            if ui.add(halt).clicked() {
                self.act(Action::Stop(view.id.clone()));
            }
            let mut ask = self
                .profile
                .as_ref()
                .ok()
                .and_then(|profile| profile.step(step))
                .is_some_and(|settings| settings.ask_after);
            if ui
                .checkbox(&mut ask, "Ask me after this step, next time")
                .changed()
            {
                self.apply_edit(step, &StepField::AskAfter(ask));
            }
            if let Some((problem_step, problem)) = &self.profile_problem
                && *problem_step == step
            {
                ui.label(RichText::new(problem).color(theme::AMBER()));
            }
        });
        ui.add_space(8.0);
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::muted("Open"));
            for text in [
                "Open the worktree",
                "Open in a terminal (resume the session)",
                "Copy the session id",
            ] {
                ui.add_enabled(false, button(text)).on_disabled_hover_text(
                    "Not built yet: the real agent session and worktree arrive with S6",
                );
            }
        });
        ui.add_space(8.0);
        if let Some((request, text)) = view
            .asking
            .as_ref()
            .filter(|(asked, _)| *asked == step)
            .and_then(|(_, request)| {
                self.waiting_for(&view.id, request)
                    .map(|text| (request.clone(), text))
            })
        {
            let _ = request;
            Frame::new()
                .fill(theme::AMBER_FILL())
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("This step is asking")
                            .strong()
                            .color(theme::AMBER()),
                    );
                    ui.label(text);
                    if step != Step::Plan {
                        ui.checkbox(
                            &mut self.always,
                            "Do not ask again after this step in this project",
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary("Approve")).clicked() {
                            self.act(Action::Decide {
                                ticket: view.id.clone(),
                                approve: true,
                                always: self.always,
                            });
                        }
                        if ui.button("Reject").clicked() {
                            self.act(Action::Decide {
                                ticket: view.id.clone(),
                                approve: false,
                                always: false,
                            });
                        }
                    });
                });
        }
    }
}

/// The criteria texts inside a stored plan, without a JSON dependency: the
/// plan is pretty-printed, so each criterion's `"text"` is on its own line.
fn serde_json_criteria(plan: &str) -> Result<Vec<String>, ()> {
    let texts: Vec<String> = plan
        .lines()
        .filter_map(|line| line.trim().strip_prefix("\"text\": \""))
        .map(|rest| rest.trim_end_matches(',').trim_end_matches('"').to_owned())
        .collect();
    if texts.is_empty() { Err(()) } else { Ok(texts) }
}
