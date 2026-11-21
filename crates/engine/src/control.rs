//! Stop, pause and resume a run from another process.
//!
//! A run holds the data directory's lock, so a second process cannot touch
//! its history. It leaves a flag file instead, and the run looks at the flag
//! between turns: never in the middle of one, so a turn is either finished
//! and committed or not started.

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use autopilot_core::TicketId;

use crate::Error;

const POLL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Flag {
    Stop,
    Pause,
}

fn flag_path(directory: &Path, ticket: &TicketId, flag: Flag) -> Result<PathBuf, Error> {
    // The id becomes a file name, so only plain ids may.
    let id = ticket.as_str();
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(Error::InvalidTicket);
    }
    let extension = match flag {
        Flag::Stop => "stop",
        Flag::Pause => "pause",
    };
    Ok(directory.join("control").join(format!("{id}.{extension}")))
}

fn set(directory: &Path, ticket: &TicketId, flag: Flag) -> Result<(), Error> {
    let path = flag_path(directory, ticket, flag)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, b"")?;
    Ok(())
}

fn clear(directory: &Path, ticket: &TicketId, flag: Flag) -> Result<(), Error> {
    match fs::remove_file(flag_path(directory, ticket, flag)?) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn is_set(directory: &Path, ticket: &TicketId, flag: Flag) -> Result<bool, Error> {
    Ok(flag_path(directory, ticket, flag)?.exists())
}

/// Ask the run of this ticket to stop at its next turn boundary. The step
/// ends as interrupted, so it can be run again.
///
/// # Errors
/// Returns an error if the flag cannot be written.
pub fn request_stop(directory: &Path, ticket: &TicketId) -> Result<(), Error> {
    set(directory, ticket, Flag::Stop)
}

/// Ask the run to hold at its next turn boundary until [`request_resume`].
///
/// # Errors
/// Returns an error if the flag cannot be written.
pub fn request_pause(directory: &Path, ticket: &TicketId) -> Result<(), Error> {
    set(directory, ticket, Flag::Pause)
}

/// Let a held run go on.
///
/// # Errors
/// Returns an error if the flag cannot be removed.
pub fn request_resume(directory: &Path, ticket: &TicketId) -> Result<(), Error> {
    clear(directory, ticket, Flag::Pause)
}

/// What the run saw at a turn boundary.
pub(crate) enum Gate {
    Go,
    /// The run was held and has been released; the caller records both.
    Held,
}

/// Called by a run between turns. Returns [`Error::Stopped`] if a stop was
/// asked for, after clearing the flag. If a pause is set, `on_hold` is
/// called once, the run waits until it is cleared, and `on_release` is called.
pub(crate) fn gate(
    directory: &Path,
    ticket: &TicketId,
    mut on_hold: impl FnMut() -> Result<(), Error>,
    mut on_release: impl FnMut() -> Result<(), Error>,
) -> Result<Gate, Error> {
    let mut held = false;
    loop {
        if is_set(directory, ticket, Flag::Stop)? {
            clear(directory, ticket, Flag::Stop)?;
            clear(directory, ticket, Flag::Pause)?;
            return Err(Error::Stopped("stopped by the owner".into()));
        }
        if !is_set(directory, ticket, Flag::Pause)? {
            break;
        }
        if !held {
            held = true;
            on_hold()?;
        }
        thread::sleep(POLL);
    }
    if held {
        on_release()?;
        Ok(Gate::Held)
    } else {
        Ok(Gate::Go)
    }
}
