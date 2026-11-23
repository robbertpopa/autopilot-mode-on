use std::path::PathBuf;

use super::{PROVIDER, REGISTRY, ROWS, Role, boundary, judge};
use crate::wsl::{Wsl, parse_discovery};

fn wsl() -> Wsl {
    let text = "home=/home/u\npath=/usr/bin\nsrt=/home/u/node/bin/srt\nseccomp=/home/u/v/seccomp\n";
    parse_discovery(text, "Ubuntu", PathBuf::from("wsl.exe"), PathBuf::new()).expect("parses")
}

fn answers(flip: Option<&str>, drop: Option<&str>) -> String {
    let mut text = String::new();
    for row in ROWS.iter().filter(|row| Some(**row) != drop) {
        let works = matches!(*row, "write-worktree" | "write-scratch");
        let allowed = works != (Some(*row) == flip);
        text.push_str(if allowed { "ALLOWED " } else { "DENIED " });
        text.push_str(row);
        text.push('\n');
    }
    text
}

// Invariant: validation passes only when every row came back as expected. A
// row that was allowed but must be denied, one that is denied but must work
// (the sandbox then could not run anything) and one with no answer (a probe
// that died half way) each fail it. A change that breaks it: treating a
// missing row as a pass, or checking only the denials.
#[test]
fn the_probe_passes_only_when_every_row_is_as_expected() {
    assert_eq!(judge(&answers(None, None), "agent"), Vec::<String>::new());
    let leaked = judge(&answers(Some("read-sibling"), None), "agent");
    assert_eq!(leaked, ["agent: read-sibling was allowed"]);
    let broken = judge(&answers(Some("write-worktree"), None), "check");
    assert_eq!(broken, ["check: write-worktree was denied but must work"]);
    let silent = judge(&answers(None, Some("run-windows-program")), "agent");
    assert_eq!(silent, ["agent: run-windows-program gave no answer"]);
    assert_eq!(judge("", "agent").len(), ROWS.len());
}

// Invariant: the agent role reaches only the provider, may not change `.git`,
// and the check role has no network and cannot read the cargo credentials.
// A change that breaks it: widening a role's domain list or dropping the
// protected path; the probe would still pass because it runs the same boundary.
#[test]
fn each_role_opens_only_what_it_is_for() {
    let wsl = wsl();
    let agent = boundary(
        &wsl,
        &Role::Agent {
            worktree: PathBuf::from(r"D:\w\wt"),
            config: PathBuf::from(r"D:\w\cfg"),
        },
    );
    assert_eq!(agent.domains, [PROVIDER]);
    assert_eq!(agent.protect, [PathBuf::from(r"D:\w\wt").join(".git")]);
    assert_eq!(
        agent.write,
        [PathBuf::from(r"D:\w\wt"), PathBuf::from(r"D:\w\cfg")]
    );
    assert!(agent.read.iter().all(|p| p.starts_with("/home/u/")));

    let check = boundary(
        &wsl,
        &Role::Check {
            worktree: PathBuf::from(r"D:\w\wt"),
            tmp: PathBuf::from(r"D:\w\tmp"),
        },
    );
    assert!(check.domains.is_empty(), "a check has no network");
    assert_eq!(check.write, [PathBuf::from(r"D:\w\wt")]);
    assert!(
        check
            .hide
            .contains(&PathBuf::from("/home/u/.cargo/credentials.toml"))
    );
    assert_eq!(check.protect, [PathBuf::from(r"D:\w\wt").join(".git")]);

    let fetch = boundary(
        &wsl,
        &Role::Fetch {
            worktree: PathBuf::from(r"D:\w\wt"),
            tmp: PathBuf::from(r"D:\w	mp"),
        },
    );
    assert_eq!(fetch.domains, REGISTRY, "only the registry hosts");
    let writes: Vec<String> = fetch
        .write
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        writes,
        [r"D:\w\wt", "/home/u/.cargo/registry", "/home/u/.cargo/git"],
        "the cache, not the whole cargo home"
    );
}

// Needs the real distro; see `wsl_runs_a_wrapped_command_under_the_boundary`.
// Invariant: on a machine set up with `tools/wsl`, the real probe passes for
// both roles, so a real launch can start.
#[test]
#[ignore = "needs the WSL distro"]
#[cfg(windows)]
fn the_real_boundary_passes_the_probe() {
    let directory = tempfile::tempdir_in(r"D:\").expect("temp dir on D:");
    let wsl = Wsl::discover("Ubuntu", directory.path().join("settings")).expect("discover");
    super::validate(wsl, directory.path()).expect("the boundary holds");
}
