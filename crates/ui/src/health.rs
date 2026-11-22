//! Pipeline health, the read-only part: how many runs a pipeline had, what each
//! step used and how much of the context window it reached. Every figure comes
//! from what an adapter reported; where it reported nothing the page says so
//! and never shows a zero. Findings come from the token review and tunings from
//! the overlay; grades need the model-graded reviewer, which is not built.

use autopilot_engine::{PipelineMetrics, StepMetrics, dollars, step_name};
use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;

/// Runs counted per pipeline. A starting default, not a measured value.
const LAST_RUNS: usize = 5;

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
            // The same line the review uses, so the page and the findings agree.
            let watch = self
                .profile
                .as_ref()
                .map_or(800, |profile| profile.review.peak_context_permille);
            health_table(ui, &metrics, watch);
            self.health_findings(ui, &name);
            self.health_tunings(ui, &name);
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

    /// Open findings of this pipeline, with the numbers behind each.
    /// What the review changed in this pipeline, each with its Undo.
    fn health_tunings(&mut self, ui: &mut egui::Ui, name: &str) {
        let mine: Vec<autopilot_engine::ActiveTuning> = self
            .tunings
            .iter()
            .filter(|each| each.tuning.pipeline == name)
            .cloned()
            .collect();
        if mine.is_empty() {
            return;
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Tuned").size(18.0).strong());
        for each in mine {
            let tuning = &each.tuning;
            theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} · effort {} to {}",
                        autopilot_engine::step_name(tuning.step),
                        tuning.effort_before.as_deref().unwrap_or("unset"),
                        tuning.effort_after
                    ))
                    .strong(),
                );
                ui.label(&tuning.why);
                ui.label(theme::muted(if each.guarded {
                    "Watching the next 3 runs; it reverts itself if they do worse."
                } else {
                    "The next 3 runs did no worse; it stays."
                }));
                if ui.button("Undo").clicked() {
                    self.notice = Some(
                        match autopilot_engine::undo_tuning(
                            &self.options.data_dir,
                            &tuning.pipeline,
                            tuning.step,
                        ) {
                            Ok(_) => {
                                self.tunings =
                                    autopilot_engine::active_tunings(&self.options.data_dir)
                                        .unwrap_or_default();
                                (true, "The tuning is undone.".to_owned())
                            }
                            Err(error) => (false, format!("Could not undo: {error}")),
                        },
                    );
                }
            });
        }
    }

    fn health_findings(&mut self, ui: &mut egui::Ui, name: &str) {
        let mine: Vec<&autopilot_engine::Finding> = self
            .findings
            .iter()
            .filter(|finding| finding.pipeline == name)
            .collect();
        if mine.is_empty() {
            return;
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Findings").size(18.0).strong());
        let mine: Vec<autopilot_engine::Finding> = mine.into_iter().cloned().collect();
        let mut approve = None;
        let mut dismiss = None;
        for finding in &mine {
            let suggestion = self
                .suggestions
                .iter()
                .find(|each| each.finding == *finding)
                .cloned();
            theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        autopilot_engine::step_name(finding.step),
                        finding.rule.label()
                    ))
                    .strong()
                    .color(theme::AMBER()),
                );
                ui.label(&finding.evidence);
                if let Some(suggestion) = suggestion {
                    ui.label(RichText::new("Proposed, needs you").color(theme::AMBER()));
                    ui.label(format!(
                        "{} (all pipelines, written to profile.toml)",
                        suggestion.proposal.describe()
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Approve").clicked() {
                            approve = Some(suggestion.proposal.clone());
                        }
                        if ui.button("Dismiss").clicked() {
                            dismiss = Some(suggestion.finding.clone());
                        }
                    });
                }
            });
        }
        if let Some(proposal) = approve {
            self.notice = Some(
                match autopilot_engine::apply_to_project(
                    &self.options.profile,
                    std::slice::from_ref(&proposal),
                ) {
                    Ok(profile) => {
                        self.profile = Ok(profile);
                        self.refresh_suggestions();
                        (true, format!("Applied: {}", proposal.describe()))
                    }
                    Err(error) => (false, format!("Could not apply: {error}")),
                },
            );
        }
        if let Some(finding) = dismiss {
            self.notice = Some(
                match autopilot_engine::dismiss_proposal(&self.options.data_dir, &finding) {
                    Ok(()) => {
                        self.refresh_suggestions();
                        (true, "Dismissed.".to_owned())
                    }
                    Err(error) => (false, format!("Could not dismiss: {error}")),
                },
            );
        }
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

fn health_table(ui: &mut egui::Ui, metrics: &PipelineMetrics, watch_permille: u32) {
    ui.label(RichText::new("Tokens by step").size(18.0).strong());
    step_rows(ui, metrics, watch_permille);
}

/// What one finished ticket used, step by step, from its own recorded usage.
/// There is no grade: that needs the model-graded reviewer, which is not built.
pub(crate) fn review_card(
    ui: &mut egui::Ui,
    view: &autopilot_engine::TicketView,
    watch_permille: u32,
) {
    let name = view
        .pipeline
        .as_ref()
        .map_or(autopilot_profile::DEFAULT_PIPELINE, |(name, _)| {
            name.as_str()
        });
    let metrics = autopilot_engine::metrics_of(name, &[view]);
    if metrics.steps.iter().all(|row| row.runs == 0) {
        return;
    }
    theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
        ui.label(RichText::new("Review").strong());
        ui.label(theme::muted(format!(
            "pipeline {name}. Reported by the agent; amber is at or above the context line."
        )));
        step_rows(ui, &metrics, watch_permille);
    });
}

fn step_rows(ui: &mut egui::Ui, metrics: &PipelineMetrics, watch_permille: u32) {
    for row in metrics.steps.iter().filter(|row| row.runs > 0) {
        ui.horizontal(|ui| {
            ui.add_sized([130.0, 20.0], egui::Label::new(step_name(row.step)));
            ui.add_sized([170.0, 20.0], egui::Label::new(tokens(row)));
            let watch = row
                .peak_context_permille
                .is_some_and(|share| share >= watch_permille);
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
