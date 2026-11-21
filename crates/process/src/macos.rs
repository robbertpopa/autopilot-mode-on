//! macOS supervision without a guardian.
//!
//! Linux needs a sibling guardian process because it can adopt orphaned
//! grandchildren (child subreaper). macOS has no such call, so a guardian would
//! add a second binary to the app bundle and still could not keep descendants
//! from escaping. What is kept: the worker leads its own process group, and the
//! whole group is killed when the call ends, so everything that stayed in the
//! group goes with it. What is not: a descendant that starts its own session
//! (`setsid`) leaves the group and survives, and if this program is killed
//! outright the worker is not cleaned up.

use std::{
    io,
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, ExitStatus, Stdio},
    thread::JoinHandle,
};

use rustix::{
    io::Errno,
    process::{Pid, Signal, kill_process_group},
};

use crate::{CommandSpec, Pipes, feed, joined};

pub(crate) struct Active {
    child: Child,
    group: Pid,
    writer: Option<JoinHandle<io::Result<()>>>,
}

impl Active {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn shutdown(&mut self) -> io::Result<()> {
        // A leader that already exited can leave members behind, so the group is
        // signalled either way. An empty group is not an error.
        match kill_process_group(self.group, Signal::KILL) {
            Ok(()) | Err(Errno::SRCH) => {}
            Err(error) => return Err(error.into()),
        }
        self.child.wait()?;
        match self.writer.take() {
            Some(writer) => joined(writer),
            None => Ok(()),
        }
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

pub(crate) fn start(_: &Path, spec: &CommandSpec, input: &[u8]) -> io::Result<(Active, Pipes)> {
    let mut child = std::process::Command::new(&spec.program)
        .args(&spec.args)
        .env_clear()
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        .current_dir(spec.cwd.as_deref().unwrap_or(Path::new(".")))
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let group = Pid::from_child(&child);
    let pipes = match (child.stdout.take(), child.stderr.take(), child.stdin.take()) {
        (Some(stdout), Some(stderr), Some(stdin)) => {
            let writer = feed(stdin, input.to_vec())?;
            (stdout, stderr, writer)
        }
        _ => {
            let _ = kill_process_group(group, Signal::KILL);
            let _ = child.wait();
            return Err(io::Error::other("missing worker pipe"));
        }
    };
    let (stdout, stderr, writer) = pipes;
    Ok((
        Active {
            child,
            group,
            writer: Some(writer),
        },
        Pipes {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
        },
    ))
}
