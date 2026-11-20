use std::{ffi::OsStr, path::Path};

use crate::{Error, parse_worktrees};

#[test]
fn worktree_inventory_keeps_unusual_paths_and_checkout_states() {
    let root = if cfg!(windows) { "C:/repo" } else { "/repo" };
    let input = format!(
        "worktree {root}\0bare\0\0\
         worktree {root}/ticket with\nnewline\0\
         HEAD 1111111111111111111111111111111111111111\0\
         branch refs/heads/ticket/42\0locked awaiting cleanup\0\0\
         worktree {root}/detached\0\
         HEAD 2222222222222222222222222222222222222222\0\
         detached\0prunable\0\0"
    );
    let entries = parse_worktrees(input.as_bytes()).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries[0].bare);
    assert!(entries[0].head.is_none());
    assert_eq!(
        entries[1].path,
        Path::new(root).join("ticket with\nnewline")
    );
    assert_eq!(
        entries[1].branch.as_deref(),
        Some(OsStr::new("refs/heads/ticket/42"))
    );
    assert!(entries[1].locked);
    assert!(entries[2].branch.is_none());
    assert!(entries[2].prunable);
    assert_eq!(
        entries[2].head.as_ref().unwrap().as_str(),
        "2222222222222222222222222222222222222222"
    );
}

#[test]
fn an_incomplete_inventory_cannot_be_used_to_reconcile_worktrees() {
    let path = if cfg!(windows) { "C:/repo" } else { "/repo" };
    let input = format!("worktree {path}\0bare\0\0worktree {path}/ticket\0");
    assert!(matches!(
        parse_worktrees(input.as_bytes()),
        Err(Error::Inventory)
    ));
}

#[test]
fn conflicting_checkout_states_cannot_be_used_to_reconcile_worktrees() {
    let path = if cfg!(windows) { "C:/repo" } else { "/repo" };
    let head = "HEAD 1111111111111111111111111111111111111111";
    for fields in [
        format!("{head}\0branch refs/heads/ticket/42\0detached"),
        head.to_owned(),
        format!("bare\0{head}"),
        format!("{head}\0{head}\0detached"),
    ] {
        let input = format!("worktree {path}\0{fields}\0\0");
        assert!(matches!(
            parse_worktrees(input.as_bytes()),
            Err(Error::Inventory)
        ));
    }
}
