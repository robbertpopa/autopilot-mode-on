use std::fs;

use std::time::Duration;

use super::{
    Error, Ownership, TicketId, read_ownership, registered_checkout, root_lock_within,
    ticket_worktree, write_ownership,
};

#[test]
fn a_git_administrative_entry_must_link_back_to_the_same_checkout() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let administrative = root.join("admin");
    let checkout = root.join("checkout");
    fs::create_dir(&administrative).unwrap();
    fs::create_dir(&checkout).unwrap();
    fs::write(checkout.join(".git"), b"fixture").unwrap();
    fs::write(root.join("different-gitfile"), b"fixture").unwrap();

    let link = administrative.join("gitdir");
    fs::write(&link, b"../checkout/.git\n").unwrap();
    assert!(registered_checkout(&administrative, &checkout).unwrap());
    fs::write(&link, b"../different-gitfile\n").unwrap();
    assert!(!registered_checkout(&administrative, &checkout).unwrap());
}

#[test]
fn ticket_paths_distinguish_case_and_reject_nonportable_branch_names() {
    let (_, upper) = ticket_worktree(&TicketId::from("Sample-42")).unwrap();
    let (_, lower) = ticket_worktree(&TicketId::from("sample-42")).unwrap();
    assert_ne!(upper, lower);
    for value in ["", "../repo", "a/b", "cOn", "Lpt1"] {
        assert!(matches!(
            ticket_worktree(&TicketId::from(value)),
            Err(Error::InvalidTicket)
        ));
    }
}

#[test]
fn a_second_reservation_cannot_replace_the_first() {
    let folder = tempfile::tempdir().unwrap();
    let marker = folder.path().join("ticket.owner.json");
    let mut first = Ownership {
        repository: vec![1, 2, 3],
        branch: "ticket/42".into(),
        base_branch: "main".into(),
        base: "1111111111111111111111111111111111111111".into(),
        ready: false,
        removing: false,
    };
    write_ownership(&marker, &first).unwrap();
    first.base = "2222222222222222222222222222222222222222".into();
    assert!(write_ownership(&marker, &first).is_err());
    let stored = read_ownership(&marker).unwrap().unwrap();
    assert_eq!(stored.base, "1111111111111111111111111111111111111111");
}

#[test]
fn a_ticket_waits_for_another_tickets_short_operation_on_the_project() {
    let folder = tempfile::tempdir().unwrap();
    let held = root_lock_within(folder.path(), Duration::ZERO).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
    });
    // Three tickets of one project start side by side: the one that arrives
    // while another holds the lock must get its turn, not be interrupted.
    root_lock_within(folder.path(), Duration::from_secs(10)).unwrap();
    release.join().unwrap();
}

#[test]
fn a_lock_that_is_never_released_still_ends_in_busy() {
    let folder = tempfile::tempdir().unwrap();
    let _held = root_lock_within(folder.path(), Duration::ZERO).unwrap();
    assert!(matches!(
        root_lock_within(folder.path(), Duration::from_millis(100)),
        Err(Error::Busy)
    ));
}
