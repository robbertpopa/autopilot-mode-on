use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use autopilot_process::{CommandSpec, Limits, Outcome, Runner};
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
}

pub(crate) struct Executed {
    pub success: bool,
    /// A headline plus the tail of the output on failure.
    pub note: String,
    /// All captured output, for a check that looks for a name in it.
    pub text: String,
}

impl Toolbox {
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
        })
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
            cwd: Some(context.worktree.to_path_buf()),
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
