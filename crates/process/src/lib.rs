//! Bounded execution and lifecycle supervision for a subprocess tree.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
use windows as platform;

use std::{
    ffi::OsString,
    io::{self, Read, Write},
    path::PathBuf,
    process::ExitStatus,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_INPUT: usize = 1024 * 1024;
const POLL: Duration = Duration::from_millis(15);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub timeout: Duration,
    pub bytes_per_stream: usize,
}

#[derive(Debug)]
pub enum Outcome {
    Exited(ExitStatus),
    Cancelled,
    TimedOut,
    OutputLimit,
}

#[derive(Debug)]
pub struct Output {
    pub outcome: Outcome,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub struct Runner {
    guardian: PathBuf,
}

impl Runner {
    /// Windows uses a job object. On Linux, set this to the trusted guardian executable.
    #[must_use]
    pub fn new(guardian: PathBuf) -> Self {
        Self { guardian }
    }

    /// Run an absolute executable path with only the specified environment.
    ///
    /// # Errors
    /// Returns an error if the arguments, launch, pipes or cleanup fail.
    pub fn run(
        &self,
        spec: &CommandSpec,
        input: &[u8],
        limits: Limits,
        cancel: &AtomicBool,
    ) -> io::Result<Output> {
        if !spec.program.is_absolute() || input.len() > MAX_INPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected an absolute executable and at most 1 MiB of input",
            ));
        }

        if cancel.load(Ordering::Acquire) {
            return Ok(Output {
                outcome: Outcome::Cancelled,
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }

        let started = Instant::now();
        let (mut child, pipes) = platform::start(&self.guardian, spec, input)?;
        let overflow = Arc::new(AtomicBool::new(false));
        let stdout = capture(pipes.stdout, limits.bytes_per_stream, &overflow)?;
        let stderr = capture(pipes.stderr, limits.bytes_per_stream, &overflow)?;

        let finished = loop {
            if overflow.load(Ordering::Acquire) {
                break Ok(Outcome::OutputLimit);
            }
            if cancel.load(Ordering::Acquire) {
                break Ok(Outcome::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(Outcome::Exited(status)),
                Ok(None) => {}
                Err(error) => break Err(error),
            }
            if started.elapsed() >= limits.timeout {
                break Ok(Outcome::TimedOut);
            }
            thread::sleep(POLL.min(limits.timeout.saturating_sub(started.elapsed())));
        };

        child.shutdown()?;
        let stdout = joined(stdout)?;
        let stderr = joined(stderr)?;
        let mut outcome = finished?;
        if matches!(outcome, Outcome::Exited(_)) && overflow.load(Ordering::Acquire) {
            outcome = Outcome::OutputLimit;
        }

        Ok(Output {
            outcome,
            stdout,
            stderr,
        })
    }
}

struct Pipes {
    stdout: Box<dyn Read + Send>,
    stderr: Box<dyn Read + Send>,
}

fn feed(
    mut pipe: impl Write + Send + 'static,
    input: Vec<u8>,
) -> io::Result<JoinHandle<io::Result<()>>> {
    thread::Builder::new().spawn(move || match pipe.write_all(&input) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    })
}

fn capture(
    pipe: Box<dyn Read + Send>,
    limit: usize,
    overflow: &Arc<AtomicBool>,
) -> io::Result<JoinHandle<io::Result<Vec<u8>>>> {
    let overflow = Arc::clone(overflow);
    thread::Builder::new().spawn(move || {
        let mut bytes = Vec::new();
        pipe.take((limit as u64).saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            overflow.store(true, Ordering::Release);
            bytes.truncate(limit);
        }
        Ok(bytes)
    })
}

fn joined<T>(task: JoinHandle<io::Result<T>>) -> io::Result<T> {
    let started = Instant::now();
    while !task.is_finished() {
        if started.elapsed() >= CLEANUP_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "subprocess pipe stayed open after cleanup",
            ));
        }
        thread::sleep(POLL);
    }
    task.join()
        .map_err(|_| io::Error::other("subprocess I/O task panicked"))?
}
