use std::path::PathBuf;

use super::{NPM_REGISTRY, PROVIDER, REGISTRY, ROWS, Role, boundary, judge, probe_script, rows};
use crate::wsl::{Host, Wsl, parse_discovery};

fn wsl() -> Wsl {
    let text = "home=/home/u\npath=/usr/bin\nsrt=/home/u/node/bin/srt\nseccomp=/home/u/v/seccomp\nhook=/home/u/.autopilot/bin/autopilot-hook\nsrt_version=0.0.83\n";
    parse_discovery(
        text,
        Host::Wsl,
        "Ubuntu",
        PathBuf::from("wsl.exe"),
        PathBuf::new(),
        PathBuf::new(),
    )
    .expect("parses")
}

fn answers(flip: Option<&str>, drop: Option<&str>) -> String {
    let mut text = String::new();
    for row in ROWS.iter().filter(|row| Some(**row) != drop) {
        let works = matches!(
            *row,
            "write-worktree"
                | "write-scratch"
                | "hook-denies-push"
                | "hook-allows-status"
                | "key-is-masked"
        );
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
    assert_eq!(
        judge(&answers(None, None), "agent", Host::Wsl, true),
        Vec::<String>::new()
    );
    let leaked = judge(
        &answers(Some("read-sibling"), None),
        "agent",
        Host::Wsl,
        true,
    );
    assert_eq!(leaked, ["agent: read-sibling was allowed"]);
    let broken = judge(
        &answers(Some("write-worktree"), None),
        "check",
        Host::Wsl,
        false,
    );
    assert_eq!(broken, ["check: write-worktree was denied but must work"]);
    let silent = judge(
        &answers(None, Some("run-windows-program")),
        "agent",
        Host::Wsl,
        true,
    );
    assert_eq!(silent, ["agent: run-windows-program gave no answer"]);
    assert_eq!(judge("", "agent", Host::Wsl, true).len(), ROWS.len());
}

// Invariant: a Linux host is judged on exactly the rows that exist there, and
// the probe does not ask the others. A row that cannot happen (a Windows
// program, `/init`) must not pass by being trivially denied, and must not fail
// a machine for lacking Windows. A change that breaks it: one shared row list.
#[test]
fn a_linux_host_drops_the_windows_rows_and_keeps_the_rest() {
    let linux = rows(Host::Linux, true);
    for gone in ["read-windows-drive", "run-windows-program", "run-init"] {
        assert!(!linux.contains(&gone), "{gone}");
        assert!(!probe_script(Host::Linux, true).contains(&format!("LABEL={gone} ")));
        assert!(probe_script(Host::Wsl, true).contains(&format!("LABEL={gone} ")));
    }
    assert_eq!(linux.len(), ROWS.len() - 3);
    // An answer set that only has the Linux rows passes on Linux and fails on WSL.
    let text: String = linux.iter().map(|row| answers_for(row)).collect();
    assert_eq!(
        judge(&text, "agent", Host::Linux, true),
        Vec::<String>::new()
    );
    assert_eq!(judge(&text, "agent", Host::Wsl, true).len(), 3);
}

fn answers_for(row: &str) -> String {
    let works = matches!(
        row,
        "write-worktree"
            | "write-scratch"
            | "hook-denies-push"
            | "hook-allows-status"
            | "key-is-masked"
    );
    format!(
        "{} {row}
",
        if works { "ALLOWED" } else { "DENIED" }
    )
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

    let node = boundary(
        &wsl,
        &Role::FetchNode {
            worktree: PathBuf::from(r"D:\w\wt"),
            tmp: PathBuf::from(r"D:\w\tmp"),
        },
    );
    assert_eq!(node.domains, NPM_REGISTRY, "only the npm registry");
    let writes: Vec<String> = node
        .write
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        writes,
        [r"D:\w\wt", "/home/u/.npm"],
        "the npm cache, not the home folder"
    );
    assert!(node.read.iter().all(|p| p.starts_with("/home/u/")));
    assert!(
        !node.write.iter().any(|p| p.starts_with("/home/u/.cargo")),
        "an npm fetch cannot touch the cargo cache"
    );
    // The check role reaches both toolchains, so `cargo test` and `npm test`
    // run, and still has no network.
    let reads: Vec<String> = check
        .read
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert!(reads.contains(&"/home/u/.cargo".to_owned()));
    assert!(reads.contains(&"/home/u/node".to_owned()));
    assert_eq!(check.domains, Vec::<String>::new());
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

// The same probe on a Linux host, with no `wsl.exe`: run inside the distro,
// which is a Linux machine. Needs the tools from `tools/wsl`.
// Invariant: a Linux host validates on the rows that exist there.
#[test]
#[ignore = "needs the sandbox tools"]
#[cfg(target_os = "linux")]
fn the_real_boundary_passes_the_probe_on_a_linux_host() {
    // Test binaries sit in target/<profile>/deps, the guardian one level up.
    let exe = std::env::current_exe().expect("exe");
    let guardian = exe
        .parent()
        .and_then(std::path::Path::parent)
        .expect("target dir")
        .join("autopilot-process-guardian");
    assert!(guardian.is_file(), "build the workspace first");
    // Under the home folder, like a real data directory: `/tmp` is readable by
    // design, so a scene there would not show the home-folder rule.
    let home = std::env::var_os("HOME").expect("HOME");
    let directory = tempfile::tempdir_in(home).expect("temp dir");
    let wsl = Wsl::discover_local(directory.path().join("settings"), guardian).expect("discover");
    assert_eq!(wsl.host(), Host::Linux);
    super::validate(wsl, directory.path()).expect("the boundary holds");
}

#[cfg(target_os = "macos")]
fn mac_wsl(directory: &std::path::Path) -> Wsl {
    // A Mac supervises without a guardian program.
    let wsl = Wsl::discover_local(directory.join("settings"), PathBuf::new()).expect("discover");
    assert_eq!(wsl.host(), Host::Macos);
    wsl
}

// Needs `tools/macos/setup.sh`, `claude` and `node` on this Mac.
// Invariant: on a Mac set up with the script, the real probe passes for every
// role, so a real launch can start.
#[test]
#[ignore = "needs the sandbox tools"]
#[cfg(target_os = "macos")]
fn the_real_boundary_passes_the_probe_on_a_mac() {
    let directory = tempfile::tempdir_in(std::env::var_os("HOME").expect("HOME")).expect("dir");
    let wsl = mac_wsl(directory.path());
    let report = super::check(&wsl, directory.path()).expect("probe runs");
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(report.rows.iter().all(|row| row.ok));
}
