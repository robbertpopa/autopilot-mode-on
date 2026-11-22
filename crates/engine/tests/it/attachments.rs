use std::fs;

use autopilot_core::{AttachmentKind, Event, Ticket, TicketId};
use autopilot_engine::{
    MAX_FILE_BYTES, attach, run_understand, run_understand_attached, stage, watch,
};

use super::Synthetic;

fn ticket(id: &str) -> Ticket {
    Ticket {
        id: TicketId::from(id),
        title: "With files".into(),
        description: "Look at the files".into(),
    }
}

fn attached(data: &std::path::Path, id: &TicketId) -> Vec<Event> {
    watch(data, id, 0)
        .unwrap()
        .into_iter()
        .map(|record| record.event.kind)
        .filter(|kind| matches!(kind, Event::Attached { .. }))
        .collect()
}

// Invariant: a copied file keeps its bytes and carries their SHA-256, an image
// is told apart by extension, and a folder is recorded but never copied. A
// change that dropped the hash or copied the folder would fail here.
#[test]
fn files_images_and_folders_are_recorded_with_a_hash_and_folders_are_not_copied() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("notes.txt");
    fs::write(&file, b"abc").unwrap();
    let image = source.path().join("shot.PNG");
    fs::write(&image, b"png").unwrap();
    let folder = source.path().join("project");
    fs::create_dir_all(folder.join("src")).unwrap();
    fs::write(folder.join("src").join("a.rs"), b"fn main() {}").unwrap();

    let staged = stage(
        data.path(),
        &TicketId::from("t-1"),
        &[file.clone(), image, folder.clone()],
        (0, 0),
    )
    .unwrap();
    assert_eq!(staged.len(), 3);
    assert_eq!(staged[0].kind, AttachmentKind::File);
    assert_eq!(
        staged[0].sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(staged[1].kind, AttachmentKind::Image);
    assert_eq!(staged[2].kind, AttachmentKind::Folder);
    assert_eq!(staged[2].items, 2);
    assert_eq!(staged[2].sha256, "");
    assert_eq!(
        fs::read(data.path().join(&staged[0].stored)).unwrap(),
        b"abc"
    );
    // Only the two files were copied.
    let copies = fs::read_dir(data.path().join("attachments").join("t-1"))
        .unwrap()
        .count();
    assert_eq!(copies, 2);
}

// Invariant: a refused source leaves nothing on disk, even when an earlier
// source was fine, and no event is written. A partial copy would be left over.
#[test]
fn a_refused_source_writes_nothing() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let good = source.path().join("good.txt");
    fs::write(&good, b"ok").unwrap();
    let big = source.path().join("big.bin");
    let handle = fs::File::create(&big).unwrap();
    handle.set_len(MAX_FILE_BYTES + 1).unwrap();
    let relative = std::path::PathBuf::from("relative.txt");

    for bad in [big, relative, source.path().join("missing")] {
        let result = stage(
            data.path(),
            &TicketId::from("t-2"),
            &[good.clone(), bad],
            (0, 0),
        );
        assert!(result.is_err());
        assert!(!data.path().join("attachments").join("t-2").exists());
    }
    let result = stage(data.path(), &TicketId::from("../evil"), &[good], (0, 0));
    assert!(result.is_err());
}

#[cfg(unix)]
#[test]
fn a_link_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let target = source.path().join("real.txt");
    fs::write(&target, b"x").unwrap();
    let link = source.path().join("link.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(stage(data.path(), &TicketId::from("t-3"), &[link], (0, 0)).is_err());
}

// Invariant: attachments given at the start are recorded in the opening
// batch, before the first step starts, so the first attempt can see them.
#[test]
fn attachments_given_at_the_start_come_before_the_first_step() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("brief.md");
    fs::write(&file, b"# brief").unwrap();
    let ticket = ticket("t-4");
    let staged = stage(data.path(), &ticket.id, &[file], (0, 0)).unwrap();
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    run_understand_attached(data.path(), &ticket, &agent, None, &staged).unwrap();
    let kinds: Vec<_> = watch(data.path(), &ticket.id, 0)
        .unwrap()
        .into_iter()
        .map(|record| record.event.kind)
        .collect();
    let attached_at = kinds
        .iter()
        .position(|kind| matches!(kind, Event::Attached { .. }))
        .unwrap();
    let started_at = kinds
        .iter()
        .position(|kind| matches!(kind, Event::StepStarted { .. }))
        .unwrap();
    assert!(attached_at < started_at);
}

// Invariant: attaching later appends events and counts what is already there
// toward the per-ticket limit.
#[test]
fn attaching_later_appends_and_respects_the_count_limit() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let ticket = ticket("t-5");
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    run_understand(data.path(), &ticket, &agent).unwrap();
    let file = source.path().join("late.txt");
    fs::write(&file, b"late").unwrap();
    assert_eq!(
        attach(data.path(), &ticket.id, std::slice::from_ref(&file)).unwrap(),
        1
    );
    assert_eq!(attached(data.path(), &ticket.id).len(), 1);
    let many = vec![file; autopilot_engine::MAX_ATTACHMENTS];
    assert!(attach(data.path(), &ticket.id, &many).is_err());
    assert_eq!(attached(data.path(), &ticket.id).len(), 1);
}

// Invariant: the agent's note quotes small text files, only names images and
// folders, skips binary files, and cuts long text. A change that quoted a
// folder listing or an image's bytes would fail here.
#[test]
fn the_note_quotes_text_and_only_names_the_rest() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let text = source.path().join("spec.md");
    fs::write(&text, "key is zebra-todos-v7").unwrap();
    let binary = source.path().join("blob.bin");
    fs::write(&binary, [0xff_u8, 0xfe, 0x00, 0x01]).unwrap();
    let image = source.path().join("shot.png");
    fs::write(&image, b"not really a png but named like one").unwrap();
    let folder = source.path().join("proj");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("secret.txt"), "do not quote me").unwrap();
    let long = source.path().join("long.txt");
    fs::write(&long, "a".repeat(40_000)).unwrap();

    let ticket = ticket("t-6");
    let staged = stage(
        data.path(),
        &ticket.id,
        &[text, binary, image, folder, long],
        (0, 0),
    )
    .unwrap();
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    run_understand_attached(data.path(), &ticket, &agent, None, &staged).unwrap();
    let history = watch(data.path(), &ticket.id, 0).unwrap();
    let note = autopilot_engine::note(data.path(), &history);
    assert!(note.contains("zebra-todos-v7"));
    assert!(note.contains("- image shot.png"));
    assert!(note.contains("- folder proj: 1 items"));
    assert!(!note.contains("do not quote me"));
    assert!(!note.contains("not really a png"));
    // The binary file is named but has no quoted body, and the long one is cut.
    assert!(note.contains("- file blob.bin"));
    assert!(note.len() < 20_000, "{}", note.len());
}

// Invariant: the project folder is kept in the ticket's history and shows in
// the view, a path that is not a folder is refused, and recording the same
// folder twice adds nothing.
#[test]
fn the_project_folder_is_recorded_once_and_a_file_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let ticket = ticket("ws-1");
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    run_understand(data.path(), &ticket, &agent).unwrap();
    let first =
        autopilot_engine::record_workspace(data.path(), &ticket.id, project.path()).unwrap();
    let again =
        autopilot_engine::record_workspace(data.path(), &ticket.id, project.path()).unwrap();
    assert_eq!(first, again);
    let history = watch(data.path(), &ticket.id, 0).unwrap();
    let recorded = history
        .iter()
        .filter(|record| matches!(record.event.kind, Event::Workspace { .. }))
        .count();
    assert_eq!(recorded, 1);
    assert_eq!(
        autopilot_engine::workspace_of(&history),
        Some(first.clone())
    );
    let view = autopilot_engine::ticket_view(&history).unwrap();
    assert_eq!(view.workspace, Some(first));

    let file = project.path().join("a.txt");
    fs::write(&file, b"x").unwrap();
    assert!(autopilot_engine::record_workspace(data.path(), &ticket.id, &file).is_err());
}

const TRIAGE_ANSWER: &str = r#"{"complexity":"high","reasoning":"Touches storage.",
    "steps":[{"step":"design","model":"sonnet","effort":"medium","why":"Two screens."},
             {"step":"plan","model":"opus","effort":"high","why":"Risky migration."}]}"#;

// Invariant: the orchestrator's choice is kept once, shows in the view, and
// becomes ordinary ticket settings the next start of each step reads; asking
// again costs nothing; a refused answer leaves no decision and no settings.
#[test]
fn the_orchestrator_sets_later_steps_once_and_a_bad_answer_changes_nothing() {
    use autopilot_core::Step;
    use autopilot_engine::{effective_overrides, triage_ticket};

    let data = tempfile::tempdir().unwrap();
    let agent = Synthetic {
        data_dir: data.path().into(),
    };
    let ticket = ticket("tri-1");
    run_understand(data.path(), &ticket, &agent).unwrap();
    let made = triage_ticket(data.path(), &ticket.id, false, |prompt| {
        assert!(prompt.contains("orchestrator"));
        Ok(TRIAGE_ANSWER.to_owned())
    })
    .unwrap();
    assert_eq!(made.steps.len(), 2);
    let plan = effective_overrides(data.path(), &ticket.id, Step::Plan).unwrap();
    assert_eq!(plan.model.as_deref(), Some("opus"));
    assert_eq!(plan.effort.as_deref(), Some("high"));
    let design = effective_overrides(data.path(), &ticket.id, Step::Design).unwrap();
    assert_eq!(design.effort.as_deref(), Some("medium"));
    let again = triage_ticket(data.path(), &ticket.id, false, |_| {
        panic!("a kept decision must not ask again")
    })
    .unwrap();
    assert_eq!(again, made);
    let history = watch(data.path(), &ticket.id, 0).unwrap();
    assert_eq!(
        autopilot_engine::ticket_view(&history).unwrap().triage,
        Some(made)
    );

    let other = self::ticket("tri-2");
    run_understand(data.path(), &other, &agent).unwrap();
    let refused = triage_ticket(data.path(), &other.id, false, |_| {
        Ok(TRIAGE_ANSWER.replace("opus", "gpt"))
    });
    assert!(refused.is_err());
    let history = watch(data.path(), &other.id, 0).unwrap();
    assert!(autopilot_engine::triage_kept(&history).is_none());
    let plan = effective_overrides(data.path(), &other.id, Step::Plan).unwrap();
    assert_eq!(plan.model, None);
}
