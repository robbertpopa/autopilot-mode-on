//! Files, images and folders attached to a ticket.
//!
//! Attachments are data. Nothing here opens, runs or interprets what is
//! inside them: a file is copied byte for byte into the data directory and
//! hashed, and a folder is only recorded by its path, item count and size,
//! never copied, because a folder can be a whole project with secrets in it.
//! No agent reads them yet; the fixture agent ignores them and real agents are
//! still refused until S6.
//!
//! Limits (the numbers are a decision, change them here): 25 MiB per file,
//! 100 MiB and 20 attachments per ticket, 20 000 items counted in a folder.
//! A source that is a symbolic link is refused, so an attachment can never be
//! a way to read a file the owner did not pick. Staging is all or nothing.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use autopilot_core::{AttachmentKind, Event, RecordedEvent, TicketId};
use autopilot_pipeline::fold;
use autopilot_store::{SqliteStore, Store};
use sha2::{Digest, Sha256};

use crate::{DATABASE, Error, event, lock_run, timestamp};

pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_TICKET_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 20;
pub const MAX_FOLDER_ITEMS: u64 = 20_000;

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// One staged attachment, ready to be recorded as an event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
    pub bytes: u64,
    pub items: u64,
    pub sha256: String,
    /// A path relative to the data directory for a copied file; the folder's own path for a folder.
    pub stored: String,
}

impl Attachment {
    pub(crate) fn into_event(self) -> Event {
        Event::Attached {
            name: self.name,
            kind: self.kind,
            bytes: self.bytes,
            items: self.items,
            sha256: self.sha256,
            stored: self.stored,
        }
    }
}

fn refuse(text: impl Into<String>) -> Error {
    Error::Attachment(text.into())
}

/// The name shown and used on disk: plain characters only, no leading dots,
/// at most 80 long, so it can never climb out of its folder.
fn clean_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    name = String::from(name.trim_start_matches(['.', ' ']).trim_end());
    if name.chars().count() > 80 {
        name = name.chars().take(80).collect();
    }
    if name.is_empty() {
        "file".to_owned()
    } else {
        name
    }
}

fn kind_of(path: &Path) -> AttachmentKind {
    let image = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        });
    if image {
        AttachmentKind::Image
    } else {
        AttachmentKind::File
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// Count regular files and their size below `path`, never following links.
fn measure_folder(path: &Path) -> Result<(u64, u64), Error> {
    let mut stack = vec![path.to_path_buf()];
    let (mut items, mut bytes) = (0_u64, 0_u64);
    while let Some(folder) = stack.pop() {
        for entry in fs::read_dir(&folder).map_err(|error| refuse(error.to_string()))? {
            let entry = entry.map_err(|error| refuse(error.to_string()))?;
            let meta =
                fs::symlink_metadata(entry.path()).map_err(|error| refuse(error.to_string()))?;
            if meta.file_type().is_symlink() {
                continue;
            }
            items += 1;
            if items > MAX_FOLDER_ITEMS {
                return Err(refuse(format!(
                    "a folder with more than {MAX_FOLDER_ITEMS} items cannot be attached"
                )));
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                bytes = bytes.saturating_add(meta.len());
            }
        }
    }
    Ok((items, bytes))
}

enum Source {
    File {
        path: PathBuf,
        name: String,
        bytes: u64,
    },
    Folder {
        path: PathBuf,
        name: String,
    },
}

fn inspect(path: &Path, total: &mut u64) -> Result<Source, Error> {
    if !path.is_absolute() {
        return Err(refuse(format!(
            "{} is not an absolute path",
            path.display()
        )));
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|error| refuse(format!("{}: {error}", path.display())))?;
    if meta.file_type().is_symlink() {
        return Err(refuse(format!(
            "{} is a link, which cannot be attached",
            path.display()
        )));
    }
    let name = clean_name(path);
    if meta.is_dir() {
        return Ok(Source::Folder {
            path: path.to_path_buf(),
            name,
        });
    }
    if !meta.is_file() {
        return Err(refuse(format!(
            "{} is not a file or a folder",
            path.display()
        )));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(refuse(format!(
            "{name} is larger than {} MiB",
            MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    *total = total.saturating_add(meta.len());
    if *total > MAX_TICKET_BYTES {
        return Err(refuse(format!(
            "the files are larger than {} MiB together",
            MAX_TICKET_BYTES / 1024 / 1024
        )));
    }
    Ok(Source::File {
        path: path.to_path_buf(),
        name,
        bytes: meta.len(),
    })
}

fn copy_file(source: &Path, target: &Path, expected: u64) -> Result<String, Error> {
    let mut input = File::open(source).map_err(|error| refuse(error.to_string()))?;
    let mut output = File::create_new(target).map_err(|error| refuse(error.to_string()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut copied = 0_u64;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| refuse(error.to_string()))?;
        if read == 0 {
            break;
        }
        copied += read as u64;
        // The file may grow after it was measured; the cap holds anyway.
        if copied > expected.max(MAX_FILE_BYTES) {
            return Err(refuse("a file grew while it was being copied"));
        }
        digest.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|error| refuse(error.to_string()))?;
    }
    Ok(hex(&digest.finalize()))
}

/// Check and copy the chosen files and record the chosen folders, all or nothing.
///
/// `already` is how many attachments and bytes the ticket holds, so the
/// per-ticket limits count them.
///
/// # Errors
/// Returns the reason for the first source that is refused. Nothing stays on
/// disk then.
pub fn stage(
    directory: &Path,
    ticket: &TicketId,
    sources: &[PathBuf],
    already: (usize, u64),
) -> Result<Vec<Attachment>, Error> {
    let id = crate::control::plain_id(ticket)?;
    if already.0 + sources.len() > MAX_ATTACHMENTS {
        return Err(refuse(format!(
            "a ticket can hold at most {MAX_ATTACHMENTS} attachments"
        )));
    }
    let mut total = already.1;
    let planned = sources
        .iter()
        .map(|path| inspect(path, &mut total))
        .collect::<Result<Vec<_>, _>>()?;

    let folder = directory.join("attachments").join(id);
    let mut made: Vec<PathBuf> = Vec::new();
    let mut staged = Vec::new();
    let outcome = (|| -> Result<(), Error> {
        for (index, source) in planned.into_iter().enumerate() {
            match source {
                Source::File { path, name, bytes } => {
                    fs::create_dir_all(&folder).map_err(|error| refuse(error.to_string()))?;
                    let stored = format!("{}-{name}", already.0 + index + 1);
                    let target = folder.join(&stored);
                    made.push(target.clone());
                    let sha256 = copy_file(&path, &target, bytes)?;
                    let kind = kind_of(&path);
                    staged.push(Attachment {
                        name,
                        kind,
                        bytes,
                        items: 1,
                        sha256,
                        stored: format!("attachments/{id}/{stored}"),
                    });
                }
                Source::Folder { path, name } => {
                    let (items, bytes) = measure_folder(&path)?;
                    staged.push(Attachment {
                        name,
                        kind: AttachmentKind::Folder,
                        bytes,
                        items,
                        sha256: String::new(),
                        stored: path.to_string_lossy().into_owned(),
                    });
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        for path in made {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    Ok(staged)
}

/// How much of one text file goes into a prompt, and of all of them together.
const EXCERPT_BYTES: usize = 16 * 1024;
const NOTE_BYTES: usize = 48 * 1024;

/// What the agent is told about a ticket's attachments, as plain text.
///
/// Small UTF-8 text files are quoted (up to 16 KiB each, 48 KiB in all);
/// images are only named, because the agent cannot see them; a folder gives
/// its name, item count and size, never a listing. All of it is data and the
/// prompt says so. Empty when there are no attachments.
#[must_use]
pub fn note(directory: &Path, history: &[RecordedEvent]) -> String {
    let mut text = String::new();
    for record in history {
        let Event::Attached {
            name,
            kind,
            bytes,
            items,
            stored,
            ..
        } = &record.event.kind
        else {
            continue;
        };
        let line = match kind {
            AttachmentKind::Folder => format!("- folder {name}: {items} items, {bytes} bytes\n"),
            AttachmentKind::Image => {
                format!("- image {name}: {bytes} bytes (you cannot see images)\n")
            }
            AttachmentKind::File => format!("- file {name}: {bytes} bytes\n"),
        };
        text.push_str(&line);
        if matches!(kind, AttachmentKind::File)
            && text.len() < NOTE_BYTES
            && let Some(excerpt) = excerpt(&directory.join(stored))
        {
            text.push_str("  ~~~\n");
            text.push_str(&excerpt);
            text.push_str("\n  ~~~\n");
        }
    }
    text
}

/// The first bytes of a text file, cut on a character boundary, or `None`
/// for a file that is not valid UTF-8 text.
fn excerpt(path: &Path) -> Option<String> {
    let mut buffer = Vec::new();
    File::open(path)
        .ok()?
        .take(EXCERPT_BYTES as u64 + 4)
        .read_to_end(&mut buffer)
        .ok()?;
    let text = match String::from_utf8(buffer) {
        Ok(text) => text,
        // The cut may land inside a character; trim back to a boundary.
        Err(error) => {
            let valid = error.utf8_error().valid_up_to();
            if error.as_bytes().len() <= valid + 3 {
                String::from_utf8(error.as_bytes()[..valid].to_vec()).ok()?
            } else {
                return None;
            }
        }
    };
    let mut end = text.len().min(EXCERPT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].replace("~~~", "- - -"))
}

/// Attach files, images or folders to a ticket that already exists.
///
/// # Errors
/// Returns an error if another action holds the data directory, the ticket is
/// unknown or done, or a source is refused. Nothing is written then.
pub fn attach(directory: &Path, ticket: &TicketId, sources: &[PathBuf]) -> Result<usize, Error> {
    let _owner = lock_run(directory)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let mut state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    let (mut count, mut bytes) = (0_usize, 0_u64);
    for record in &history {
        if let Event::Attached {
            kind, bytes: size, ..
        } = &record.event.kind
        {
            count += 1;
            if *kind != AttachmentKind::Folder {
                bytes = bytes.saturating_add(*size);
            }
        }
    }
    let staged = stage(directory, ticket, sources, (count, bytes))?;
    let now = timestamp()?;
    let events: Vec<_> = staged
        .iter()
        .cloned()
        .map(|attachment| event(now, None, attachment.into_event()))
        .collect();
    for item in &events {
        state
            .apply(item)
            .map_err(|error| Error::NotReady(error.to_string()))?;
    }
    store.append(ticket, last, &events)?;
    Ok(events.len())
}
