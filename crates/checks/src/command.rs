use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use autopilot_process::{
    CommandSpec, Limits, Outcome, Runner,
    isolation::{Role, Validated},
};
use autopilot_profile::Profile;

use crate::{Check, Context, Error, Report, tail};

const TIMEOUT: Duration = Duration::from_secs(600);
const BYTES_PER_STREAM: usize = 4 * 1024 * 1024;
const NOTE_TAIL: usize = 2000;

/// Variables a build or test tool needs to find its toolchain. Nothing else is
/// passed on, so a token in the parent environment never reaches a check.
const ALLOWED: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "TEMP",
    "TMP",
    "TMPDIR",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "CARGO_HOME",
    "RUSTUP_HOME",
];

#[derive(Clone, Debug, Default)]
pub struct Environment(Vec<(OsString, OsString)>);

impl Environment {
    /// Keep only the allowed variables, matching names the way the OS does.
    #[must_use]
    pub fn from_vars(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        let mut kept: Vec<(OsString, OsString)> = vars
            .into_iter()
            .filter(|(name, _)| {
                name.to_str().is_some_and(|name| {
                    ALLOWED.iter().any(|allowed| {
                        if cfg!(windows) {
                            allowed.eq_ignore_ascii_case(name)
                        } else {
                            *allowed == name
                        }
                    })
                })
            })
            .collect();
        kept.push(("CARGO_TERM_COLOR".into(), "never".into()));
        kept.push(("NO_COLOR".into(), "1".into()));
        Self(kept)
    }
}

/// Find an executable by name in `path_var`. A name with a separator is only
/// accepted as an absolute path: the profile names tools, not locations
/// relative to the project.
#[must_use]
pub fn resolve_executable(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) {
        let path = Path::new(name);
        return (path.is_absolute() && path.is_file()).then(|| path.to_path_buf());
    }
    // The runner cannot start a script through a shell, so only real programs count.
    let file = if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::split_paths(path_var)
        .map(|directory| directory.join(&file))
        .find(|candidate| candidate.is_absolute() && candidate.is_file())
}

/// The spelling of a directory that build tools cope with. Managed worktrees
/// are canonical, which on Windows means `\\?\C:\...`, and the MinGW linker
/// cannot find its object files from a working directory in that form.
#[must_use]
pub fn plain_directory(path: &Path) -> PathBuf {
    if cfg!(windows)
        && let Some(text) = path.to_str()
        && let Some(rest) = text.strip_prefix(r"\\?\")
        // Only a drive path. A verbatim UNC path means something else.
        && rest.as_bytes().get(1..3) == Some(b":\\".as_slice())
        && rest.len() < 240
    {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

struct Launch {
    program: PathBuf,
    args: Vec<String>,
}

/// The commands the profile names, resolved once. A command whose executable
/// cannot be found stays in the box as an error, so its check fails with a
/// reason instead of the whole run refusing to start.
pub struct Toolbox {
    commands: BTreeMap<String, Result<Launch, String>>,
    runner: Arc<Runner>,
    environment: Environment,
    /// When set, every command runs inside the validated Linux boundary.
    sandbox: Option<Sandbox>,
}

struct Sandbox {
    boundary: Validated,
    /// Scratch space for the commands; outside the worktree, so it never shows
    /// up in a diff.
    tmp: PathBuf,
}

pub(crate) struct Executed {
    pub success: bool,
    /// A headline plus the tail of the output on failure.
    pub note: String,
    /// All captured output, for a check that looks for a name in it.
    pub text: String,
}

impl Toolbox {
    /// Like [`Toolbox::from_profile`], building the runner from the process
    /// guardian so a caller above this layer never touches `process`.
    #[must_use]
    pub fn with_guardian(
        profile: &Profile,
        guardian: PathBuf,
        environment: Environment,
        path_var: &OsStr,
    ) -> Arc<Self> {
        Self::from_profile(
            profile,
            Arc::new(Runner::new(guardian)),
            environment,
            path_var,
        )
    }

    #[must_use]
    pub fn from_profile(
        profile: &Profile,
        runner: Arc<Runner>,
        environment: Environment,
        path_var: &OsStr,
    ) -> Arc<Self> {
        let commands = profile
            .checks
            .iter()
            .map(|(name, command)| {
                let launch = resolve_executable(&command.executable, path_var)
                    .map(|program| Launch {
                        program,
                        args: command.args.clone(),
                    })
                    .ok_or_else(|| format!("executable {:?} was not found", command.executable));
                (name.clone(), launch)
            })
            .collect();
        Arc::new(Self {
            commands,
            runner,
            environment,
            sandbox: None,
        })
    }

    /// Like [`Toolbox::sandboxed`], building the runner from the process
    /// guardian so a caller above this layer never touches `process`.
    #[must_use]
    pub fn sandboxed_with_guardian(
        profile: &Profile,
        guardian: PathBuf,
        boundary: Validated,
        tmp: PathBuf,
    ) -> Arc<Self> {
        Self::sandboxed(profile, Arc::new(Runner::new(guardian)), boundary, tmp)
    }

    /// A toolbox whose commands run inside the validated Linux boundary, with
    /// no network. Commands are looked up on the distro's `PATH`, so the
    /// profile may name only plain tools (`cargo`), not Windows locations.
    #[must_use]
    pub fn sandboxed(
        profile: &Profile,
        runner: Arc<Runner>,
        boundary: Validated,
        tmp: PathBuf,
    ) -> Arc<Self> {
        let commands = profile
            .checks
            .iter()
            .map(|(name, command)| {
                let launch = if command.executable.is_empty()
                    || command.executable.contains(['/', '\\', ':'])
                {
                    Err(format!(
                        "executable {:?} must be a plain tool name in the sandbox",
                        command.executable
                    ))
                } else {
                    Ok(Launch {
                        program: PathBuf::from(&command.executable),
                        args: command.args.clone(),
                    })
                };
                (name.clone(), launch)
            })
            .collect();
        // The registry cache is filled by `prepare`; nothing here may reach the
        // network, so cargo should fail at once instead of retrying.
        let environment = Environment(
            [
                ("CARGO_NET_OFFLINE", "true"),
                ("CARGO_TERM_COLOR", "never"),
                ("NO_COLOR", "1"),
                ("LANG", "C.UTF-8"),
            ]
            .map(|(name, value)| (name.into(), value.into()))
            .to_vec(),
        );
        Arc::new(Self {
            commands,
            runner,
            environment,
            sandbox: Some(Sandbox { boundary, tmp }),
        })
    }

    /// Fetch what a project depends on, once per round, before the checks run
    /// without a network. Each fetch runs under a role that reaches only its
    /// own registry: crates.io for a project with a `Cargo.toml`, the npm
    /// registry for one with a `package-lock.json`. A Rust fetch updates
    /// `Cargo.lock` when the agent added a dependency, because the agent has no
    /// toolchain to do it. `npm ci` runs with install scripts off, so no
    /// package's own code runs; it installs exactly what the lockfile says.
    /// Any other kind of project is left alone: a Python install builds source
    /// packages by running their code, which is not something to do on trust.
    ///
    /// # Errors
    /// Returns the tail of the output if a fetch fails or cannot start.
    pub fn prepare(
        &self,
        worktree: &Path,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(), String> {
        let Some(sandbox) = &self.sandbox else {
            return Ok(());
        };
        let worktree = plain_directory(worktree);
        if worktree.join("Cargo.toml").is_file() {
            let role = Role::Fetch {
                worktree: worktree.clone(),
                tmp: sandbox.tmp.clone(),
            };
            let env = [("CARGO_TERM_COLOR", "never"), ("LANG", "C.UTF-8")];
            self.fetch(sandbox, &role, "cargo", &["fetch"], &env, cancel)?;
        }
        if worktree.join("package-lock.json").is_file() {
            let role = Role::FetchNode {
                worktree,
                tmp: sandbox.tmp.clone(),
            };
            let env = [("LANG", "C.UTF-8"), ("NO_COLOR", "1")];
            let args = ["ci", "--ignore-scripts", "--no-audit", "--no-fund"];
            self.fetch(sandbox, &role, "npm", &args, &env, cancel)?;
        }
        Ok(())
    }

    fn fetch(
        &self,
        sandbox: &Sandbox,
        role: &Role,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(), String> {
        let worktree = match role {
            Role::Fetch { worktree, .. } | Role::FetchNode { worktree, .. } => worktree.clone(),
            Role::Agent { .. } | Role::Check { .. } => {
                return Err("only a fetch role may fetch".to_owned());
            }
        };
        let what = format!("{program} {}", args.first().copied().unwrap_or_default());
        let spec = CommandSpec {
            program: PathBuf::from(program),
            args: args.iter().map(OsString::from).collect(),
            env: env
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
            cwd: Some(worktree),
        };
        let wrapped = sandbox
            .boundary
            .wrap(role, &spec, &[])
            .map_err(|error| format!("{what} could not start: {error}"))?;
        let limits = Limits {
            timeout: TIMEOUT,
            bytes_per_stream: BYTES_PER_STREAM,
        };
        let output = self
            .runner
            .run(&wrapped, b"", limits, cancel)
            .map_err(|error| format!("{what} could not start: {error}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        match output.outcome {
            Outcome::Exited(status) if status.success() => Ok(()),
            _ => Err(format!(
                "{what} failed. Last output:\n{}",
                tail(text.trim_end(), NOTE_TAIL)
            )),
        }
    }

    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// The check that runs the profile command `name`, if the profile has it.
    #[must_use]
    pub fn check(self: &Arc<Self>, name: &str) -> Option<CommandCheck> {
        self.has(name).then(|| CommandCheck {
            toolbox: Arc::clone(self),
            name: name.to_owned(),
        })
    }

    pub(crate) fn configuration(&self, name: &str) -> String {
        match self.commands.get(name) {
            Some(Ok(launch)) => format!("{}\0{:?}", launch.program.display(), launch.args),
            Some(Err(reason)) => reason.clone(),
            None => String::new(),
        }
    }

    pub(crate) fn all_configuration(&self) -> String {
        self.commands
            .keys()
            .map(|name| format!("{name}={}", self.configuration(name)))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    }

    /// Run a profile command with extra arguments appended. `Ok(None)` means
    /// the profile has no such command.
    pub(crate) fn execute(
        &self,
        name: &str,
        extra: &[String],
        context: &Context<'_>,
    ) -> Result<Option<Executed>, Error> {
        let launch = match self.commands.get(name) {
            None => return Ok(None),
            Some(Err(reason)) => {
                return Ok(Some(Executed {
                    success: false,
                    note: format!("{name}: {reason}"),
                    text: String::new(),
                }));
            }
            Some(Ok(launch)) => launch,
        };
        let spec = CommandSpec {
            program: launch.program.clone(),
            args: launch
                .args
                .iter()
                .chain(extra)
                .map(OsString::from)
                .collect(),
            env: self.environment.0.clone(),
            cwd: Some(plain_directory(context.worktree)),
        };
        let spec = match &self.sandbox {
            None => spec,
            Some(sandbox) => {
                let role = Role::Check {
                    worktree: plain_directory(context.worktree),
                    tmp: sandbox.tmp.clone(),
                };
                sandbox
                    .boundary
                    .wrap(&role, &spec, &[])
                    .map_err(|error| Error::Launch {
                        check: name.to_owned(),
                        reason: error.to_string(),
                    })?
            }
        };
        let limits = Limits {
            timeout: TIMEOUT,
            bytes_per_stream: BYTES_PER_STREAM,
        };
        let output = self
            .runner
            .run(&spec, b"", limits, context.cancel)
            .map_err(|error| Error::Launch {
                check: name.to_owned(),
                reason: error.to_string(),
            })?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        let (success, headline) = match output.outcome {
            Outcome::Exited(status) if status.success() => (true, format!("{name} passed")),
            Outcome::Exited(status) => (
                false,
                match status.code() {
                    Some(code) => format!("{name} failed with exit code {code}"),
                    None => format!("{name} was stopped by a signal"),
                },
            ),
            Outcome::TimedOut => (
                false,
                format!("{name} timed out after {} seconds", TIMEOUT.as_secs()),
            ),
            Outcome::OutputLimit => (false, format!("{name} printed more than it may")),
            Outcome::Cancelled => {
                return Err(Error::Cancelled {
                    check: name.to_owned(),
                });
            }
        };
        let shown = tail(text.trim_end(), NOTE_TAIL);
        let note = if success || shown.is_empty() {
            headline
        } else {
            format!("{headline}. Last output:\n{shown}")
        };
        Ok(Some(Executed {
            success,
            note,
            text,
        }))
    }
}

/// One profile command run as a check: it passes when the command exits 0.
pub struct CommandCheck {
    toolbox: Arc<Toolbox>,
    name: String,
}

impl Check for CommandCheck {
    fn id(&self) -> &str {
        &self.name
    }

    fn applies(&self, _: &Context<'_>) -> bool {
        self.toolbox.has(&self.name)
    }

    fn can_refine(&self) -> bool {
        true
    }

    fn configuration(&self) -> String {
        self.toolbox.configuration(&self.name)
    }

    fn run(&self, context: &Context<'_>) -> Result<Report, Error> {
        let executed = self
            .toolbox
            .execute(&self.name, &[], context)?
            .ok_or_else(|| Error::Launch {
                check: self.name.clone(),
                reason: "the profile no longer has this command".into(),
            })?;
        Ok(if executed.success {
            Report::pass(executed.note)
        } else {
            Report::fail(executed.note)
        })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::path::Path;

    use super::plain_directory;

    // Invariant: a canonical worktree path reaches the build tool as a plain
    // drive path. The MinGW linker fails from a `\\?\` working directory, and
    // managed worktrees are always canonical, so only a real cargo build
    // would otherwise catch this.
    #[test]
    fn a_verbatim_drive_path_is_simplified_and_other_paths_are_left_alone() {
        assert_eq!(
            plain_directory(Path::new(r"\\?\C:\work\tree")),
            Path::new(r"C:\work\tree")
        );
        assert_eq!(
            plain_directory(Path::new(r"C:\work\tree")),
            Path::new(r"C:\work\tree")
        );
        let unc = Path::new(r"\\?\UNC\server\share\tree");
        assert_eq!(plain_directory(unc), unc);
    }
}
