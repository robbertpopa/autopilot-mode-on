//! The window. Everything it shows comes from `engine`; everything it does is
//! an `engine` action that the command line has too.

mod accounts;
mod app;
mod backend;
mod composer;
pub mod demo;
mod pipeline;
mod run;
mod step;
mod theme;

pub use app::{App, Mode, Screen, Tab};
pub use backend::{Action, Options, Snapshot, now_ms, perform, read};

/// Open the window and run until it is closed.
///
/// # Errors
/// Returns an error if the window cannot be created.
pub fn run(options: Options) -> eframe::Result {
    let mut viewport = eframe::egui::ViewportBuilder::default();
    if let Ok(icon) = eframe::icon_data::from_png_bytes(theme::ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let native = eframe::NativeOptions {
        viewport: viewport
            .with_title("Autopilot Mode On")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([980.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Autopilot Mode On",
        native,
        Box::new(|creation| {
            Ok(Box::new(Window(App::new(
                &creation.egui_ctx,
                options,
                true,
            ))))
        }),
    )
}

struct Window(App);

impl eframe::App for Window {
    fn update(&mut self, context: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        self.0.ui(context);
    }
}

#[cfg(test)]
mod tests {
    use autopilot_core::Step;
    use eframe::egui;

    use super::*;

    fn frame(app: &mut App, context: &egui::Context) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1500.0, 920.0),
            )),
            ..Default::default()
        };
        let _ = context.run(input, |context| app.ui(context));
    }

    // Invariant: every screen builds for every ticket state (running, waiting
    // for the owner, failed) and for an empty window, without panicking.
    #[test]
    fn every_screen_builds_for_every_ticket_state() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        for mode in [Mode::Design, Mode::Watch, Mode::Cost, Mode::Accounts] {
            app.mode = mode;
            frame(&mut app, &context);
        }
        demo::seed(&options.data_dir, now_ms()).unwrap();
        app.set_snapshot(read(&options));
        assert_eq!(app.snapshot.tickets.len(), 3);
        for index in 0..3 {
            let id = app.snapshot.tickets[index].id.clone();
            app.open(&id);
            frame(&mut app, &context);
            for step in Step::ALL {
                app.screen = Screen::Step(step);
                for tab in [
                    Tab::Overview,
                    Tab::Input,
                    Tab::Prompt,
                    Tab::Output,
                    Tab::Log,
                    Tab::Changes,
                    Tab::Cost,
                    Tab::Settings,
                ] {
                    app.tab = tab;
                    frame(&mut app, &context);
                }
            }
        }
        for advanced in [false, true] {
            app.advanced = advanced;
            app.mode = Mode::Design;
            frame(&mut app, &context);
        }
    }

    // Invariant: a click in the window is an engine action. Approving the
    // waiting sample ticket lets Understand pass, and the next read shows it.
    #[test]
    fn approving_a_waiting_ticket_goes_through_the_engine() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let id = autopilot_core::TicketId::from("demo-heartbeat");
        let before = read(&options);
        let waiting = before.tickets.iter().find(|view| view.id == id).unwrap();
        assert_eq!(waiting.badge, autopilot_engine::Badge::NeedsYou);
        let outcome = perform(
            &options,
            &Action::Decide {
                ticket: id.clone(),
                approve: true,
            },
        );
        assert_eq!(outcome, Ok("Approved".to_owned()), "{outcome:?}");
        let after = read(&options);
        let settled = after.tickets.iter().find(|view| view.id == id).unwrap();
        assert_ne!(settled.badge, autopilot_engine::Badge::NeedsYou);
    }

    // Invariant: pausing from the window writes the engine's pause flag, the
    // same one the CLI writes.
    #[test]
    fn pausing_from_the_window_requests_a_pause() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let id = autopilot_core::TicketId::from("demo-timezone");
        assert!(perform(&options, &Action::Pause(id)).is_ok());
        assert!(options.data_dir.join("control").is_dir());
    }

    // Invariant: starting a ticket from the window runs the real engine with
    // the real fixture agent, and the ticket that results is a valid history.
    #[test]
    fn starting_a_ticket_runs_the_real_engine_and_fixture_agent() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let outcome = perform(
            &options,
            &Action::Start {
                title: "Real ticket".into(),
                description: "Show the time in the right zone".into(),
                attachments: Vec::new(),
            },
        );
        assert!(outcome.is_ok(), "{outcome:?}");
        let snapshot = read(&options);
        assert_eq!(snapshot.tickets.len(), 1, "{:?}", snapshot.problem);
        let ticket = &snapshot.tickets[0];
        assert_eq!(ticket.title, "Real ticket");
        let understand = ticket.step(Step::Understand).unwrap();
        assert!(
            matches!(
                understand.state,
                autopilot_pipeline::StepState::Passed | autopilot_pipeline::StepState::Asking(_)
            ),
            "{:?}",
            understand.state
        );
        assert!(understand.in_short.is_some());
    }

    // Invariant: "Try again" from the window runs Understand a second time
    // through the real engine, keeps both attempts in the history, and the
    // ticket ends with Understand settled again. A change that left the step
    // idle or reused the first attempt id would fail the attempt count.
    #[test]
    fn rerunning_understand_from_the_window_adds_a_second_attempt() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        perform(
            &options,
            &Action::Start {
                title: "Again".into(),
                description: "Words kept for the second run".into(),
                attachments: Vec::new(),
            },
        )
        .unwrap();
        let id = read(&options).tickets[0].id.clone();
        // The fixture's Understand has an open question, so Clarify holds the
        // ticket until it is answered, and nothing may run again meanwhile.
        let refused = perform(&options, &Action::RerunUnderstand(id.clone()));
        assert!(
            refused
                .as_ref()
                .is_err_and(|why| why.contains("still active")),
            "{refused:?}"
        );
        perform(&options, &Action::AcceptDefaults { ticket: id.clone() }).unwrap();
        let outcome = perform(&options, &Action::RerunUnderstand(id.clone()));
        assert!(outcome.is_ok(), "{outcome:?}");
        let events = autopilot_engine::watch(&options.data_dir, &id, 0).unwrap();
        let starts = events
            .iter()
            .filter(|record| {
                record.event.step == Some(Step::Understand)
                    && matches!(record.event.kind, autopilot_core::Event::StepStarted { .. })
            })
            .count();
        assert_eq!(starts, 2);
        let ticket = &read(&options).tickets[0];
        assert!(matches!(
            ticket.step(Step::Understand).unwrap().state,
            autopilot_pipeline::StepState::Passed | autopilot_pipeline::StepState::Asking(_)
        ));
    }

    // Invariant: a ticket started with a file lists it on the ticket view, and
    // a refused file stops the start before any ticket exists.
    #[test]
    fn starting_with_a_file_lists_it_and_a_bad_path_starts_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let file = directory.path().join("brief.txt");
        std::fs::write(&file, b"hello").unwrap();
        perform(
            &options,
            &Action::Start {
                title: "With a file".into(),
                description: "See the brief".into(),
                attachments: vec![file.clone()],
            },
        )
        .unwrap();
        let snapshot = read(&options);
        assert_eq!(snapshot.tickets[0].attachments.len(), 1);
        assert_eq!(snapshot.tickets[0].attachments[0].name, "brief.txt");

        let refused = perform(
            &options,
            &Action::Start {
                title: "Bad".into(),
                description: "x".into(),
                attachments: vec![directory.path().join("missing.txt")],
            },
        );
        assert!(refused.is_err());
        assert_eq!(read(&options).tickets.len(), 1);
    }

    // Invariant: the theme choice is kept for the next start.
    #[test]
    fn the_theme_choice_is_remembered() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().to_path_buf(),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let context = egui::Context::default();
        assert!(!App::new(&context, options.clone(), false).light);
        std::fs::write(directory.path().join("ui-theme.txt"), "light").unwrap();
        let mut app = App::new(&context, options, false);
        assert!(app.light);
        frame(&mut app, &context);
    }

    // Invariant: a change asked for in words is shown first and only applied
    // on request; applied to the project it lands in profile.toml, applied to
    // a ticket it lands in that ticket's history.
    #[test]
    fn a_change_request_is_proposed_then_applied_to_the_project_or_a_ticket() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let id = autopilot_core::TicketId::from("demo-timezone");
        app.open(&id);
        app.change_text = "use opus for the plan and 2 rounds".into();
        let reading =
            autopilot_engine::propose(&app.change_text, app.profile.as_ref().unwrap()).unwrap();
        assert_eq!(reading.proposals.len(), 2);
        assert!(!options.profile.exists(), "proposing must not write");
        // Per ticket: only model, effort and ask-after fit.
        let ticket_only = &reading.proposals[..1];
        autopilot_engine::apply_to_ticket(&options.data_dir, &id, ticket_only).unwrap();
        let effective = autopilot_engine::effective_overrides(
            &options.data_dir,
            &id,
            autopilot_core::Step::Plan,
        )
        .unwrap();
        assert_eq!(effective.model.as_deref(), Some("opus-5-5"));
        // For the project: written to the file the window reads.
        let profile =
            autopilot_engine::apply_to_project(&options.profile, &reading.proposals).unwrap();
        assert_eq!(
            profile
                .step(autopilot_core::Step::BuildCheck)
                .unwrap()
                .limits
                .rounds,
            2
        );
        app.reading = Some(Ok(reading));
        app.for_project = true;
        frame(&mut app, &context);
    }

    // Invariant: a newer release shows the update button, a failed install
    // brings it back with the reason instead of closing the window, and a
    // failed check shows nothing.
    #[test]
    fn an_update_is_offered_and_a_failed_install_says_why() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let context = egui::Context::default();
        let mut app = App::new(&context, options, false);
        app.sender
            .send(backend::Message::Update(Err("offline".into())))
            .unwrap();
        frame(&mut app, &context);
        assert!(app.update_offered().is_none());

        let update = autopilot_update::Update {
            version: "9.9.9".into(),
            notes: String::new(),
            asset: autopilot_update::Asset {
                url: "https://example.test/x".into(),
                sha256: "0".repeat(64),
            },
        };
        app.sender
            .send(backend::Message::Update(Ok(Some(update))))
            .unwrap();
        frame(&mut app, &context);
        assert_eq!(app.update_offered(), Some("9.9.9"));

        app.sender
            .send(backend::Message::Installing(Err("checksum".into())))
            .unwrap();
        frame(&mut app, &context);
        assert_eq!(app.update_offered(), Some("9.9.9"));
        assert!(!app.update_installing());
    }
}
