//! The guardian watches a private pipe and stays alive if its caller is killed.

use std::{
    env,
    io::{self, Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::Instant,
};

use rustix::{
    io::Errno,
    process::{
        Pid, Signal, WaitId, WaitIdOptions, WaitOptions, kill_process_group, set_child_subreaper,
        waitid, waitpgid,
    },
};

use crate::{CLEANUP_TIMEOUT, CommandSpec, MAX_INPUT, POLL, Pipes, feed};

#[doc(hidden)]
pub const GUARDIAN_FAILURE: u8 = 125;

pub(crate) struct Active {
    guardian: Child,
    control: Option<ChildStdin>,
}

impl Active {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.guardian.try_wait()?.map(check_guardian).transpose()
    }

    pub(crate) fn shutdown(&mut self) -> io::Result<()> {
        drop(self.control.take());
        let started = Instant::now();
        while self.try_wait()?.is_none() {
            if started.elapsed() >= CLEANUP_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "guardian did not finish cleanup",
                ));
            }
            thread::sleep(POLL);
        }
        Ok(())
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        // An early pipe or thread error must still close the lifetime pipe and reap the guardian.
        let _ = self.shutdown();
    }
}

fn check_guardian(status: ExitStatus) -> io::Result<ExitStatus> {
    match status.code() {
        Some(code) if code != i32::from(GUARDIAN_FAILURE) => Ok(status),
        _ => Err(io::Error::other(format!(
            "Linux guardian failed with {status}"
        ))),
    }
}

pub(crate) fn start(
    guardian: &Path,
    spec: &CommandSpec,
    input: &[u8],
) -> io::Result<(Active, Pipes)> {
    if !guardian.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected an absolute guardian path",
        ));
    }
    let child = Command::new(guardian)
        .arg(&spec.program)
        .args(&spec.args)
        .env_clear()
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        // A terminal interrupt for the owner must leave the guardian around to clean up.
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut active = Active {
        guardian: child,
        control: None,
    };
    let mut control = active
        .guardian
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing guardian stdin"))?;
    let input_len = u32::try_from(input.len()).map_err(io::Error::other)?;
    control.write_all(&input_len.to_le_bytes())?;
    control.write_all(input)?;
    active.control = Some(control);
    let pipes = Pipes {
        stdout: Box::new(
            active
                .guardian
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing guardian stdout"))?,
        ),
        stderr: Box::new(
            active
                .guardian
                .stderr
                .take()
                .ok_or_else(|| io::Error::other("missing guardian stderr"))?,
        ),
    };
    Ok((active, pipes))
}

/// Entrypoint for the sibling Linux guardian binary.
///
/// # Errors
/// Returns an error if the private protocol, spawn or process-group cleanup fails.
#[doc(hidden)]
pub fn supervise() -> io::Result<u8> {
    let mut args = env::args_os().skip(1);
    let program = PathBuf::from(
        args.next()
            .ok_or_else(|| io::Error::other("missing worker path"))?,
    );
    if !program.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected an absolute worker path",
        ));
    }
    let mut length = [0; 4];
    let mut control = io::stdin();
    control.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_INPUT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "guardian input exceeds 1 MiB",
        ));
    }

    let mut input = vec![0; length];
    control.read_exact(&mut input)?;
    let (closed, owner) = mpsc::channel();
    thread::Builder::new().spawn(move || {
        let mut byte = [0];
        let _ = control.read_exact(&mut byte);
        let _ = closed.send(());
    })?;

    set_child_subreaper(Some(Pid::INIT))?;
    let mut worker = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::piped())
        .spawn()?;
    let group = Pid::from_child(&worker);
    let stdin = worker.stdin.take();
    let mut tree = Tree {
        worker: Some(worker),
        group,
    };
    let writer = stdin
        .ok_or_else(|| io::Error::other("missing worker stdin"))
        .and_then(|stdin| feed(stdin, input))?;

    loop {
        // Leave the exited leader unreaped until after signaling the group so its ID can't be reused.
        match waitid(
            WaitId::Pid(group),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(Some(_)) => break,
            Ok(None) | Err(Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
        if !matches!(
            owner.recv_timeout(POLL),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            break;
        }
    }

    let code = tree
        .finish()?
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .filter(|&code| code != GUARDIAN_FAILURE)
        .unwrap_or(1);
    crate::joined(writer)?;
    Ok(code)
}

struct Tree {
    worker: Option<Child>,
    group: Pid,
}

impl Tree {
    fn finish(&mut self) -> io::Result<ExitStatus> {
        let worker = self
            .worker
            .as_mut()
            .ok_or_else(|| io::Error::other("worker already reaped"))?;
        match kill_process_group(self.group, Signal::KILL) {
            Ok(()) | Err(Errno::SRCH) => {}
            Err(error) => return Err(error.into()),
        }
        let status = worker.wait()?;
        self.worker = None;
        // Adopted grandchildren in the group also need to be reaped, especially under container PID 1.
        loop {
            match waitpgid(self.group, WaitOptions::empty()) {
                Ok(Some(_)) | Err(Errno::INTR) => {}
                Ok(None) | Err(Errno::CHILD) => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(status)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.finish();
        }
    }
}
