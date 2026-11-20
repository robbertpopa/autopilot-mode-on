use super::{Error, Ownership, TicketId, read_ownership, ticket_worktree, write_ownership};

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
    };
    write_ownership(&marker, &first).unwrap();
    first.base = "2222222222222222222222222222222222222222".into();
    assert!(write_ownership(&marker, &first).is_err());
    let stored = read_ownership(&marker).unwrap().unwrap();
    assert_eq!(stored.base, "1111111111111111111111111111111111111111");
}
