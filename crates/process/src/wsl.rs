//! Run a command inside the Linux boundary: WSL2 with Anthropic's sandbox
//! runtime (`srt`). The boundary is bubblewrap with no network namespace, a
//! seccomp filter that refuses Unix sockets, and a proxy that passes only the
//! domains listed in the settings.
//!
//! [`Wsl::wrap`] turns a command into a `wsl.exe` command that the normal
//! [`Runner`](crate::Runner) supervises. The settings file for `srt` comes from
//! one function, [`Boundary::settings`], so what a launch runs under is what the
//! validation in `crate::isolation` ran under.

use std::{
    collections::hash_map::DefaultHasher,
    ffi::OsString,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
};

use crate::{CommandSpec, Limits, Outcome, Runner};

/// Runs inside the distro with every value as a positional parameter, so no
/// path, argument or value is ever pasted into the script text. Secrets arrive
/// by `WSLENV` and are read by name here, which keeps them out of every
/// command line in the distro.
const LAUNCH: &str = r#"set -eu
srt=$1; settings=$2; cwd=$3; names=$4; tmp=$5; shift 5
vars=()
while [ "$1" != "--" ]; do vars+=("$1"); shift; done
shift
for n in $names; do vars+=("$n=${!n}"); done
# srt points TMPDIR at a folder that does not exist; a command that needs
# scratch space (rustdoc) fails without a writable one.
inner="cd $(printf '%q' "$cwd") && ${tmp:+export TMPDIR=$(printf '%q' "$tmp") &&} exec $(printf '%q ' "$@")"
exec /usr/bin/env -i "${vars[@]}" "$srt" -s "$settings" -c "$inner"
"#;

/// What `wsl.exe` itself needs from the Windows side to find the distro.
const WINDOWS_PASSTHROUGH: &[&str] = &[
    "SYSTEMROOT",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    // A Linux host finds its tools under the user's home.
    "HOME",
];

/// Prefixes under the distro's home that hold tools the sandboxed command may
/// need to read. A caller picks the ones its role needs.
pub const NODE_ROOTS: &[&str] = &[".npm-global", "node"];
pub const RUST_ROOTS: &[&str] = &[".cargo", ".rustup"];

/// Folders hidden from every sandboxed command unless a boundary opens a path
/// inside them.
const HIDDEN_ROOTS: &[&str] = &["/mnt", "/media", "/home", "/root", "/srv"];

/// A file under a tool prefix that holds a credential. It is hidden even when
/// its prefix is readable.
pub const CARGO_CREDENTIALS: &str = ".cargo/credentials.toml";

/// Where the boundary runs: in a WSL2 distro reached through `wsl.exe` on
/// Windows, or directly on a Linux host. macOS is not supported: Seatbelt has
/// no network namespace and treats loopback and Unix sockets differently, so
/// the probe's rows would mean something else and have not been written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Host {
    Wsl,
    Linux,
}

#[derive(Clone, Debug)]
pub struct Wsl {
    host: Host,
    distro: String,
    launcher: PathBuf,
    /// The supervisor binary a Linux host needs for the `Runner`; Windows has none.
    guardian: PathBuf,
    srt: String,
    path: String,
    home: String,
    /// Where the seccomp helper lives. The command must be able to start it.
    seccomp: String,
    /// The installed deny hook, a Linux program an agent runs before each shell
    /// call. It sits in a folder the agent can read but not write.
    hook: String,
    settings_dir: PathBuf,
}

/// What one command may touch. Paths are Windows paths (`D:\...`) or absolute
/// Linux paths; they are translated when the settings are written.
#[derive(Clone, Debug, Default)]
pub struct Boundary {
    /// Folders the command may read and write.
    pub write: Vec<PathBuf>,
    /// Folders it may only read.
    pub read: Vec<PathBuf>,
    /// Paths inside a writable folder it may not change (`.git`).
    pub protect: Vec<PathBuf>,
    /// Paths it may not read even inside a readable folder.
    pub hide: Vec<PathBuf>,
    /// Hosts the command may reach. Empty means no network at all.
    pub domains: Vec<String>,
    /// Scratch folder, made writable and exported as `TMPDIR`.
    pub tmp: Option<PathBuf>,
}

impl Wsl {
    /// Find the distro's tools. This starts `wsl.exe` once and changes nothing.
    ///
    /// # Errors
    /// Returns an error if the distro cannot start or lacks `srt`,
    /// `bubblewrap`, `socat`, `ripgrep` or `python3`, naming what is missing.
    pub fn discover(distro: &str, settings_dir: PathBuf) -> io::Result<Self> {
        check_distro_name(distro)?;
        let launcher = windows_wsl()?;
        let args = ["-d", distro, "--exec", "/usr/bin/bash", "-c", DISCOVER];
        let guardian = PathBuf::new();
        let text = ask(
            &launcher,
            &args,
            &format!("the WSL distro {distro}"),
            &guardian,
        )?;
        parse_discovery(&text, Host::Wsl, distro, launcher, guardian, settings_dir)
    }

    /// Find the tools on this Linux machine, where the boundary runs directly.
    ///
    /// # Errors
    /// Returns an error on any other system, or if a tool is missing.
    pub fn discover_local(settings_dir: PathBuf, guardian: PathBuf) -> io::Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the sandbox runs on a Linux host, or through WSL2 on Windows; macOS has not been validated",
            ));
        }
        let launcher = PathBuf::from("/usr/bin/env");
        let args = ["/usr/bin/bash", "-c", DISCOVER];
        let text = ask(&launcher, &args, "this machine", &guardian)?;
        parse_discovery(
            &text,
            Host::Linux,
            "local",
            launcher,
            guardian,
            settings_dir,
        )
    }

    #[must_use]
    pub(crate) fn guardian(&self) -> &Path {
        &self.guardian
    }

    #[must_use]
    pub fn host(&self) -> Host {
        self.host
    }

    /// The program that starts something inside the boundary's machine, and
    /// the arguments that go before the command (`wsl.exe -d Ubuntu --exec`).
    #[must_use]
    pub(crate) fn entry(&self) -> (PathBuf, Vec<OsString>) {
        let prefix = match self.host {
            Host::Wsl => ["-d", &self.distro, "--exec"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            Host::Linux => Vec::new(),
        };
        (self.launcher.clone(), prefix)
    }

    /// Whether the caller can make this folder. A Linux path inside a distro
    /// cannot be made from the Windows side, but on a Linux host it can.
    #[must_use]
    pub(crate) fn can_make(&self, folder: &Path) -> bool {
        self.host == Host::Linux || !folder.to_string_lossy().starts_with('/')
    }

    /// The deny hook's path inside the boundary's machine.
    #[must_use]
    pub fn hook(&self) -> &str {
        &self.hook
    }

    #[must_use]
    pub fn home(&self) -> &str {
        &self.home
    }

    #[must_use]
    pub fn distro(&self) -> &str {
        &self.distro
    }

    /// A path under the distro's home, as the folders a role may read.
    #[must_use]
    pub fn under_home(&self, relative: &str) -> PathBuf {
        PathBuf::from(format!("{}/{relative}", self.home))
    }

    /// The `srt` settings for `boundary`. The only place they are written.
    ///
    /// # Errors
    /// Returns an error if a path cannot be translated.
    pub fn settings(&self, boundary: &Boundary) -> io::Result<String> {
        let mut write = linux_paths(&boundary.write)?;
        if let Some(tmp) = &boundary.tmp {
            write.push(linux_path(tmp)?);
        }
        let mut read = linux_paths(&boundary.read)?;
        read.extend(write.iter().cloned());
        read.push(self.seccomp.clone());
        let hide = linux_paths(&boundary.hide)?;
        let protect = linux_paths(&boundary.protect)?;
        // Where a person's own files live: other drives (`/mnt`, `/media`), every
        // home folder, root's and service data. System folders (`/usr`, `/etc`,
        // `/lib`) stay readable because programs need them, and so does `/tmp`:
        // the sandbox's own proxy socket lives there, and hiding it cuts off the
        // one host an agent may reach.
        let mut deny_read: Vec<String> =
            HIDDEN_ROOTS.iter().map(|root| (*root).to_owned()).collect();
        let covered = deny_read
            .iter()
            .any(|root| self.home == *root || self.home.starts_with(&format!("{root}/")));
        if !covered {
            deny_read.push(self.home.clone());
        }
        deny_read.extend(hide);
        let value = serde_json::json!({
            "network": {
                "allowedDomains": boundary.domains,
                "deniedDomains": [],
            },
            "filesystem": {
                "denyRead": deny_read,
                "allowRead": read,
                "allowWrite": write,
                "denyWrite": protect,
            },
        });
        serde_json::to_string_pretty(&value).map_err(io::Error::other)
    }

    /// Wrap `spec` so it runs under `boundary`. `spec.program` is a Linux path
    /// and `spec.cwd` a Windows or Linux folder. An environment entry named in
    /// `secrets` travels by `WSLENV`, never by command line.
    ///
    /// # Errors
    /// Returns an error if a path or a variable name cannot be passed safely, or
    /// the settings file cannot be written.
    pub fn wrap(
        &self,
        spec: &CommandSpec,
        boundary: &Boundary,
        secrets: &[&str],
    ) -> io::Result<CommandSpec> {
        let cwd = spec
            .cwd
            .as_deref()
            .ok_or_else(|| invalid("a sandboxed command needs a working directory"))?;
        let program = spec
            .program
            .to_str()
            .filter(|text| text.starts_with('/') || is_program_name(text))
            .ok_or_else(|| {
                invalid("the sandboxed program must be an absolute Linux path or a plain name")
            })?;
        let settings_file = self.write_settings(boundary)?;

        let mut vars: Vec<String> =
            vec![format!("PATH={}", self.path), format!("HOME={}", self.home)];
        let mut windows_env = passthrough_env();
        for (key, value) in &spec.env {
            let key = key
                .to_str()
                .filter(|name| is_variable_name(name))
                .ok_or_else(|| invalid("an environment name is not a plain variable name"))?;
            if secrets.contains(&key) {
                windows_env.push((OsString::from(key), value.clone()));
            } else {
                let value = value
                    .to_str()
                    .ok_or_else(|| invalid("an environment value is not UTF-8"))?;
                vars.push(format!("{key}={value}"));
            }
        }
        for name in secrets {
            if !is_variable_name(name) {
                return Err(invalid("a secret name is not a plain variable name"));
            }
        }
        if !secrets.is_empty() && self.host == Host::Wsl {
            windows_env.push((OsString::from("WSLENV"), secrets.join(":").into()));
        }

        let (launcher, mut args) = self.entry();
        args.extend(["/usr/bin/bash", "-c", LAUNCH].map(OsString::from));
        args.push("ap-sandbox".into());
        args.push(self.srt.clone().into());
        args.push(linux_path(&settings_file)?.into());
        args.push(linux_path(cwd)?.into());
        args.push(secrets.join(" ").into());
        args.push(match &boundary.tmp {
            Some(tmp) => linux_path(tmp)?.into(),
            None => OsString::new(),
        });
        args.extend(vars.into_iter().map(OsString::from));
        args.push("--".into());
        args.push(program.into());
        args.extend(spec.args.iter().cloned());
        Ok(CommandSpec {
            program: launcher,
            args,
            env: windows_env,
            cwd: None,
        })
    }

    fn write_settings(&self, boundary: &Boundary) -> io::Result<PathBuf> {
        let text = self.settings(boundary)?;
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        fs::create_dir_all(&self.settings_dir)?;
        let file = self
            .settings_dir
            .join(format!("srt-{:016x}.json", hasher.finish()));
        if fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
            fs::write(&file, text)?;
        }
        Ok(file)
    }
}

/// Prints one fact per line: home, the `srt` real path, a PATH, then one
/// `missing:` line per absent tool. It reads and changes nothing.
const DISCOVER: &str = r#"home=$HOME
dirs=""
for d in "$home/node/bin" "$home/.npm-global/bin" "$home/.cargo/bin" /usr/local/bin /usr/bin /bin; do
  [ -d "$d" ] && dirs="${dirs:+$dirs:}$d"
done
export PATH="$dirs"
echo "home=$home"
echo "path=$dirs"
srt=$(command -v srt || true)
if [ -n "$srt" ]; then
  real=$(readlink -f "$srt")
  echo "srt=$srt"
  echo "seccomp=${real%/dist/*}/vendor/seccomp"
fi
for tool in srt bwrap socat rg python3 node; do
  command -v "$tool" >/dev/null || echo "missing:$tool"
done
if [ -x "$home/.autopilot/bin/autopilot-hook" ]; then
  echo "hook=$home/.autopilot/bin/autopilot-hook"
else
  echo "missing:autopilot-hook (run tools/wsl/install-hook.sh)"
fi
"#;

pub(crate) fn parse_discovery(
    text: &str,
    host: Host,
    distro: &str,
    launcher: PathBuf,
    guardian: PathBuf,
    settings_dir: PathBuf,
) -> io::Result<Wsl> {
    let mut home = None;
    let mut path = None;
    let mut srt = None;
    let mut seccomp = None;
    let mut hook = None;
    let mut missing = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("home=") {
            home = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("path=") {
            path = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("srt=") {
            srt = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("seccomp=") {
            seccomp = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("hook=") {
            hook = Some(value.to_owned());
        } else if let Some(tool) = line.strip_prefix("missing:") {
            missing.push(tool.to_owned());
        }
    }
    let place = match host {
        Host::Wsl => format!("the WSL distro {distro}"),
        Host::Linux => "this machine".to_owned(),
    };
    if !missing.is_empty() {
        return Err(io::Error::other(format!(
            "{place} is missing: {}",
            missing.join(", ")
        )));
    }
    let (Some(home), Some(path), Some(srt), Some(seccomp), Some(hook)) =
        (home, path, srt, seccomp, hook)
    else {
        return Err(io::Error::other(format!(
            "{place} gave an incomplete answer"
        )));
    };
    for value in [&home, &path, &srt, &seccomp, &hook] {
        if value.is_empty() || value.contains(['\n', '\0']) {
            return Err(io::Error::other(format!("{place} gave an unusable answer")));
        }
    }
    Ok(Wsl {
        host,
        distro: distro.to_owned(),
        launcher,
        guardian,
        srt,
        path,
        home,
        seccomp,
        hook,
        settings_dir,
    })
}

/// Run a discovery command and return what it printed.
fn ask(program: &Path, args: &[&str], what: &str, guardian: &Path) -> io::Result<String> {
    let spec = CommandSpec {
        program: program.to_path_buf(),
        args: args.iter().map(OsString::from).collect(),
        env: passthrough_env(),
        cwd: None,
    };
    let limits = Limits {
        timeout: std::time::Duration::from_secs(60),
        bytes_per_stream: 64 * 1024,
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let output = Runner::new(guardian.to_path_buf()).run(&spec, b"", limits, &cancel)?;
    if !matches!(&output.outcome, Outcome::Exited(status) if status.success()) {
        return Err(io::Error::other(format!(
            "{what} did not answer: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn windows_wsl() -> io::Result<PathBuf> {
    let root = std::env::var_os("SystemRoot").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "WSL needs Windows (no SystemRoot)",
        )
    })?;
    let exe = Path::new(&root).join("System32").join("wsl.exe");
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "wsl.exe was not found; install WSL 2",
        ))
    }
}

pub(crate) fn passthrough_env() -> Vec<(OsString, OsString)> {
    WINDOWS_PASSTHROUGH
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(*name), value)))
        .collect()
}

fn check_distro_name(name: &str) -> io::Result<()> {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if plain {
        Ok(())
    } else {
        Err(invalid("a WSL distro name has letters, digits, - _ and ."))
    }
}

/// A bare program name, looked up on the distro's `PATH` (`cargo`, `npm`).
fn is_program_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_owned())
}

fn linux_paths(paths: &[PathBuf]) -> io::Result<Vec<String>> {
    paths.iter().map(PathBuf::as_path).map(linux_path).collect()
}

/// Translate `D:\a\b` to `/mnt/d/a/b`. A path that is already absolute and
/// Linux-shaped passes through. Anything relative, `..`-bearing, UNC or with a
/// control character is refused, because a path that escapes its folder would
/// widen the boundary.
///
/// # Errors
/// Returns an error for a path that cannot be translated safely.
pub fn linux_path(path: &Path) -> io::Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("a sandbox path is not UTF-8"))?;
    if text.chars().any(char::is_control) {
        return Err(invalid("a sandbox path has a control character"));
    }
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let translated = if text.starts_with('/') {
        text.to_owned()
    } else {
        let mut chars = text.chars();
        let (Some(drive), Some(':')) = (chars.next(), chars.next()) else {
            return Err(invalid("a sandbox path must be absolute"));
        };
        let rest = chars.as_str();
        if !drive.is_ascii_alphabetic() || !(rest.starts_with('\\') || rest.starts_with('/')) {
            return Err(invalid("a sandbox path must be absolute"));
        }
        format!(
            "/mnt/{}{}",
            drive.to_ascii_lowercase(),
            rest.replace('\\', "/")
        )
    };
    let mut clean = String::new();
    let mut parts = 0;
    for part in translated.split('/').filter(|part| !part.is_empty()) {
        if part == ".." || part == "." {
            return Err(invalid("a sandbox path must not contain . or .."));
        }
        clean.push('/');
        clean.push_str(part);
        parts += 1;
    }
    // The root, `/mnt` and a whole drive (`/mnt/d`) are never a folder to open.
    let whole_drive = clean == "/mnt" || (clean.starts_with("/mnt/") && parts == 2);
    if parts == 0 || whole_drive {
        return Err(invalid(
            "a sandbox path must not be the root or a whole drive",
        ));
    }
    Ok(clean)
}

#[cfg(test)]
mod tests;
