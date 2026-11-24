use std::{ffi::OsString, path::PathBuf};

use super::{Boundary, Host, Wsl, linux_path, parse_discovery};
use crate::CommandSpec;

fn wsl(settings_dir: PathBuf) -> Wsl {
    let text = "home=/home/u\npath=/home/u/node/bin:/usr/bin\nsrt=/home/u/node/bin/srt\n\
                seccomp=/home/u/lib/vendor/seccomp\nhook=/home/u/.autopilot/bin/autopilot-hook\n\
                srt_version=0.0.83\n";
    parse_discovery(
        text,
        Host::Wsl,
        "Ubuntu",
        PathBuf::from(r"C:\Windows\System32\wsl.exe"),
        PathBuf::new(),
        settings_dir,
    )
    .expect("a complete answer parses")
}

fn args(spec: &CommandSpec) -> Vec<String> {
    spec.args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

// Invariant: a path that is not plainly absolute is refused, so a relative or
// `..` path cannot name a folder the caller did not mean to open.
// A change that breaks it: joining the text without checking its parts. No
// other test looks at the translation.
#[test]
fn a_windows_path_becomes_a_mnt_path_and_anything_unclear_is_refused() {
    let translated = |text: &str| linux_path(&PathBuf::from(text)).ok();
    assert_eq!(translated(r"D:\a\b"), Some("/mnt/d/a/b".into()));
    assert_eq!(translated("d:/a//b/"), Some("/mnt/d/a/b".into()));
    assert_eq!(translated(r"\\?\C:\x"), Some("/mnt/c/x".into()));
    assert_eq!(translated("/home/u/x"), Some("/home/u/x".into()));
    for refused in [
        r"relative\dir",
        r"D:rel",
        r"D:\a\..\b",
        "/home/u/../x",
        r"\\server\share\x",
        "/",
        r"D:\",
        "/mnt/d",
        "/home/u/a\nb",
    ] {
        assert_eq!(translated(refused), None, "{refused}");
    }
}

// Invariant: the settings deny `/mnt` and the home folder and then open only
// what the boundary names, with the seccomp helper readable so the command can
// start. A change that breaks it: opening a parent folder, or dropping the
// helper (every command then fails to start, as the matrix first showed).
#[test]
fn the_settings_open_only_what_the_boundary_names() {
    let directory = tempfile::tempdir().expect("temp dir");
    let wsl = wsl(directory.path().to_path_buf());
    let boundary = Boundary {
        write: vec![PathBuf::from(r"D:\work\wt")],
        read: vec![PathBuf::from("/home/u/.cargo")],
        protect: vec![PathBuf::from(r"D:\work\wt\.git")],
        hide: vec![PathBuf::from("/home/u/.cargo/credentials.toml")],
        domains: vec!["api.anthropic.com".into()],
        tmp: Some(PathBuf::from(r"D:\work\tmp")),
        mask_env: vec!["ANTHROPIC_API_KEY".into()],
    };
    let value: serde_json::Value =
        serde_json::from_str(&wsl.settings(&boundary).expect("settings")).expect("json");
    assert_eq!(value["network"]["allowedDomains"][0], "api.anthropic.com");
    assert_eq!(value["network"]["deniedDomains"], serde_json::json!([]));
    // A masked variable turns TLS termination on, and the real value is put
    // back only on requests to the hosts the boundary reaches.
    assert_eq!(
        value["network"]["tlsTerminate"],
        serde_json::json!({"excludeDomains": []})
    );
    assert_eq!(
        value["credentials"]["envVars"],
        serde_json::json!([{
            "name": "ANTHROPIC_API_KEY",
            "mode": "mask",
            "injectHosts": ["api.anthropic.com"]
        }])
    );
    // A masked variable turns TLS termination on, and the real value is put
    // back only on requests to the hosts the boundary reaches.
    assert_eq!(
        value["network"]["tlsTerminate"],
        serde_json::json!({"excludeDomains": []})
    );
    assert_eq!(
        value["credentials"]["envVars"],
        serde_json::json!([{
            "name": "ANTHROPIC_API_KEY",
            "mode": "mask",
            "injectHosts": ["api.anthropic.com"]
        }])
    );
    let fs = &value["filesystem"];
    assert_eq!(
        fs["denyRead"],
        serde_json::json!([
            "/mnt",
            "/media",
            "/home",
            "/root",
            "/srv",
            "/home/u/.cargo/credentials.toml"
        ])
    );
    assert_eq!(
        fs["allowRead"],
        serde_json::json!([
            "/home/u/.cargo",
            "/mnt/d/work/wt",
            "/mnt/d/work/tmp",
            "/home/u/lib/vendor/seccomp"
        ])
    );
    assert_eq!(
        fs["allowWrite"],
        serde_json::json!(["/mnt/d/work/wt", "/mnt/d/work/tmp"])
    );
    assert_eq!(fs["denyWrite"], serde_json::json!(["/mnt/d/work/wt/.git"]));

    let none = wsl.settings(&Boundary::default()).expect("settings");
    let value: serde_json::Value = serde_json::from_str(&none).expect("json");
    assert_eq!(
        value["network"]["allowedDomains"],
        serde_json::json!([]),
        "no domains means no network"
    );
    assert_eq!(value["filesystem"]["allowWrite"], serde_json::json!([]));
    assert!(
        value.get("credentials").is_none() && value["network"].get("tlsTerminate").is_none(),
        "no masked variable, no interception"
    );
}

// Invariant: a secret reaches the distro by `WSLENV` and never by a command
// line, which anything in the distro can read. A change that breaks it:
// putting every variable in the argument list.
#[test]
fn a_secret_travels_by_wslenv_and_never_by_command_line() {
    let directory = tempfile::tempdir().expect("temp dir");
    let wsl = wsl(directory.path().to_path_buf());
    let spec = CommandSpec {
        program: PathBuf::from("/home/u/node/bin/claude"),
        args: vec![OsString::from("-p"), OsString::from("a b")],
        env: vec![
            (
                OsString::from("ANTHROPIC_API_KEY"),
                OsString::from("sk-secret"),
            ),
            (OsString::from("LANG"), OsString::from("C.UTF-8")),
        ],
        cwd: Some(PathBuf::from(r"D:\work\wt")),
    };
    let wrapped = wsl
        .wrap(&spec, &Boundary::default(), &["ANTHROPIC_API_KEY"])
        .expect("wrap");
    let all = args(&wrapped).join("\u{1}");
    assert!(!all.contains("sk-secret"), "the key is on a command line");
    assert!(all.contains("LANG=C.UTF-8"));
    assert!(all.contains("ANTHROPIC_API_KEY"), "only the name is passed");
    let env: Vec<(String, String)> = wrapped
        .env
        .iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect();
    assert!(env.contains(&("ANTHROPIC_API_KEY".into(), "sk-secret".into())));
    assert!(env.contains(&("WSLENV".into(), "ANTHROPIC_API_KEY".into())));
    assert_eq!(
        wrapped.program,
        PathBuf::from(r"C:\Windows\System32\wsl.exe")
    );
}

// Invariant: the command and its arguments follow a lone `--` as separate
// parameters, after the working folder in Linux form. A change that breaks it:
// joining them into one string, which re-parses spaces and quotes.
#[test]
fn the_command_follows_the_separator_as_separate_parameters() {
    let directory = tempfile::tempdir().expect("temp dir");
    let wsl = wsl(directory.path().to_path_buf());
    let spec = CommandSpec {
        program: PathBuf::from("/usr/bin/env"),
        args: vec![OsString::from("a b"), OsString::from("$(x)")],
        env: Vec::new(),
        cwd: Some(PathBuf::from(r"D:\my work\wt")),
    };
    let wrapped = wsl.wrap(&spec, &Boundary::default(), &[]).expect("wrap");
    let list = args(&wrapped);
    let at = list.iter().position(|arg| arg == "--").expect("separator");
    assert_eq!(list[at + 1..], ["/usr/bin/env", "a b", "$(x)"]);
    assert_eq!(list[10], "", "no scratch folder");
    assert_eq!(list[6], "ap-sandbox");
    assert_eq!(list[7], "/home/u/node/bin/srt");
    assert_eq!(list[9], "/mnt/d/my work/wt");
    assert!(list[8].contains("/srt-"), "{}", list[8]);
    assert!(wrapped.env.iter().all(|(key, _)| key != "WSLENV"));
}

// Invariant: a command that is not an absolute Linux program, has no working
// folder or names a variable that is not a plain name is refused rather than
// run unconfined or with a smuggled value.
#[test]
fn a_command_that_cannot_be_passed_safely_is_refused() {
    let directory = tempfile::tempdir().expect("temp dir");
    let wsl = wsl(directory.path().to_path_buf());
    let base = || CommandSpec {
        program: PathBuf::from("/usr/bin/env"),
        args: Vec::new(),
        env: Vec::new(),
        cwd: Some(PathBuf::from(r"D:\work\wt")),
    };
    let run = |spec: &CommandSpec, secrets: &[&str]| {
        wsl.wrap(spec, &Boundary::default(), secrets).is_err()
    };
    let mut windows_program = base();
    windows_program.program = PathBuf::from(r"C:\tool.exe");
    assert!(run(&windows_program, &[]));
    let mut relative = base();
    relative.program = PathBuf::from("rel/env");
    assert!(run(&relative, &[]));
    let mut option_like = base();
    option_like.program = PathBuf::from("-x");
    assert!(run(&option_like, &[]));
    let mut plain_name = base();
    plain_name.program = PathBuf::from("cargo");
    assert!(
        !run(&plain_name, &[]),
        "a bare name is looked up in the distro"
    );
    let mut no_cwd = base();
    no_cwd.cwd = None;
    assert!(run(&no_cwd, &[]));
    let mut bad_name = base();
    bad_name.env = vec![(OsString::from("A=B"), OsString::from("x"))];
    assert!(run(&bad_name, &[]));
    assert!(run(&base(), &["not a name"]));
}

#[test]
fn a_distro_missing_a_tool_is_refused_by_name() {
    let text =
        "home=/h\npath=/usr/bin\nsrt=/x/srt\nseccomp=/x\nhook=/h/hook\nmissing:bwrap\nmissing:rg\n";
    let error = parse_discovery(
        text,
        Host::Wsl,
        "Ubuntu",
        PathBuf::from("wsl.exe"),
        PathBuf::new(),
        PathBuf::new(),
    )
    .expect_err("missing tools");
    let message = error.to_string();
    assert!(
        message.contains("bwrap") && message.contains("rg"),
        "{message}"
    );
    assert!(
        parse_discovery(
            "home=/h\n",
            Host::Wsl,
            "Ubuntu",
            PathBuf::from("wsl.exe"),
            PathBuf::new(),
            PathBuf::new()
        )
        .is_err()
    );
}

// Needs this PC's WSL distro with the tools installed (`tools/wsl/setup.sh`),
// so it stays out of the normal run: `cargo test -p autopilot-process -- --ignored wsl_`.
// Invariant: a command launched through `wrap` runs under the boundary on the
// real distro: it writes inside its folder, cannot write beside it, and gets
// only the environment it was given.
#[test]
#[ignore = "needs the WSL distro"]
#[cfg(windows)]
fn wsl_runs_a_wrapped_command_under_the_boundary() {
    use crate::{Limits, Outcome, Runner};
    use std::{sync::atomic::AtomicBool, time::Duration};

    let directory = tempfile::tempdir_in(r"D:\").expect("temp dir on D:");
    let work = directory.path().join("wt");
    let beside = directory.path().join("beside");
    std::fs::create_dir_all(&work).expect("work");
    std::fs::create_dir_all(&beside).expect("beside");
    let wsl = Wsl::discover("Ubuntu", directory.path().join("settings")).expect("discover");
    let boundary = Boundary {
        write: vec![work.clone()],
        ..Boundary::default()
    };
    let script = format!(
        "echo in > in.txt; echo out > '{}/out.txt' || echo blocked; printenv GH_TOKEN || echo no-token",
        super::linux_path(&beside).expect("path")
    );
    let spec = CommandSpec {
        program: PathBuf::from("/usr/bin/bash"),
        args: vec![OsString::from("-c"), OsString::from(script)],
        env: Vec::new(),
        cwd: Some(work.clone()),
    };
    let wrapped = wsl.wrap(&spec, &boundary, &[]).expect("wrap");
    let limits = Limits {
        timeout: Duration::from_secs(60),
        bytes_per_stream: 64 * 1024,
    };
    let output = Runner::new(PathBuf::new())
        .run(&wrapped, b"", limits, &AtomicBool::new(false))
        .expect("run");
    assert!(
        matches!(&output.outcome, Outcome::Exited(s) if s.success()),
        "{output:?}"
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("blocked") && text.contains("no-token"),
        "{text}"
    );
    assert!(
        work.join("in.txt").is_file(),
        "the write inside did not land"
    );
    assert!(
        !beside.join("out.txt").exists(),
        "a write beside the folder landed"
    );
}

// Invariant: arguments reach the program in the distro byte for byte, whatever
// quotes, spaces, backslashes or shell syntax they hold. Build passes a JSON
// settings argument, so a mangled quote would change what the agent runs with.
// A change that breaks it: building the inner command with plain joining.
#[test]
#[ignore = "needs the WSL distro"]
#[cfg(windows)]
fn wsl_passes_awkward_arguments_through_unchanged() {
    use crate::{Limits, Outcome, Runner};
    use std::{sync::atomic::AtomicBool, time::Duration};

    let directory = tempfile::tempdir_in(r"D:\").expect("temp dir on D:");
    let work = directory.path().join("my work");
    std::fs::create_dir_all(&work).expect("work");
    let wsl = Wsl::discover("Ubuntu", directory.path().join("settings")).expect("discover");
    let awkward = [
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"\"C:\a b\x.exe\" hook"}]}]}}"#,
        "it's $(echo x) `y` $HOME ${Z} * ; | & > <",
        "trailing backslash\\",
        "tab\there",
        "two\nlines",
        "ünïcödé ✔",
        "",
        "--flag=with space",
        r"C:\Users\x y\z",
    ];
    let spec = CommandSpec {
        program: PathBuf::from("/usr/bin/printf"),
        args: std::iter::once(OsString::from("%s\\0"))
            .chain(awkward.iter().map(OsString::from))
            .collect(),
        env: Vec::new(),
        cwd: Some(work.clone()),
    };
    let boundary = Boundary {
        write: vec![work],
        ..Boundary::default()
    };
    let wrapped = wsl.wrap(&spec, &boundary, &[]).expect("wrap");
    let limits = Limits {
        timeout: Duration::from_secs(60),
        bytes_per_stream: 64 * 1024,
    };
    let output = Runner::new(PathBuf::new())
        .run(&wrapped, b"", limits, &AtomicBool::new(false))
        .expect("run");
    assert!(
        matches!(&output.outcome, Outcome::Exited(s) if s.success()),
        "{output:?}"
    );
    let got: Vec<&str> = std::str::from_utf8(&output.stdout)
        .expect("utf8")
        .split_terminator('\0')
        .collect();
    assert_eq!(got, awkward);
}

fn discovery_with(extra: &str) -> std::io::Result<Wsl> {
    let text = format!("home=/h\npath=/usr/bin\nsrt=/x/srt\nseccomp=/x\nhook=/h/hook\n{extra}");
    parse_discovery(
        &text,
        Host::Wsl,
        "Ubuntu",
        PathBuf::from("wsl.exe"),
        PathBuf::new(),
        PathBuf::new(),
    )
}

// Invariant: an `srt` version the probe was never run with is refused, and so
// is a machine that does not report one. A change that breaks it: dropping the
// list check, so a newer `srt` with different settings semantics is trusted.
// The probe itself cannot catch it: it only runs after discovery succeeds.
#[test]
fn an_srt_version_that_was_not_validated_is_refused() {
    let error = discovery_with("srt_version=9.9.9\n").expect_err("unlisted");
    assert!(error.to_string().contains("9.9.9"), "{error}");
    assert!(discovery_with("").is_err());
    assert!(discovery_with("srt_version=\n").is_err());
}

// Invariant: the versions that go into the probe's output are the ones the
// tools reported, with the leading program name cut off.
#[test]
fn the_reported_tool_versions_are_kept_for_the_probe_output() {
    let wsl = discovery_with(
        "srt_version=0.0.83\nbwrap_version=bubblewrap 0.9.0\nclaude_version=2.1.330 (Claude Code)\n",
    )
    .expect("validated version");
    assert_eq!(
        wsl.tool_versions(),
        "srt 0.0.83, bubblewrap 0.9.0, claude 2.1.330"
    );
}

fn mac(settings_dir: PathBuf) -> Wsl {
    let text = "home=/Users/u\npath=/Users/u/.local/bin:/usr/bin\nsrt=/Users/u/.autopilot/npm/bin/srt\n\
                seccomp=none\nroot=/Users/u/.local/share/claude/versions\n\
                hook=/Users/u/.autopilot/bin/autopilot-hook\nsrt_version=0.0.83\n";
    parse_discovery(
        text,
        Host::Macos,
        "local",
        PathBuf::from("/usr/bin/env"),
        PathBuf::new(),
        settings_dir,
    )
    .expect("a Mac's answer parses without a seccomp helper")
}

// Invariant: on a Mac the settings hide every user's folder and other volumes,
// open only what the boundary names, and name no seccomp helper (there is none,
// and a bogus read entry would hide a mistake). A change that breaks it: reusing
// the Linux hidden roots, which leave `/Users` readable.
#[test]
fn a_mac_hides_users_and_has_no_seccomp_entry() {
    let directory = tempfile::tempdir().expect("temp dir");
    let wsl = mac(directory.path().to_path_buf());
    let boundary = Boundary {
        write: vec![PathBuf::from("/Users/u/work/wt")],
        read: vec![PathBuf::from("/Users/u/.local/share/claude/versions")],
        protect: vec![PathBuf::from("/Users/u/work/wt/.git")],
        hide: Vec::new(),
        domains: vec!["api.anthropic.com".into()],
        tmp: None,
        mask_env: Vec::new(),
    };
    let settings: serde_json::Value =
        serde_json::from_str(&wsl.settings(&boundary).expect("settings")).expect("json");
    let deny = &settings["filesystem"]["denyRead"];
    assert_eq!(deny[0], "/Users");
    assert_eq!(deny[1], "/Volumes");
    let allow: Vec<&str> = settings["filesystem"]["allowRead"]
        .as_array()
        .expect("list")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert!(allow.contains(&"/Users/u/.local/share/claude/versions"));
    assert!(!allow.iter().any(|path| path.is_empty() || *path == "none"));
}

// Invariant: a Mac matches Seatbelt rules against real paths, so a folder given
// through a symlink (`/tmp`) reaches the settings as its target. A change that
// breaks it: passing the path through, which leaves the write rule matching
// nothing and every write in the worktree refused.
#[test]
#[cfg(target_os = "macos")]
fn a_mac_resolves_a_symlinked_folder_before_it_writes_the_rule() {
    let directory = tempfile::tempdir_in("/tmp").expect("temp dir");
    let wsl = mac(directory.path().join("settings"));
    let boundary = Boundary {
        write: vec![directory.path().to_path_buf()],
        ..Boundary::default()
    };
    let text = wsl.settings(&boundary).expect("settings");
    assert!(text.contains("/private/tmp/"), "{text}");
    assert!(!text.contains("\"/tmp/"), "{text}");
}

// Invariant: the discovery of a Mac names the tools folder it found, so the
// agent role can read the installed program and nothing else under the home.
#[test]
fn a_mac_keeps_the_tool_folders_the_discovery_named() {
    let directory = tempfile::tempdir().expect("temp dir");
    let wsl = mac(directory.path().to_path_buf());
    assert_eq!(wsl.tool_roots(), ["/Users/u/.local/share/claude/versions"]);
    assert_eq!(wsl.host(), Host::Macos);
}
