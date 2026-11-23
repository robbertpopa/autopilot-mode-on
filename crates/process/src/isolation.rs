//! The only way to get a [`Validated`] boundary is [`validate`], which runs an
//! escape probe through the same wrapper and settings a real launch uses and
//! fails closed: if any row behaves differently from what was expected, there
//! is no value to hand to the code that starts an agent or a check.
//!
//! The rows are the S6 matrix (`docs/spikes/S6-containment.md`). Writes and
//! reads outside the allowed folders, the Git metadata, a listener on the
//! distro's loopback, a Windows program reached through WSL interop and the
//! interop socket are each tried from inside the boundary. The host then looks
//! at the files, because a command that reports "denied" proves nothing if the
//! change landed anyway.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use crate::{
    CommandSpec, Limits, Outcome, Runner,
    wsl::{Boundary, CARGO_CREDENTIALS, NODE_ROOTS, RUST_ROOTS, Wsl},
};

/// The one host an agent may reach.
pub const PROVIDER: &str = "api.anthropic.com";

/// What a sandboxed command is for. Each role opens a different set of folders.
#[derive(Clone, Debug)]
pub enum Role {
    /// The agent that edits a worktree. It reaches the provider and nothing else.
    Agent { worktree: PathBuf, config: PathBuf },
    /// A check command (build, test) in a worktree. No network at all.
    Check { worktree: PathBuf, tmp: PathBuf },
}

/// A boundary that passed the probe on this machine. It cannot be built any
/// other way.
#[derive(Clone, Debug)]
pub struct Validated {
    wsl: Arc<Wsl>,
}

impl Validated {
    /// Wrap `spec` for `role`. The folders the role writes are created first,
    /// because the sandbox cannot bind a folder that does not exist.
    ///
    /// # Errors
    /// Returns an error if a folder cannot be made or the wrapper refuses.
    pub fn wrap(
        &self,
        role: &Role,
        spec: &CommandSpec,
        secrets: &[&str],
    ) -> io::Result<CommandSpec> {
        let boundary = boundary(&self.wsl, role);
        for folder in boundary.write.iter().chain(&boundary.tmp) {
            fs::create_dir_all(folder)?;
        }
        self.wsl.wrap(spec, &boundary, secrets)
    }

    #[must_use]
    pub fn wsl(&self) -> &Wsl {
        &self.wsl
    }
}

/// The folders, network and protected paths of each role. One function, used by
/// both a real launch and the probe.
pub(crate) fn boundary(wsl: &Wsl, role: &Role) -> Boundary {
    match role {
        Role::Agent { worktree, config } => Boundary {
            write: vec![worktree.clone(), config.clone()],
            read: NODE_ROOTS.iter().map(|root| wsl.under_home(root)).collect(),
            protect: vec![worktree.join(".git")],
            hide: Vec::new(),
            domains: vec![PROVIDER.to_owned()],
            tmp: Some(config.join("tmp")),
        },
        Role::Check { worktree, tmp } => Boundary {
            write: vec![worktree.clone()],
            read: RUST_ROOTS.iter().map(|root| wsl.under_home(root)).collect(),
            protect: vec![worktree.join(".git")],
            hide: vec![wsl.under_home(CARGO_CREDENTIALS)],
            domains: Vec::new(),
            tmp: Some(tmp.clone()),
        },
    }
}

/// Run the probe for every role and return the boundary only if all of it held.
///
/// # Errors
/// Returns an error naming each row that did not behave as expected, or the
/// setup step that failed. The boundary is then unusable.
pub fn validate(wsl: Wsl, work: &Path) -> io::Result<Validated> {
    let wsl = Arc::new(wsl);
    let scene = Scene::build(work)?;
    let listener = Listener::start(&wsl, &scene)?;
    let mut failures = Vec::new();
    for role in scene.roles() {
        failures.extend(probe(&wsl, &scene, &role, &listener)?);
    }
    drop(listener);
    if failures.is_empty() {
        Ok(Validated { wsl })
    } else {
        Err(io::Error::other(format!(
            "the WSL boundary failed {} check(s): {}",
            failures.len(),
            failures.join("; ")
        )))
    }
}

/// A disposable layout on the drive the product really uses: a worktree with a
/// `.git` folder, a sibling with a marker file, and a Windows program inside
/// the worktree for the interop row.
struct Scene {
    worktree: PathBuf,
    sibling: PathBuf,
    config: PathBuf,
    tmp: PathBuf,
    hit: PathBuf,
}

const MARKER: &str = "SYNTHETIC-MARKER\n";
const GIT_CONFIG: &str = "[core]\n\tbare = false\n";

impl Scene {
    fn build(work: &Path) -> io::Result<Self> {
        let root = work.join("isolation-check");
        match fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let scene = Self {
            worktree: root.join("wt"),
            sibling: root.join("sibling"),
            config: root.join("config"),
            tmp: root.join("tmp"),
            hit: root.join("listener-hit.txt"),
        };
        fs::create_dir_all(scene.worktree.join(".git").join("hooks"))?;
        fs::create_dir_all(&scene.sibling)?;
        fs::write(scene.worktree.join(".git").join("config"), GIT_CONFIG)?;
        fs::write(scene.sibling.join("marker.txt"), MARKER)?;
        if let Some(system) = std::env::var_os("SystemRoot") {
            let cmd = Path::new(&system).join("System32").join("cmd.exe");
            if cmd.is_file() {
                fs::copy(cmd, scene.worktree.join("cmd.exe"))?;
            }
        }
        Ok(scene)
    }

    fn roles(&self) -> [Role; 2] {
        [
            Role::Agent {
                worktree: self.worktree.clone(),
                config: self.config.clone(),
            },
            Role::Check {
                worktree: self.worktree.clone(),
                tmp: self.tmp.clone(),
            },
        ]
    }
}

/// A TCP listener on the distro's own loopback, started outside the boundary.
/// It records a hit in a file, so the host can tell whether a sandboxed
/// command reached it.
struct Listener {
    child: Child,
    port: u16,
}

const LISTEN: &str = "import socket, sys
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', int(sys.argv[1])))
s.listen(5)
while True:
    c, _ = s.accept()
    open(sys.argv[2], 'a').write('hit\\n')
    c.close()
";

const CONNECT: &str = "import socket, sys
s = socket.create_connection(('127.0.0.1', int(sys.argv[1])), timeout=5)
s.close()
";

impl Listener {
    fn start(wsl: &Wsl, scene: &Scene) -> io::Result<Self> {
        let port = u16::try_from(40_000 + std::process::id() % 20_000).unwrap_or(41_000);
        let hit = crate::wsl::linux_path(&scene.hit)?;
        let child = Command::new(wsl.launcher())
            .args([
                "-d",
                wsl.distro(),
                "--exec",
                "/usr/bin/python3",
                "-c",
                LISTEN,
            ])
            .arg(port.to_string())
            .arg(&hit)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let listener = Self { child, port };
        // Control: the same connection from outside the boundary must reach the
        // listener, or a denied row below would prove nothing.
        let mut reached = false;
        for _ in 0..20 {
            let spec = CommandSpec {
                program: wsl.launcher().to_path_buf(),
                args: [
                    "-d",
                    wsl.distro(),
                    "--exec",
                    "/usr/bin/python3",
                    "-c",
                    CONNECT,
                ]
                .iter()
                .map(OsString::from)
                .chain([OsString::from(port.to_string())])
                .collect(),
                env: crate::wsl::passthrough_env(),
                cwd: None,
            };
            if run(&spec, Duration::from_secs(20))?.success && scene.hit.is_file() {
                reached = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !reached {
            return Err(io::Error::other(
                "the control connection to the test listener failed, so the loopback row cannot be trusted",
            ));
        }
        fs::remove_file(&scene.hit)?;
        Ok(listener)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // A listener left running would hold its port for the next check.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Finished {
    success: bool,
    stdout: String,
    stderr: String,
}

fn run(spec: &CommandSpec, timeout: Duration) -> io::Result<Finished> {
    let limits = Limits {
        timeout,
        bytes_per_stream: 256 * 1024,
    };
    let output = Runner::new(PathBuf::new()).run(spec, b"", limits, &AtomicBool::new(false))?;
    Ok(Finished {
        success: matches!(&output.outcome, Outcome::Exited(status) if status.success()),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Each line is `ALLOWED label` or `DENIED label`. Positional parameters:
/// worktree, sibling, port, scratch folder, the sibling as a Windows path.
const PROBE: &str = r#"w=$1; s=$2; port=$3; tmp=$4; win=$5
r() { if timeout 20 "$@" </dev/null >/dev/null 2>&1; then echo "ALLOWED $LABEL"; else echo "DENIED $LABEL"; fi; }
LABEL=write-worktree r bash -c 'echo x > "$0/out.txt"' "$w"
LABEL=write-scratch r bash -c 'echo x > "$0/out.txt"' "$tmp"
LABEL=write-sibling r bash -c 'echo x > "$0/new.txt"' "$s"
LABEL=read-sibling r cat "$s/marker.txt"
LABEL=read-windows-drive r cat /mnt/c/Windows/win.ini
LABEL=read-home r cat "$HOME/.profile"
LABEL=write-git-config r bash -c 'echo "[x]" >> "$0/.git/config"' "$w"
LABEL=write-git-hook r bash -c 'echo x > "$0/.git/hooks/pre-push"' "$w"
LABEL=connect-loopback r bash -c 'exec 3<>/dev/tcp/127.0.0.1/$0' "$port"
LABEL=run-windows-program r "$w/cmd.exe" /c "echo x > $win\\pwned.txt"
LABEL=run-init r /init
LABEL=unix-socket r python3 -c 'import socket; socket.socket(socket.AF_UNIX)'
"#;

const EXPECTED_ALLOWED: &[&str] = &["write-worktree", "write-scratch"];
const ROWS: &[&str] = &[
    "write-worktree",
    "write-scratch",
    "write-sibling",
    "read-sibling",
    "read-windows-drive",
    "read-home",
    "write-git-config",
    "write-git-hook",
    "connect-loopback",
    "run-windows-program",
    "run-init",
    "unix-socket",
];

fn probe(wsl: &Wsl, scene: &Scene, role: &Role, listener: &Listener) -> io::Result<Vec<String>> {
    let name = match role {
        Role::Agent { .. } => "agent",
        Role::Check { .. } => "check",
    };
    let scratch = match role {
        Role::Agent { config, .. } => config.join("tmp"),
        Role::Check { tmp, .. } => tmp.clone(),
    };
    let sibling_windows = scene
        .sibling
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    let spec = CommandSpec {
        program: PathBuf::from("/usr/bin/bash"),
        args: vec![
            OsString::from("-c"),
            OsString::from(PROBE),
            OsString::from("probe"),
            crate::wsl::linux_path(&scene.worktree)?.into(),
            crate::wsl::linux_path(&scene.sibling)?.into(),
            listener.port.to_string().into(),
            crate::wsl::linux_path(&scratch)?.into(),
            sibling_windows.into(),
        ],
        env: Vec::new(),
        cwd: Some(scene.worktree.clone()),
    };
    let validated = Validated {
        wsl: Arc::new(wsl.clone()),
    };
    let wrapped = validated.wrap(role, &spec, &[])?;
    let finished = run(&wrapped, Duration::from_secs(180))?;
    let mut failures = Vec::new();
    if !finished.success {
        failures.push(format!(
            "{name}: the probe did not run ({})",
            finished.stderr.trim()
        ));
        return Ok(failures);
    }
    failures.extend(judge(&finished.stdout, name));
    failures.extend(host_findings(scene, name));
    Ok(failures)
}

/// Compare the probe's answers with what each row must be. A row with no
/// answer fails: a probe that died half way must not count as a pass.
fn judge(stdout: &str, name: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for row in ROWS {
        let must_work = EXPECTED_ALLOWED.contains(row);
        let answer = stdout.lines().find_map(|line| {
            let (verdict, label) = line.split_once(' ')?;
            (label == *row).then_some(verdict)
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

/// What the host sees afterwards. A row that reported "denied" must also have
/// changed nothing.
fn host_findings(scene: &Scene, name: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut check = |bad: bool, text: &str| {
        if bad {
            found.push(format!("{name}: {text}"));
        }
    };
    let git_config = scene.worktree.join(".git").join("config");
    check(
        fs::read_to_string(git_config).ok().as_deref() != Some(GIT_CONFIG),
        "the Git config changed",
    );
    check(
        fs::read_to_string(scene.sibling.join("marker.txt"))
            .ok()
            .as_deref()
            != Some(MARKER),
        "the sibling marker changed",
    );
    for landed in [
        scene.sibling.join("new.txt"),
        scene.sibling.join("pwned.txt"),
        scene.worktree.join(".git").join("hooks").join("pre-push"),
        scene.hit.clone(),
    ] {
        check(landed.exists(), &format!("{} exists", landed.display()));
    }
    found
}

#[cfg(test)]
mod tests;
