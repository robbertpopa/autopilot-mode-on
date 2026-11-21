//! Host-side publishing: reading a remote's address and pushing one commit to
//! one branch. Nothing here is reachable from an agent.
//!
//! A push needs the user's own Git config and credential helper, which every
//! other call in this crate deliberately blocks, so it builds its own
//! environment: an allowlist, no prompts, no hooks, and only the protocols a
//! remote can reasonably use.

use std::{ffi::OsString, sync::atomic::AtomicBool, time::Duration};

use autopilot_process::{CommandSpec, Limits, Outcome};

use crate::{Error, ObjectId, Repository, git_path};

const PUSH_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    bytes_per_stream: 256 * 1024,
};

const ENV_ALLOWED: &[&str] = &[
    "APPDATA",
    "GIT_ASKPASS",
    "GIT_SSH_COMMAND",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "HTTPS_PROXY",
    "LOCALAPPDATA",
    "NO_PROXY",
    "PATH",
    "PROGRAMDATA",
    "SSH_AUTH_SOCK",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "XDG_CONFIG_HOME",
];

fn plain_name(text: &str, extra: &[u8]) -> bool {
    !text.is_empty()
        && text.len() <= 200
        && !text.starts_with('-')
        && !text.contains("..")
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.')
                || extra.contains(&byte)
        })
}

impl Repository {
    /// The address a configured remote points at.
    ///
    /// # Errors
    /// Returns an error if the remote name is not plain or Git cannot read it.
    pub fn remote_url(&self, remote: &str) -> Result<String, Error> {
        if !plain_name(remote, b"") {
            return Err(Error::InvalidRemote(remote.into()));
        }
        let key = format!("remote.{remote}.url");
        let output = self.read_git("config", &["--get", &key])?;
        Ok(String::from_utf8_lossy(&output).trim().to_owned())
    }

    /// Push one commit to `refs/heads/<branch>` on `remote`, never forced: a
    /// branch that is not an ancestor of `commit` is refused by the remote
    /// and comes back as an error.
    ///
    /// The caller has already checked the branch with `Policy::check_push`
    /// and bound the commit in an approval; this only refuses names that
    /// could change what is pushed.
    ///
    /// # Errors
    /// Returns an error for an unusable name or when Git fails.
    pub fn push_commit(&self, remote: &str, commit: &ObjectId, branch: &str) -> Result<(), Error> {
        if !plain_name(remote, b"") || !plain_name(branch, b"/") || branch.starts_with('/') {
            return Err(Error::InvalidRemote(format!("{remote} {branch}")));
        }
        let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| {
                ENV_ALLOWED.iter().any(|allowed| {
                    name.to_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(allowed))
                })
            })
            .collect();
        env.push(("GIT_TERMINAL_PROMPT".into(), "0".into()));
        env.push(("GIT_ALLOW_PROTOCOL".into(), "https:ssh:file".into()));
        let refspec = format!("{}:refs/heads/{branch}", commit.as_str());
        let args: Vec<OsString> = ["--no-pager", "-c", "core.hooksPath=/dev/null", "-C"]
            .map(OsString::from)
            .into_iter()
            .chain([
                git_path(&self.directory),
                "push".into(),
                "--no-verify".into(),
                "--porcelain".into(),
                remote.into(),
                refspec.into(),
            ])
            .collect();
        let output = self.runner.run(
            &CommandSpec {
                program: self.program.clone(),
                args,
                env,
                cwd: None,
            },
            b"",
            PUSH_LIMITS,
            &AtomicBool::new(false),
        )?;
        match output.outcome {
            Outcome::Exited(status) if status.success() => Ok(()),
            Outcome::Exited(status) => Err(Error::Git {
                operation: "push",
                status,
                diagnostic: String::from_utf8_lossy(&output.stderr).trim_end().into(),
            }),
            Outcome::TimedOut => Err(Error::TimedOut("push")),
            Outcome::OutputLimit => Err(Error::OutputLimit("push")),
            Outcome::Cancelled => Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Git was cancelled",
            )
            .into()),
        }
    }
}
