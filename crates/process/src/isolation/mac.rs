//! The escape probe for a Mac. Seatbelt has no network namespace: loopback and
//! Unix sockets are one network rule, the folders are matched as real paths, and
//! system services reached over Mach ports can act for a sandboxed process. So
//! these rows are written for it, not taken from the Linux probe.
//!
//! Every row that must be denied has a control that must work: the same action
//! from outside the boundary (the host connects to its own listener, reads its
//! own decoy file), or the same action inside it on an allowed path. A denied
//! answer therefore means the boundary refused, not that the setup was broken.

use std::{
    ffi::OsString,
    fs, io,
    net::{TcpListener, TcpStream},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use super::{
    API_KEY, GIT_CONFIG, MARKER, PROBE_SECRET, Report, Role, RowResult, boundary, role_name, run,
};
use crate::{
    CommandSpec,
    wsl::{Boundary, Wsl},
};

/// A preference file the `defaults` tool writes through the `cfprefsd` service.
/// It must not appear: the sandboxed process may not write outside its folders
/// by asking a system service to.
const PREFS_PREFIX: &str = "com.autopilot.isolation-probe";

/// Each line is `ALLOWED label` or `DENIED label`. Positional parameters:
/// worktree, sibling, port, scratch folder, config folder, socket, hook, secret,
/// decoy folder, `~/.ssh`, `~/.claude`, the preference domain.
const PROBE: &str = r#"w=$1; s=$2; port=$3; tmp=$4; cfg=$5; sock=$6; hook=$7; secret=$8; decoy=$9; ssh=${10}; claude=${11}; domain=${12}
push='{"tool_name":"Bash","tool_input":{"command":"git push origin main"}}'
status='{"tool_name":"Bash","tool_input":{"command":"git status"}}'
r() { if "$@" </dev/null >/dev/null 2>&1; then echo "ALLOWED $LABEL"; else echo "DENIED $LABEL"; fi; }
LABEL=write-worktree r /bin/bash -c 'echo x > "$0/out.txt"' "$w"
LABEL=write-scratch r /bin/bash -c 'echo x > "$0/out.txt"' "$tmp"
LABEL=write-config r /bin/bash -c 'echo x > "$0/out.txt"' "$cfg"
LABEL=write-sibling r /bin/bash -c 'echo x > "$0/new.txt"' "$s"
LABEL=write-git-config r /bin/bash -c 'echo "[x]" >> "$0/.git/config"' "$w"
LABEL=write-git-hook r /bin/bash -c 'echo x > "$0/.git/hooks/pre-push"' "$w"
LABEL=write-hook r /bin/bash -c 'echo x >> "$0"' "$hook"
LABEL=write-prefs-by-service r /usr/bin/defaults write "$domain" key 1
LABEL=read-sibling r /bin/cat "$s/marker.txt"
LABEL=read-home-file r /bin/cat "$decoy/note.txt"
LABEL=read-key-file r /bin/cat "$decoy/claude_key.txt"
LABEL=read-other-repo r /bin/cat "$decoy/repo/.git/config"
LABEL=read-ssh-folder r /bin/ls "$ssh"
LABEL=read-claude-folder r /bin/ls "$claude"
LABEL=connect-loopback r /usr/bin/nc -z -w 3 127.0.0.1 "$port"
LABEL=connect-unix-socket r /usr/bin/nc -U -w 3 "$sock"
LABEL=reach-other-host r /usr/bin/curl -sS -m 15 -o /dev/null https://example.com/
LABEL=reach-provider r /usr/bin/curl -sS -m 20 -o /dev/null https://api.anthropic.com/
LABEL=hook-denies-push r /bin/bash -c 'printf %s "$1" | "$0" | grep -q "permissionDecision.:.deny"' "$hook" "$push"
LABEL=hook-allows-status r /bin/bash -c '[ "$(printf %s "$1" | "$0")" = "{}" ]' "$hook" "$status"
LABEL=key-is-masked r /bin/bash -c '[ -n "$ANTHROPIC_API_KEY" ] && [ "$ANTHROPIC_API_KEY" != "$0" ]' "$secret"
"#;

/// The rows, with whether the agent role, the other roles, or both run them.
const ROWS: &[&str] = &[
    "write-worktree",
    "write-scratch",
    "write-config",
    "write-sibling",
    "write-git-config",
    "write-git-hook",
    "write-hook",
    "write-prefs-by-service",
    "read-sibling",
    "read-home-file",
    "read-key-file",
    "read-other-repo",
    "read-ssh-folder",
    "read-claude-folder",
    "connect-loopback",
    "connect-unix-socket",
    "reach-other-host",
    "reach-provider",
    "hook-denies-push",
    "hook-allows-status",
    "key-is-masked",
];

const AGENT_ONLY: &[&str] = &[
    "write-config",
    "write-hook",
    "reach-provider",
    "hook-denies-push",
    "hook-allows-status",
    "key-is-masked",
];

/// The rows that must work. Every other row must be refused.
const MUST_WORK: &[&str] = &[
    "write-worktree",
    "write-scratch",
    "write-config",
    "reach-provider",
    "hook-denies-push",
    "hook-allows-status",
    "key-is-masked",
];

/// A row that only means something when its folder exists on this machine. The
/// control is the host reading the same folder.
const IF_PRESENT: &[&str] = &["read-ssh-folder", "read-claude-folder"];

fn rows_for(agent: bool) -> Vec<&'static str> {
    ROWS.iter()
        .copied()
        .filter(|row| agent || !AGENT_ONLY.contains(row))
        .collect()
}

struct Scene {
    worktree: PathBuf,
    sibling: PathBuf,
    config: PathBuf,
    tmp: PathBuf,
    sock: PathBuf,
    /// Stands in for files in the user's home folder: a note, a key file and
    /// another repository. It lives in the real home, outside every folder the
    /// boundary opens.
    decoy: PathBuf,
    ssh: PathBuf,
    claude: PathBuf,
    /// Makes the socket, the decoys and the preference domain unique, so two
    /// probes running at once do not see each other's files.
    id: String,
}

impl Scene {
    fn prefs_domain(&self) -> String {
        format!("{PREFS_PREFIX}-{}", self.id)
    }

    fn prefs_file(&self, wsl: &Wsl) -> PathBuf {
        PathBuf::from(wsl.home()).join(format!("Library/Preferences/{}.plist", self.prefs_domain()))
    }

    fn build(wsl: &Wsl, work: &Path) -> io::Result<Self> {
        let root = work.join("isolation-check");
        match fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::create_dir_all(&root)?;
        // Seatbelt matches real paths, so the scene is built on one.
        let root = fs::canonicalize(&root)?;
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let id = format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let scene = Self {
            worktree: root.join("wt"),
            sibling: root.join("sibling"),
            config: root.join("config"),
            tmp: root.join("tmp"),
            // A socket path is limited to about 100 bytes, so it sits in a short folder.
            sock: PathBuf::from(format!("/private/tmp/ap-probe-{id}.sock")),
            decoy: PathBuf::from(wsl.home())
                .join(".autopilot")
                .join(format!("isolation-decoy-{id}")),
            ssh: PathBuf::from(wsl.home()).join(".ssh"),
            claude: PathBuf::from(wsl.home()).join(".claude"),
            id,
        };
        fs::create_dir_all(scene.worktree.join(".git").join("hooks"))?;
        fs::create_dir_all(&scene.sibling)?;
        fs::write(scene.worktree.join(".git").join("config"), GIT_CONFIG)?;
        fs::write(scene.sibling.join("marker.txt"), MARKER)?;
        fs::create_dir_all(scene.decoy.join("repo").join(".git"))?;
        fs::write(scene.decoy.join("note.txt"), MARKER)?;
        fs::write(scene.decoy.join("claude_key.txt"), MARKER)?;
        fs::write(scene.decoy.join("repo").join(".git").join("config"), MARKER)?;
        Ok(scene)
    }

    fn roles(&self) -> [Role; 4] {
        [
            Role::Agent {
                worktree: self.worktree.clone(),
                config: self.config.clone(),
            },
            Role::Check {
                worktree: self.worktree.clone(),
                tmp: self.tmp.clone(),
            },
            Role::Fetch {
                worktree: self.worktree.clone(),
                tmp: self.tmp.clone(),
            },
            Role::FetchNode {
                worktree: self.worktree.clone(),
                tmp: self.tmp.clone(),
            },
        ]
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.decoy);
        let _ = fs::remove_file(&self.sock);
    }
}

/// Listeners the host runs outside the boundary: one TCP port on loopback and
/// one Unix socket. A connection that arrives sets a flag.
struct Listeners {
    port: u16,
    tcp_hit: Arc<AtomicBool>,
    unix_hit: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Listeners {
    fn start(scene: &Scene) -> io::Result<Self> {
        let tcp = TcpListener::bind("127.0.0.1:0")?;
        let port = tcp.local_addr()?.port();
        let _ = fs::remove_file(&scene.sock);
        let unix = UnixListener::bind(&scene.sock)?;
        tcp.set_nonblocking(true)?;
        unix.set_nonblocking(true)?;
        let tcp_hit = Arc::new(AtomicBool::new(false));
        let unix_hit = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let (hit, quit) = (tcp_hit.clone(), stop.clone());
        threads.push(std::thread::spawn(move || {
            while !quit.load(Ordering::Relaxed) {
                if tcp.accept().is_ok() {
                    hit.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }));
        let (hit, quit) = (unix_hit.clone(), stop.clone());
        threads.push(std::thread::spawn(move || {
            while !quit.load(Ordering::Relaxed) {
                if unix.accept().is_ok() {
                    hit.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }));
        let listeners = Self {
            port,
            tcp_hit,
            unix_hit,
            stop,
            threads,
        };
        // Controls: from outside the boundary both must be reachable, or a
        // denied row below would prove nothing.
        TcpStream::connect(("127.0.0.1", port))?;
        UnixStream::connect(&scene.sock)?;
        for _ in 0..50 {
            if listeners.tcp_hit.load(Ordering::Relaxed)
                && listeners.unix_hit.load(Ordering::Relaxed)
            {
                listeners.tcp_hit.store(false, Ordering::Relaxed);
                listeners.unix_hit.store(false, Ordering::Relaxed);
                return Ok(listeners);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(io::Error::other(
            "the control connections to the test listeners failed, so the loopback rows cannot be trusted",
        ))
    }
}

impl Drop for Listeners {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Run the probe for every role. `tweak` weakens a boundary before it is used,
/// so a test can show that each rule is what makes its row pass.
pub(super) fn check(
    wsl: &Wsl,
    work: &Path,
    tweak: &dyn Fn(&Role, &mut Boundary),
) -> io::Result<Report> {
    let scene = Scene::build(wsl, work)?;
    let listeners = Listeners::start(&scene)?;
    // The controls for the folder rows: the host itself can read them.
    let present: Vec<&str> = [
        ("read-ssh-folder", &scene.ssh),
        ("read-claude-folder", &scene.claude),
    ]
    .into_iter()
    .filter(|(_, path)| fs::read_dir(path).is_ok())
    .map(|(row, _)| row)
    .collect();
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for role in scene.roles() {
        let found = probe(wsl, &scene, &listeners, &role, &present, tweak)?;
        let name = role_name(&role);
        let agent = matches!(role, Role::Agent { .. });
        for row in rows_for(agent) {
            let prefix = format!("{name}: {row} ");
            results.push(RowResult {
                role: name,
                row,
                ok: !found.iter().any(|failure| failure.starts_with(&prefix)),
            });
        }
        failures.extend(found);
    }
    Ok(Report {
        versions: wsl.tool_versions(),
        rows: results,
        failures,
    })
}

fn probe(
    wsl: &Wsl,
    scene: &Scene,
    listeners: &Listeners,
    role: &Role,
    present: &[&str],
    tweak: &dyn Fn(&Role, &mut Boundary),
) -> io::Result<Vec<String>> {
    let name = role_name(role);
    let agent = matches!(role, Role::Agent { .. });
    let spec = probe_spec(wsl, scene, listeners, role);
    // The folders a role writes exist first, as in a real launch.
    let mut limits = boundary(wsl, role);
    tweak(role, &mut limits);
    for folder in limits.write.iter().chain(&limits.tmp) {
        fs::create_dir_all(folder)?;
    }
    let secrets: &[&str] = if agent { &[API_KEY] } else { &[] };
    let wrapped = wsl.wrap(&spec, &limits, secrets)?;
    let finished = run(wsl, &wrapped, Duration::from_secs(240))?;
    let mut failures = Vec::new();
    if !finished.success {
        failures.push(format!(
            "{name}: the probe did not run ({})",
            finished.stderr.trim()
        ));
        return Ok(failures);
    }
    failures.extend(judge(&finished.stdout, name, agent, present));
    failures.extend(host_findings(wsl, scene, listeners, name));
    Ok(failures)
}

fn probe_spec(wsl: &Wsl, scene: &Scene, listeners: &Listeners, role: &Role) -> CommandSpec {
    let agent = matches!(role, Role::Agent { .. });
    let scratch = match role {
        Role::Agent { config, .. } => config.join("tmp"),
        Role::Check { tmp, .. } | Role::Fetch { tmp, .. } | Role::FetchNode { tmp, .. } => {
            tmp.clone()
        }
    };
    let _ = fs::remove_file(scene.prefs_file(wsl));
    CommandSpec {
        program: PathBuf::from("/bin/bash"),
        args: vec![
            OsString::from("-c"),
            OsString::from(PROBE),
            OsString::from("probe"),
            scene.worktree.clone().into(),
            scene.sibling.clone().into(),
            listeners.port.to_string().into(),
            scratch.into(),
            match role {
                Role::Agent { config, .. } => config.clone().into(),
                _ => scene.config.clone().into(),
            },
            scene.sock.clone().into(),
            wsl.hook().into(),
            PROBE_SECRET.into(),
            scene.decoy.clone().into(),
            scene.ssh.clone().into(),
            scene.claude.clone().into(),
            scene.prefs_domain().into(),
        ],
        env: if agent {
            vec![(OsString::from(API_KEY), OsString::from(PROBE_SECRET))]
        } else {
            Vec::new()
        },
        cwd: Some(scene.worktree.clone()),
    }
}

fn judge(stdout: &str, name: &str, agent: bool, present: &[&str]) -> Vec<String> {
    let mut failures = Vec::new();
    for row in rows_for(agent) {
        if IF_PRESENT.contains(&row) && !present.contains(&row) {
            continue;
        }
        let must_work = MUST_WORK.contains(&row);
        let answer = stdout.lines().find_map(|line| {
            let (verdict, label) = line.split_once(' ')?;
            (label == row).then_some(verdict)
        });
        match (answer, must_work) {
            (Some("ALLOWED"), true) | (Some("DENIED"), false) => {}
            (Some("ALLOWED"), false) => failures.push(format!("{name}: {row} was allowed")),
            (Some("DENIED"), true) => {
                failures.push(format!("{name}: {row} was denied but must work"));
            }
            _ => failures.push(format!("{name}: {row} gave no answer")),
        }
    }
    failures
}

/// What the host sees afterwards. A row that answered "denied" must also have
/// changed nothing.
fn host_findings(wsl: &Wsl, scene: &Scene, listeners: &Listeners, name: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut check = |bad: bool, text: &str| {
        if bad {
            found.push(format!("{name}: {text}"));
        }
    };
    check(
        fs::read_to_string(scene.worktree.join(".git").join("config"))
            .ok()
            .as_deref()
            != Some(GIT_CONFIG),
        "the Git config changed",
    );
    check(
        fs::read_to_string(scene.sibling.join("marker.txt"))
            .ok()
            .as_deref()
            != Some(MARKER),
        "the sibling marker changed",
    );
    check(
        listeners.tcp_hit.swap(false, Ordering::Relaxed),
        "the loopback listener was reached",
    );
    check(
        listeners.unix_hit.swap(false, Ordering::Relaxed),
        "the Unix socket listener was reached",
    );
    let prefs = scene.prefs_file(wsl);
    for landed in [
        scene.sibling.join("new.txt"),
        scene.worktree.join(".git").join("hooks").join("pre-push"),
        prefs,
    ] {
        check(landed.exists(), &format!("{} exists", landed.display()));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: every label the script prints is a row the judge knows, and
    // every row is printed, so a typo in either cannot make a row vanish.
    #[test]
    fn the_script_and_the_row_list_name_the_same_rows() {
        let printed: Vec<&str> = PROBE
            .lines()
            .filter_map(|line| line.strip_prefix("LABEL="))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        assert_eq!(printed, ROWS);
        assert!(MUST_WORK.iter().all(|row| ROWS.contains(row)));
        assert!(AGENT_ONLY.iter().all(|row| ROWS.contains(row)));
        assert!(IF_PRESENT.iter().all(|row| ROWS.contains(row)));
    }

    // Invariant: a row that answered the wrong way, or not at all, is a failure
    // that names the row. Breaks if `judge` treats a missing answer as a pass.
    #[test]
    fn a_wrong_or_missing_answer_fails_the_named_row() {
        let mut text = String::new();
        for row in rows_for(true) {
            let works = MUST_WORK.contains(&row);
            if row == "write-git-config" {
                text.push_str("ALLOWED write-git-config\n");
            } else if row == "reach-provider" {
                text.push_str("DENIED reach-provider\n");
            } else if row != "read-sibling" {
                let verdict = if works { "ALLOWED" } else { "DENIED" };
                text.push_str(verdict);
                text.push(' ');
                text.push_str(row);
                text.push('\n');
            }
        }
        let present = ["read-ssh-folder", "read-claude-folder"];
        let failures = judge(&text, "agent", true, &present);
        for expected in [
            "agent: write-git-config was allowed",
            "agent: read-sibling gave no answer",
            "agent: reach-provider was denied but must work",
        ] {
            assert!(failures.iter().any(|f| f == expected), "{failures:?}");
        }
        assert_eq!(failures.len(), 3, "{failures:?}");
    }

    fn mac_wsl(directory: &Path) -> Wsl {
        // A Mac supervises without a guardian program.
        Wsl::discover_local(directory.join("settings"), PathBuf::new()).expect("discover")
    }

    fn failures_with(tweak: &dyn Fn(&Role, &mut Boundary)) -> Vec<String> {
        let directory = tempfile::tempdir_in(std::env::var_os("HOME").expect("HOME")).expect("dir");
        let wsl = mac_wsl(directory.path());
        check(&wsl, directory.path(), tweak)
            .expect("probe runs")
            .failures
    }

    fn agent(role: &Role) -> bool {
        matches!(role, Role::Agent { .. })
    }

    // The mutations below weaken one rule at a time and expect the row that rule
    // protects to fail by name. Each would pass unnoticed if its row were
    // missing or if the probe asked the wrong question.
    // Invariant: each folder, network and key rule is what makes its row pass.
    #[test]
    #[ignore = "needs the sandbox tools"]
    fn weakening_one_rule_fails_its_row() {
        type Tweak = Box<dyn Fn(&Role, &mut Boundary)>;
        let cases: Vec<(&str, Tweak, &str)> = vec![
            (
                "a writable parent folder",
                Box::new(|role, b| {
                    if agent(role) {
                        let parent = b.write[0].parent().expect("parent").parent().expect("root");
                        b.write.push(parent.to_path_buf());
                    }
                }),
                "agent: write-sibling was allowed",
            ),
            (
                ".git left writable",
                Box::new(|_, b| b.protect.clear()),
                "agent: write-git-config was allowed",
            ),
            (
                "the hook folder writable",
                Box::new(|role, b| {
                    if agent(role) {
                        b.write.push(
                            b.read
                                .iter()
                                .find(|p| p.ends_with("bin"))
                                .expect("hook")
                                .clone(),
                        );
                    }
                }),
                "agent: write-hook was allowed",
            ),
            (
                "another host allowed",
                Box::new(|role, b| {
                    if agent(role) {
                        b.domains.push("example.com".into());
                    }
                }),
                "agent: reach-other-host was allowed",
            ),
            (
                "a check with network",
                Box::new(|role, b| {
                    if matches!(role, Role::Check { .. }) {
                        b.domains.push("example.com".into());
                    }
                }),
                "check: reach-other-host was allowed",
            ),
            (
                "the provider not listed",
                Box::new(|role, b| {
                    if agent(role) {
                        b.domains = vec!["example.com".into()];
                    }
                }),
                "agent: reach-provider was denied but must work",
            ),
            (
                "the key not masked",
                Box::new(|_, b| b.mask_env.clear()),
                "agent: key-is-masked was denied but must work",
            ),
            (
                "the home folder readable",
                Box::new(|role, b| {
                    if agent(role) {
                        let home = std::env::var("HOME").expect("HOME");
                        b.read.push(PathBuf::from(home).join(".autopilot"));
                    }
                }),
                "agent: read-home-file was allowed",
            ),
        ];
        for (what, tweak, expected) in cases {
            let failures = failures_with(&*tweak);
            assert!(
                failures.iter().any(|f| f == expected),
                "{what}: expected {expected:?} in {failures:?}"
            );
        }
    }

    // Invariant: with no boundary at all the rows that only the sandbox can
    // refuse (loopback, a Unix socket, a write through a system service) are
    // allowed, so they are measuring the boundary and not a broken setup.
    #[test]
    #[ignore = "needs the sandbox tools"]
    fn without_a_boundary_the_service_and_socket_rows_fail() {
        let directory = tempfile::tempdir_in(std::env::var_os("HOME").expect("HOME")).expect("dir");
        let wsl = mac_wsl(directory.path());
        let scene = Scene::build(&wsl, directory.path()).expect("scene");
        let listeners = Listeners::start(&scene).expect("listeners");
        let role = scene.roles()[0].clone();
        let spec = probe_spec(&wsl, &scene, &listeners, &role);
        let finished = run(&wsl, &spec, Duration::from_secs(60)).expect("runs");
        let failures = judge(&finished.stdout, "agent", true, &["read-ssh-folder"]);
        for row in [
            "connect-loopback",
            "connect-unix-socket",
            "write-prefs-by-service",
            "read-home-file",
            "write-sibling",
        ] {
            let expected = format!("agent: {row} was allowed");
            assert!(failures.contains(&expected), "{row}: {failures:?}");
        }
        let _ = fs::remove_file(scene.prefs_file(&wsl));
    }
}
