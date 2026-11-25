//! The spend limits: the most all paid calls may cost together, and the most
//! one call may cost. They are written to `profile.toml` only from here.

use autopilot_core::Money;
use autopilot_engine::{SpendField, dollars, edit_spend, micro_from_dollars};
use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;

/// What the owner is typing, in dollars. `None` shows the saved value.
#[derive(Default)]
pub(crate) struct SpendEdit {
    total: Option<String>,
    call: Option<String>,
    daily: Option<String>,
    raise: Option<String>,
    problem: Option<String>,
}

fn plain(micro: u64) -> String {
    format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

impl App {
    fn saved_limits(&self) -> autopilot_profile::SpendLimits {
        self.profile
            .as_ref()
            .map(|profile| profile.spend)
            .unwrap_or_default()
    }

    /// Write one limit and show the profile as saved. A bad value is said in
    /// words and nothing is written.
    pub(crate) fn save_spend(&mut self, field: SpendField, text: &str) -> bool {
        let Some(micro) = micro_from_dollars(text) else {
            self.spend_edit.problem = Some(format!("\"{text}\" is not an amount in dollars"));
            return false;
        };
        let field = match field {
            SpendField::Total(_) => SpendField::Total(micro),
            SpendField::PerCall(_) => SpendField::PerCall(micro),
            SpendField::Daily(_) | SpendField::NoDaily => SpendField::Daily(micro),
        };
        match edit_spend(&self.options.profile, field) {
            Ok(profile) => {
                self.profile = Ok(profile);
                self.spend_edit.problem = None;
                true
            }
            Err(error) => {
                self.spend_edit.problem = Some(error.to_string());
                false
            }
        }
    }

    /// An empty box removes the daily limit; anything else must be dollars.
    fn save_daily(&mut self, text: &str) -> bool {
        if !text.trim().is_empty() {
            return self.save_spend(SpendField::Daily(0), text.trim());
        }
        match edit_spend(&self.options.profile, SpendField::NoDaily) {
            Ok(profile) => {
                self.profile = Ok(profile);
                true
            }
            Err(error) => {
                self.spend_edit.problem = Some(error.to_string());
                false
            }
        }
    }

    /// Settings, Cost: the limits and what has been spent against the total.
    pub(crate) fn spend_card(&mut self, ui: &mut egui::Ui) {
        let limits = self.saved_limits();
        let spent = autopilot_engine::spent_since(&self.options.data_dir, 0)
            .map(|(api, _)| api)
            .unwrap_or_default();
        let mut total = self
            .spend_edit
            .total
            .clone()
            .unwrap_or_else(|| plain(limits.total_microusd.get()));
        let mut call = self
            .spend_edit
            .call
            .clone()
            .unwrap_or_else(|| plain(limits.call_microusd.get()));
        let saved_daily = self
            .profile
            .as_ref()
            .ok()
            .and_then(|profile| profile.budgets.daily_microusd);
        let mut daily = self
            .spend_edit
            .daily
            .clone()
            .unwrap_or_else(|| saved_daily.map_or_else(String::new, |n| plain(n.get())));
        let mut save = false;
        theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Spend limits").strong());
            ui.label(theme::muted(format!(
                "{} spent of {} allowed in all. A paid call is refused once the total is spent.",
                dollars(spent),
                dollars(Money::from_microusd(limits.total_microusd.get())),
            )));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Total, dollars");
                ui.add(egui::TextEdit::singleline(&mut total).desired_width(80.0));
                ui.add_space(12.0);
                ui.label("Most for one call, dollars");
                ui.add(egui::TextEdit::singleline(&mut call).desired_width(80.0));
                save = ui.add(theme::primary("Save")).clicked();
            });
            ui.horizontal(|ui| {
                ui.label("Most in one day, dollars");
                ui.add(
                    egui::TextEdit::singleline(&mut daily)
                        .desired_width(80.0)
                        .hint_text("no limit"),
                );
            });
            ui.label(theme::muted(
                "A step that sets its own budget keeps it. Only this page changes these limits.                  A day counts from midnight UTC; leave the daily box empty for no daily limit.",
            ));
            if let Some(problem) = &self.spend_edit.problem {
                ui.label(RichText::new(problem).color(theme::RED()));
            }
        });
        if save {
            let a = self.save_spend(SpendField::Total(0), &total);
            let b = self.save_spend(SpendField::PerCall(0), &call);
            let c = self.save_daily(&daily);
            if a && b && c {
                self.spend_edit.total = None;
                self.spend_edit.call = None;
                self.spend_edit.daily = None;
            }
        } else {
            self.spend_edit.total = Some(total);
            self.spend_edit.call = Some(call);
            self.spend_edit.daily = Some(daily);
        }
    }

    /// Under the notice of a stop on the total cap: raise it and run the
    /// stopped action again.
    pub(crate) fn cap_retry_row(&mut self, ui: &mut egui::Ui) {
        if self.cap_retry.is_none() {
            return;
        }
        let mut text = self
            .spend_edit
            .raise
            .clone()
            .unwrap_or_else(|| plain(self.saved_limits().total_microusd.get()));
        let mut go = false;
        ui.horizontal(|ui| {
            ui.label("New total, dollars");
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0));
            go = ui.add(theme::primary("Raise the cap and retry")).clicked();
        });
        if let Some(problem) = &self.spend_edit.problem {
            ui.label(RichText::new(problem).color(theme::RED()));
        }
        self.spend_edit.raise = Some(text.clone());
        if go {
            self.raise_and_retry(&text);
        }
    }

    /// Save the new total, then run the stopped action again. A value that
    /// cannot be saved leaves the stop in place.
    pub(crate) fn raise_and_retry(&mut self, dollars: &str) {
        if self.save_spend(SpendField::Total(0), dollars)
            && let Some(action) = self.cap_retry.take()
        {
            self.spend_edit.raise = None;
            self.act(action);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use autopilot_core::{Event, Money, NewEvent, Step, TicketId};
    use autopilot_store::{SqliteStore, Store};
    use eframe::egui;

    use super::plain;
    use crate::app::App;
    use crate::backend::{Action, Message, Options};

    fn window(directory: &std::path::Path) -> (App, Options) {
        let options = Options {
            data_dir: directory.join("data"),
            profile: directory.join("profile.toml"),
            view: None,
            ticket: None,
        };
        crate::backend::AgentChoice::Claude.save(&options.data_dir);
        std::fs::write(
            &options.profile,
            "[steps.understand]
auth = \"api\"
[spend]
total_microusd = 100000
",
        )
        .unwrap();
        crate::backend::mark_welcomed(&options.data_dir);
        let app = App::new(&egui::Context::default(), options.clone(), false);
        (app, options)
    }

    fn spent(options: &Options, micro: u64) {
        std::fs::create_dir_all(&options.data_dir).unwrap();
        let mut store = SqliteStore::open(options.data_dir.join("events.sqlite3")).unwrap();
        let events = [
            NewEvent {
                at_ms: 1,
                step: None,
                kind: Event::TicketCreated { title: "t".into() },
            },
            NewEvent {
                at_ms: 2,
                step: Some(Step::Understand),
                kind: Event::CostTick {
                    amount: Money::from_microusd(micro),
                    estimated: false,
                },
            },
        ];
        store.append(&TicketId::from("t"), 0, &events).unwrap();
    }

    // Invariant: an API step's call is refused once the total is spent, and
    // before then its per-call cap is cut to what is left. Breaks if the gate
    // is dropped from `claude_config`, the one place every real call's
    // settings come from.
    #[test]
    fn a_paid_step_is_cut_to_what_is_left_and_refused_at_zero() {
        let directory = tempfile::tempdir().unwrap();
        let (_, options) = window(directory.path());
        spent(&options, 70_000);
        let config = crate::backend::claude_config(&options, Step::Understand, None).unwrap();
        assert_eq!(config.max_budget_usd.as_deref(), Some("0.030000"));
        let directory = tempfile::tempdir().unwrap();
        let (_, options) = window(directory.path());
        spent(&options, 100_000);
        let refused = crate::backend::claude_config(&options, Step::Understand, None).unwrap_err();
        assert!(autopilot_engine::is_cap_refusal(&refused), "{refused}");
    }

    // Invariant: only the cap's own stop offers "raise and retry", and raising
    // it runs the stopped action again. Breaks if any failure keeps the
    // action, or the retry runs without saving the new total.
    #[test]
    fn a_stop_on_the_cap_can_be_raised_and_retried() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, options) = window(directory.path());
        let action = || Action::Attach {
            ticket: TicketId::from("t"),
            paths: Vec::new(),
        };
        app.act(action());
        app.sender
            .send(Message::Done(Some(1), Err("something else broke".into())))
            .unwrap();
        app.drain();
        assert!(app.cap_retry.is_none());

        app.act(action());
        let words = autopilot_engine::CAP_REACHED.to_owned() + ": $0.10 spent of $0.10 allowed";
        app.sender.send(Message::Done(Some(2), Err(words))).unwrap();
        app.drain();
        assert!(app.cap_retry.is_some());

        app.raise_and_retry("nonsense");
        assert!(app.cap_retry.is_some(), "a bad amount keeps the stop");
        app.raise_and_retry("2.5");
        assert!(app.cap_retry.is_none());
        let saved = autopilot_engine::load_profile(&options.profile).unwrap();
        assert_eq!(saved.spend.total_microusd.get(), 2_500_000);
        assert_eq!(
            app.profile.as_ref().unwrap().spend.total_microusd.get(),
            2_500_000
        );
    }

    // Invariant: the stop's retry row is drawn, so the owner can find it.
    #[test]
    fn the_retry_row_is_drawn_after_a_stop_on_the_cap() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = window(directory.path());
        app.cap_retry = Some(Action::Attach {
            ticket: TicketId::from("t"),
            paths: Vec::new(),
        });
        app.notice = Some((false, autopilot_engine::CAP_REACHED.to_owned()));
        let context = egui::Context::default();
        let output = context.run(egui::RawInput::default(), |context| app.ui(context));
        let drawn: String = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(drawn.contains("Raise the cap and retry"), "{drawn}");
    }

    // Invariant: the daily limit saved in Settings stops a paid step once today's
    // spend reaches it, with words that do not offer to raise the total, and an
    // empty daily box removes it. Breaks if `claude_config` ignores the daily
    // limit or the card writes it anywhere but `[budgets]`.
    #[test]
    fn the_daily_limit_from_settings_stops_a_paid_step_today() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, options) = window(directory.path());
        assert!(app.save_daily("0.04"));
        std::fs::create_dir_all(&options.data_dir).unwrap();
        let mut store = SqliteStore::open(options.data_dir.join("events.sqlite3")).unwrap();
        let at_ms = autopilot_engine::start_of_today_ms() + 1;
        let events = [
            NewEvent {
                at_ms,
                step: None,
                kind: Event::TicketCreated { title: "t".into() },
            },
            NewEvent {
                at_ms,
                step: Some(Step::Understand),
                kind: Event::CostTick {
                    amount: Money::from_microusd(40_000),
                    estimated: false,
                },
            },
        ];
        store.append(&TicketId::from("t"), 0, &events).unwrap();
        let refused = crate::backend::claude_config(&options, Step::Understand, None).unwrap_err();
        assert!(
            refused.starts_with(autopilot_engine::DAY_REACHED),
            "{refused}"
        );
        assert!(!autopilot_engine::is_cap_refusal(&refused));

        assert!(app.save_daily(""));
        assert!(crate::backend::claude_config(&options, Step::Understand, None).is_ok());
        assert!(!app.save_daily("not money"));
    }

    #[test]
    fn a_limit_is_written_without_trailing_zeros() {
        assert_eq!(plain(1_000_000), "1");
        assert_eq!(plain(50_000), "0.05");
        assert_eq!(plain(1_234_567), "1.234567");
    }
}
