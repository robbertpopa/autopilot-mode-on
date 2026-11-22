//! Settings > Pipeline review: Off, Notify only, Notify and tune, and the
//! thresholds. Edits wait
//! for Save and the diff is shown first, like the asking switches.

use autopilot_engine::{ReviewField, edit_review};
use autopilot_profile::{ReviewMode, ReviewSettings};
use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;

fn diff_lines(saved: &ReviewSettings, edit: &ReviewSettings) -> Vec<(ReviewField, String)> {
    let mut lines = Vec::new();
    if saved.mode != edit.mode {
        lines.push((
            ReviewField::Mode(edit.mode),
            format!("mode  {:?} -> {:?}", saved.mode, edit.mode),
        ));
    }
    for (name, before, after, field) in [
        (
            "window_runs",
            saved.window_runs,
            edit.window_runs,
            ReviewField::WindowRuns(edit.window_runs),
        ),
        (
            "peak_context_permille",
            saved.peak_context_permille,
            edit.peak_context_permille,
            ReviewField::PeakContextPermille(edit.peak_context_permille),
        ),
        (
            "min_hits",
            saved.min_hits,
            edit.min_hits,
            ReviewField::MinHits(edit.min_hits),
        ),
        (
            "cost_multiple_percent",
            saved.cost_multiple_percent,
            edit.cost_multiple_percent,
            ReviewField::CostMultiplePercent(edit.cost_multiple_percent),
        ),
    ] {
        if before != after {
            lines.push((field, format!("{name}  {before} -> {after}")));
        }
    }
    lines
}

impl App {
    pub(crate) fn review_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(RichText::new("Pipeline review").size(18.0).strong());
        ui.label(theme::muted(
            "Watches what each step uses and tells you when it drifts. Off until you turn it on.",
        ));
        let Ok(profile) = &self.profile else {
            ui.label(theme::muted("The profile could not be read."));
            return;
        };
        let saved = profile.review;
        let mut edit = self.review_edit.unwrap_or(saved);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (mode, label) in [
                (ReviewMode::Off, "Off"),
                (ReviewMode::NotifyOnly, "Notify only"),
            ] {
                if ui.selectable_label(edit.mode == mode, label).clicked() {
                    edit.mode = mode;
                }
            }
            if ui
                .selectable_label(edit.mode == ReviewMode::NotifyAndTune, "Notify and tune")
                .on_hover_text(
                    "Lowers effort one notch on Understand or Clarify of a custom pipeline when context or cost stays high. Reverts itself if the next 3 runs do worse; Undo is on the Health page.",
                )
                .clicked()
            {
                edit.mode = ReviewMode::NotifyAndTune;
            }
        });
        ui.add_space(8.0);
        for (label, value, range) in [
            ("window_runs", &mut edit.window_runs, 1..=50),
            (
                "peak_context_permille",
                &mut edit.peak_context_permille,
                1..=1000,
            ),
            ("min_hits", &mut edit.min_hits, 1..=50),
            (
                "cost_multiple_percent",
                &mut edit.cost_multiple_percent,
                100..=10_000,
            ),
        ] {
            ui.horizontal(|ui| {
                theme::cell(ui, 200.0, theme::mono(label));
                ui.add(egui::DragValue::new(value).range(range));
            });
        }
        ui.label(theme::muted(
            "Starting defaults, not measured facts. A finding needs min_hits runs that reported the figure.",
        ));
        self.review_edit = (edit != saved).then_some(edit);
        let changes = diff_lines(&saved, &edit);
        if !changes.is_empty() {
            ui.add_space(8.0);
            theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                ui.label(RichText::new("Unsaved changes").strong());
                for (_, line) in &changes {
                    ui.label(theme::mono(line));
                }
                ui.horizontal(|ui| {
                    if ui.add(theme::primary("Save")).clicked() {
                        self.save_review(&changes);
                    }
                    if ui.button("Discard").clicked() {
                        self.review_edit = None;
                    }
                });
            });
        }
        if let Some((ok, text)) = &self.pipeline_notice {
            let color = if *ok { theme::GREEN() } else { theme::RED() };
            ui.label(RichText::new(text).color(color));
        }
        ui.add_space(8.0);
        ui.add_enabled(false, egui::Button::new("Quality reviewer"))
            .on_disabled_hover_text(
                "needs a live model and the review side of M9; it would spend money, so it ships off",
            );
    }

    fn save_review(&mut self, changes: &[(ReviewField, String)]) {
        for (field, _) in changes {
            match edit_review(&self.options.profile, *field) {
                Ok(profile) => self.profile = Ok(profile),
                Err(error) => {
                    self.pipeline_notice = Some((false, error.to_string()));
                    return;
                }
            }
        }
        self.pipeline_notice = Some((true, "saved the review settings".into()));
        self.review_edit = None;
    }

    #[cfg(test)]
    pub(crate) fn save_review_for_test(&mut self, changes: &[(ReviewField, String)]) {
        self.save_review(changes);
    }
}
