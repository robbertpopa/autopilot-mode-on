//! macOS supervision without a guardian.
//!
//! Linux needs a sibling guardian process because it can adopt orphaned
//! grandchildren (child subreaper). macOS has no such call, so a guardian would
//! add a second binary to the app bundle and still could not keep descendants
//! from escaping. What is kept: the worker leads its own process group, and the
//! whole group is killed when the call ends, so everything that stayed in the
//! group goes with it. A descendant that starts its own session (`setsid`)
//! leaves the group, so each launch also carries a unique marker in its
//! environment and the end of the call kills every process of this user that
//! still has it. What is not covered: a descendant that clears its environment
//! first, and if this program is killed outright the worker is not cleaned up.

use std::{
    io,
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

use rustix::{
    io::Errno,
    process::{Pid, Signal, kill_process, kill_process_group},
};

use crate::{CommandSpec, Pipes, feed, joined};

const MARK_VARIABLE: &str = "AUTOPILOT_RUN_MARK";

/// Rounds of "list, kill" after the group kill. A survivor can fork once more
/// between the list and the kill; each round removes one generation.
const SWEEP_ROUNDS: usize = 5;

pub(crate) struct Active {
    child: Child,
    group: Pid,
    mark: String,
    finished: bool,
    writer: Option<JoinHandle<io::Result<()>>>,
}

impl Active {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn shutdown(&mut self) -> io::Result<()> {
        // `Drop` calls this again after the caller did; the second run would
        // only list every process for nothing.
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        // A leader that already exited can leave members behind, so the group is
        // signalled either way. macOS answers EPERM, not ESRCH, when the only
        // member left is the exited leader waiting to be reaped, and every
        // process in the group is our own descendant, so both mean "nothing
        // alive to signal".
        match kill_process_group(self.group, Signal::KILL) {
            Ok(()) | Err(Errno::SRCH | Errno::PERM) => {}
            Err(error) => return Err(error.into()),
        }
        self.child.wait()?;
        sweep_marked(&self.mark);
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

/// Kill every process of this user whose environment still carries `mark`: the
/// ones that left the process group with `setsid` and were adopted by launchd,
/// which no parent link can find. Best effort, because the group kill already
/// ran and a failed listing must not fail a finished command.
fn sweep_marked(mark: &str) {
    for _ in 0..SWEEP_ROUNDS {
        let Ok(listing) = Command::new("/bin/ps")
            .args(["-A", "-E", "-ww", "-o", "pid=,command="])
            .stdin(Stdio::null())
            .output()
        else {
            return;
        };
        let pids = marked_pids(&String::from_utf8_lossy(&listing.stdout), mark);
        if pids.is_empty() {
            return;
        }
        for pid in pids {
            let _ = kill_process(pid, Signal::KILL);
        }
    }
}

/// The pids in a `ps -E` listing whose environment has exactly `mark`. A mark
/// that only starts with ours (another run's, longer) must not match.
fn marked_pids(listing: &str, mark: &str) -> Vec<Pid> {
    let wanted = format!("{MARK_VARIABLE}={mark}");
    let own = std::process::id();
    let mut found = Vec::new();
    for line in listing.lines() {
        let Some((pid, rest)) = line.trim_start().split_once(' ') else {
            continue;
        };
        if !rest.split(' ').any(|word| word == wanted) {
            continue;
        }
        let Some(pid) = pid.parse::<u32>().ok().filter(|pid| *pid != own) else {
            continue;
        };
        if let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) {
            found.push(pid);
        }
    }
    found
}

/// Unique per launch: the pid and clock tell this program's runs apart from
/// another program's, the counter tells two launches in one tick apart.
fn new_mark() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos:x}-{count}", std::process::id())
}

pub(crate) fn start(_: &Path, spec: &CommandSpec, input: &[u8]) -> io::Result<(Active, Pipes)> {
    let mark = new_mark();
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .env_clear()
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        .env(MARK_VARIABLE, &mark)
        .current_dir(spec.cwd.as_deref().unwrap_or(Path::new(".")))
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let group = Pid::from_child(&child);
    let (Some(stdout), Some(stderr), Some(stdin)) =
        (child.stdout.take(), child.stderr.take(), child.stdin.take())
    else {
        let _ = kill_process_group(group, Signal::KILL);
        let _ = child.wait();
        return Err(io::Error::other("missing worker pipe"));
    };
    let writer = feed(stdin, input.to_vec())?;
    Ok((
        Active {
            child,
            group,
            mark,
            finished: false,
            writer: Some(writer),
        },
        Pipes {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::marked_pids;

    // Invariant: only a process that carries this run's exact mark is killed.
    // A prefix match would kill another run's worker, and a missing match
    // leaves a `setsid` descendant running; no integration test sees a wrong
    // pid in a listing, so this one does.
    #[test]
    fn only_the_exact_mark_is_picked_from_a_listing() {
        let listing = "  101 /bin/sleep 5 HOME=/x AUTOPILOT_RUN_MARK=7-ab-1 PATH=/bin
                         102 /bin/sleep 5 AUTOPILOT_RUN_MARK=7-ab-10 PATH=/bin
                         103 /bin/sleep 5 PATH=/bin
                         bad line
";
        let pids: Vec<i32> = marked_pids(listing, "7-ab-1")
            .iter()
            .map(|pid| pid.as_raw_nonzero().get())
            .collect();
        assert_eq!(pids, vec![101]);
    }
}
