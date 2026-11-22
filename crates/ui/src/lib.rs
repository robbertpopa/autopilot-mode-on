//! The window. Everything it shows comes from `engine`; everything it does is
//! an `engine` action that the command line has too.

mod accounts;
mod app;
mod backend;
mod board;
mod composer;
pub mod demo;
mod first_run;
mod nav;
mod new_ticket;
mod pipeline;
mod pipelines;
mod plan_review;
mod projects;
mod run;
mod settings;
mod step;
mod theme;
mod ticket;
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
        backend::AgentChoice::Practice.save(&options.data_dir);
        let context = egui::Context::default();
        let mut app = App::new(&context, options.clone(), false);
        for mode in [
            Mode::Today,
            Mode::Board,
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
        ] {
            app.mode = mode;
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
        backend::save_workspace(&options.data_dir, Some(directory.path()));
        assert!(!app.is_first_run());
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
        let draft = backend::draft_pipeline(&options, &app.draft_text).unwrap();
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
}
