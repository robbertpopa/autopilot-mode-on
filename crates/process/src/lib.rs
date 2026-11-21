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
    /// Absolute directory to start in. `None` inherits the supervisor's.
    pub cwd: Option<PathBuf>,
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
        self.run_with_stdout(spec, input, limits, cancel, |_| {})
    }

    /// Run an absolute executable and observe stdout as it arrives. Chunks can
    /// split lines and UTF-8 characters. They are delivered in order, up to the
    /// same byte limit as the returned output.
    ///
    /// The observer runs on the stdout reader thread. It must return promptly
    /// so the reader can keep draining the pipe.
    ///
    /// # Errors
    /// Returns an error if the arguments, launch, pipes or cleanup fail.
    pub fn run_with_stdout(
        &self,
        spec: &CommandSpec,
        input: &[u8],
        limits: Limits,
        cancel: &AtomicBool,
        observe: impl FnMut(&[u8]) + Send + 'static,
    ) -> io::Result<Output> {
        if !spec.program.is_absolute() || input.len() > MAX_INPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected an absolute executable and at most 1 MiB of input",
            ));
        }
        if spec.cwd.as_ref().is_some_and(|dir| !dir.is_absolute()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected an absolute working directory",
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
        let stdout = capture(pipes.stdout, limits.bytes_per_stream, &overflow, observe)?;
        let stderr = capture(pipes.stderr, limits.bytes_per_stream, &overflow, |_| {})?;

        let finished = loop {
            if overflow.load(Ordering::Acquire) {
                break Ok(Outcome::OutputLimit);
            }
            if cancel.load(Ordering::Acquire) {
                // An observer may cancel after the reader has reported overflow.
                break Ok(if overflow.load(Ordering::Acquire) {
                    Outcome::OutputLimit
                } else {
                    Outcome::Cancelled
                });
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
    mut observe: impl FnMut(&[u8]) + Send + 'static,
) -> io::Result<JoinHandle<io::Result<Vec<u8>>>> {
    let overflow = Arc::clone(overflow);
    thread::Builder::new().spawn(move || {
        let mut pipe = pipe.take((limit as u64).saturating_add(1));
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            let read = match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            let accepted = read.min(limit - bytes.len());
            let exceeded = accepted < read;
            if exceeded {
                overflow.store(true, Ordering::Release);
            }
            if accepted != 0 {
                bytes.extend_from_slice(&chunk[..accepted]);
                observe(&chunk[..accepted]);
            }
            if exceeded {
                break;
            }
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
