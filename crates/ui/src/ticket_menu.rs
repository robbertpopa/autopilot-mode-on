//! What the owner can do with a whole ticket from a menu: stop it, pause it,
//! carry it on from the last finished step, put it away, bring it back, or
//! delete it after a confirmation. The same menu is on the ticket page and on a
//! row of the sidebar.

use autopilot_core::{Step, TicketId};
use autopilot_engine::{Badge, TicketView};
use autopilot_pipeline::StepState;
use eframe::egui::{self, RichText};

use crate::{app::App, backend::Action, theme};

/// The action that carries a stopped ticket on from its last finished step: the
/// step that failed or was interrupted runs again, and everything before it
/// keeps its result.
pub(crate) fn resume_action(view: &TicketView) -> Option<Action> {
    let card = view
        .steps
        .iter()
        .find(|card| matches!(card.state, StepState::Failed(_) | StepState::Interrupted(_)))?;
    let id = view.id.clone();
    match card.step {
        Step::Understand => Some(Action::RerunUnderstand(id)),
        Step::Design => Some(Action::RerunDesign(id)),
        Step::Plan => Some(Action::RerunPlan(id)),
        Step::BuildCheck => Some(Action::MakeBuild(id)),
        Step::PullRequest => Some(Action::PreparePullRequest(id)),
        Step::Clarify | Step::Custom(_) => None,
    }
}

/// What the owner picked from the menu.
pub(crate) enum Choice {
    Act(Action),
    ConfirmDelete(TicketId),
}

/// The menu's items for one ticket; the caller applies the choice, because the
/// sidebar draws it while it still borrows the ticket list.
pub(crate) fn ticket_menu(ui: &mut egui::Ui, view: &TicketView) -> Option<Choice> {
    let live = matches!(view.badge, Badge::Running | Badge::NeedsYou | Badge::Paused);
    let mut choice = None;
    if view.archived {
        if ui.button("Restore").clicked() {
            choice = Some(Choice::Act(Action::Restore(view.id.clone())));
        }
    } else {
        if live && ui.button("Stop").clicked() {
            choice = Some(Choice::Act(Action::Stop(view.id.clone())));
        }
        if view.badge == Badge::Running && ui.button("Pause").clicked() {
            choice = Some(Choice::Act(Action::Pause(view.id.clone())));
        }
        if view.badge == Badge::Paused && ui.button("Resume").clicked() {
            choice = Some(Choice::Act(Action::Resume(view.id.clone())));
        }
        if let Some(action) = resume_action(view)
            && ui.button("Resume from the last finished step").clicked()
        {
            choice = Some(Choice::Act(action));
        }
        ui.separator();
        if ui
            .add_enabled(!live, egui::Button::new("Archive"))
            .on_disabled_hover_text("Stop it first: a running ticket cannot be put away.")
            .clicked()
        {
            choice = Some(Choice::Act(Action::Archive(view.id.clone())));
        }
    }
    if ui
        .add_enabled(!live, egui::Button::new("Delete…"))
        .on_disabled_hover_text("Stop it first: a running ticket cannot be deleted.")
        .clicked()
    {
        choice = Some(Choice::ConfirmDelete(view.id.clone()));
    }
    if choice.is_some() {
        ui.close();
    }
    choice
}

impl App {
    pub(crate) fn apply_choice(&mut self, choice: Option<Choice>) {
        match choice {
            Some(Choice::Act(action)) => self.act(action),
            Some(Choice::ConfirmDelete(id)) => self.confirm_delete = Some(id),
            None => {}
        }
    }

    /// The confirmation before a Build is reset: the copy of the project holds
    /// the agent's files and commits, and removing it cannot be undone.
    pub(crate) fn reset_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_reset.clone() else {
            return;
        };
        let title = self
            .ticket(&id)
            .map_or_else(|| id.as_str().to_owned(), |view| view.title.clone());
        egui::Window::new("Reset the Build?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(RichText::new(title).strong());
                ui.label(
                    "The copy of the project this Build worked in and its branch are deleted, with every file and commit in them, including work that was never committed. This cannot be undone. The ticket's history stays, and Build starts again from the project's base branch.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary("Delete the copy and reset")).clicked() {
                        self.act(Action::ResetBuild(id.clone()));
                        self.confirm_reset = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_reset = None;
                    }
                });
            });
    }

    /// The confirmation before a delete. It says what stays and what does not.
    pub(crate) fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_delete.clone() else {
            return;
        };
        let title = self
            .ticket(&id)
            .map_or_else(|| id.as_str().to_owned(), |view| view.title.clone());
        egui::Window::new("Delete this ticket?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(RichText::new(title).strong());
                ui.label(
                    "It disappears from every list. Its history stays on disk, but its worktree and branch are not removed by this: do that by hand if you want them gone.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary("Delete")).clicked() {
                        self.act(Action::Delete(id.clone()));
                        if self.selected.as_ref() == Some(&id) {
                            self.selected = None;
                        }
                        self.confirm_delete = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_delete = None;
                    }
                });
            });
    }
}
