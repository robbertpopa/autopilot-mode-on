//! Pipeline health, the read-only part: how many runs a pipeline had, what each
//! step used and how much of the context window it reached. Every figure comes
//! from what an adapter reported; where it reported nothing the page says so
//! and never shows a zero. Grades, findings and tuning need the reviewer
//! (R11) and the tuning overlay (R12), which are not built.

use autopilot_engine::{PipelineMetrics, StepMetrics, dollars, step_name};
use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;

/// Runs counted per pipeline. A starting default, not a measured value.
const LAST_RUNS: usize = 5;

/// The share a peak context may reach before a step is worth a look, in
/// thousandths. The reviewer will own this number; the page only draws it.
const WATCH_PERMILLE: u32 = 800;

fn percent(permille: u32) -> String {
    format!("{}.{}%", permille / 10, permille % 10)
}

fn tokens(row: &StepMetrics) -> String {
    if row.reported == 0 {
        "not reported".to_owned()
    } else {
        format!("{} in · {} out", row.input_tokens, row.output_tokens)
    }
}

fn peak(row: &StepMetrics) -> String {
    row.peak_context_permille
        .map_or_else(|| "not reported".to_owned(), percent)
}

impl App {
    pub(crate) fn health_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Pipeline health").size(30.0).strong());
        ui.label(theme::muted(format!(
            "last {LAST_RUNS} runs of each pipeline. Reported by the agent; nothing is estimated."
        )));
        let Ok(profile) = &self.profile else {
            ui.label(theme::muted("The profile could not be read."));
            return;
        };
        let names: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
        if self.health_selected.is_none() {
            self.health_selected = names.first().cloned();
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for name in &names {
                let chosen = self.health_selected.as_deref() == Some(name.as_str());
                if ui.selectable_label(chosen, name).clicked() {
                    self.health_selected = Some(name.clone());
                }
            }
        });
        let Some(name) = self.health_selected.clone() else {
            return;
        };
        let metrics = self.health_metrics(&name);
        ui.add_space(10.0);
        let Some(metrics) = metrics else {
            ui.label(theme::muted("The runs could not be read."));
            return;
        };
        if metrics.tickets == 0 {
            ui.label(theme::muted(format!("{name} has no runs yet.")));
        } else {
            Self::health_tiles(ui, &metrics);
            ui.add_space(10.0);
            health_table(ui, &metrics);
        }
        ui.add_space(14.0);
        ui.label(theme::muted(
            "Grades, findings and automatic tuning are not built: they need the reviewer and the tuning overlay.",
        ));
    }

    /// The metrics of one pipeline, kept until a ticket changes.
    fn health_metrics(&mut self, name: &str) -> Option<PipelineMetrics> {
        let stamp: u64 = self
            .snapshot
            .tickets
            .iter()
            .map(|view| view.last_seq)
            .sum::<u64>()
            + self.snapshot.tickets.len() as u64;
        let fresh = self
            .health_cache
            .as_ref()
            .is_some_and(|(cached, at, _)| cached == name && *at == stamp);
        if !fresh {
            let loaded =
                autopilot_engine::pipeline_metrics(&self.options.data_dir, name, LAST_RUNS).ok();
            self.health_cache = loaded.map(|metrics| (name.to_owned(), stamp, metrics));
        }
        self.health_cache
            .as_ref()
            .map(|(_, _, metrics)| metrics.clone())
    }

    fn health_tiles(ui: &mut egui::Ui, metrics: &PipelineMetrics) {
        let worst = metrics
            .steps
            .iter()
            .filter_map(|row| row.peak_context_permille)
            .max();
        let api = metrics
            .steps
            .iter()
            .fold(autopilot_core::Money::default(), |total, row| {
                total.checked_add(row.api).unwrap_or(total)
            });
        let subscription = metrics
            .steps
            .iter()
            .fold(autopilot_core::Money::default(), |total, row| {
                total.checked_add(row.subscription).unwrap_or(total)
            });
        ui.horizontal(|ui| {
            tile(ui, "runs", &metrics.tickets.to_string());
            tile(
                ui,
                "peak context",
                &worst.map_or_else(|| "not reported".into(), percent),
            );
            let per = |money: autopilot_core::Money| {
                autopilot_core::Money::from_microusd(
                    money.as_microusd() / u64::try_from(metrics.tickets).unwrap_or(1).max(1),
                )
            };
            tile(
                ui,
                "cost per run",
                &format!(
                    "{} API · ~{} est.",
                    dollars(per(api)),
                    dollars(per(subscription))
                ),
            );
        });
    }
}

fn tile(ui: &mut egui::Ui, label: &str, value: &str) {
    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
        ui.set_width(150.0);
        ui.label(theme::muted(label));
        ui.label(RichText::new(value).size(18.0).strong());
    });
}

fn health_table(ui: &mut egui::Ui, metrics: &PipelineMetrics) {
    ui.label(RichText::new("Tokens by step").size(18.0).strong());
    for row in metrics.steps.iter().filter(|row| row.runs > 0) {
        ui.horizontal(|ui| {
            ui.add_sized([130.0, 20.0], egui::Label::new(step_name(row.step)));
            ui.add_sized([170.0, 20.0], egui::Label::new(tokens(row)));
            let watch = row
                .peak_context_permille
                .is_some_and(|share| share >= WATCH_PERMILLE);
            let color = if watch {
                theme::AMBER()
            } else {
                theme::TEXT_SOFT()
            };
            ui.add_sized(
                [150.0, 20.0],
                egui::Label::new(RichText::new(format!("peak {}", peak(row))).color(color)),
            );
            let mut cost = format!("{} API", dollars(row.api));
            if row.subscription.as_microusd() > 0 {
                cost = format!("{cost} · ~{} est.", dollars(row.subscription));
            }
            ui.label(theme::muted(cost));
            if row.rounds > 0 {
                ui.label(theme::muted(format!(
                    "{} rounds · first passed {} of {}",
                    row.rounds, row.first_round_passed, row.runs
                )));
            }
            if row.reported < row.runs {
                ui.label(theme::muted(format!(
                    "{} of {} runs reported",
                    row.reported, row.runs
                )));
            }
        });
    }
}
