//! The inbox file protocol: durable requests and answers as small JSON files.
//!
//! The event log stays the source of truth. A request file mirrors an `Asked`
//! event so another process can see what is waiting, and an answer file is
//! only a proposal until the engine commits `Answered`. Files are deleted
//! after that commit, so a crash in between leaves stale files that the engine
//! recognises and removes instead of applying them twice.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use autopilot_core::{AssumptionCorrection, OpenQuestion, RequestId, TicketId};
use serde::{Deserialize, Serialize};

use crate::clarify::Choice;

pub const DIRECTORY: &str = "inbox";
const REQUEST_SUFFIX: &str = ".request.json";
const ANSWER_SUFFIX: &str = ".answer.json";
/// Request and answer files are small; a larger one is hostile or corrupt.
const MAX_FILE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequestBody {
    Question {
        questions: Vec<OpenQuestion>,
    },
    Approval {
        /// What exactly would happen if the owner approves.
        action: String,
        /// A forced confirmation cannot be turned into a standing allowance.
        forced: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub id: RequestId,
    pub ticket: TicketId,
    pub body: RequestBody,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Edit(String),
    Reject,
    /// Stop the step and run it again with these notes from the owner. Only
    /// Understand and Plan read them.
    SendBack(String),
    /// Approve, and record corrections to assumptions on the way.
    ApproveCorrected(Vec<AssumptionCorrection>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Question {
        choices: Vec<Choice>,
    },
    Approval {
        decision: Decision,
        /// Stop asking about this kind of optional confirmation in the project.
        #[serde(default)]
        allow_for_project: bool,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error("request id {0:?} may only contain letters, digits, '-' and '_'")]
    BadId(String),
    #[error("the reply is {reply}, but the request is a {request}")]
    WrongKind {
        request: &'static str,
        reply: &'static str,
    },
    #[error("a forced confirmation cannot be turned into a standing allowance")]
    ForcedAllowance,
    #[error("{0} is larger than the inbox allows")]
    TooLarge(String),
    #[error("{name} is not valid: {source}")]
    Malformed {
        name: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Check a reply against the request it claims to answer.
///
/// # Errors
/// Returns an error for a reply of the wrong kind, or one that tries to make
/// a forced confirmation permanent.
pub fn check_reply(request: &Request, reply: &Reply) -> Result<(), InboxError> {
    match (&request.body, reply) {
        (RequestBody::Question { .. }, Reply::Question { .. }) => Ok(()),
        (
            RequestBody::Approval { forced, .. },
            Reply::Approval {
                allow_for_project, ..
            },
        ) => {
            if *forced && *allow_for_project {
                Err(InboxError::ForcedAllowance)
            } else {
                Ok(())
            }
        }
        (RequestBody::Question { .. }, Reply::Approval { .. }) => Err(InboxError::WrongKind {
            request: "question",
            reply: "an approval",
        }),
        (RequestBody::Approval { .. }, Reply::Question { .. }) => Err(InboxError::WrongKind {
            request: "approval",
            reply: "a question answer",
        }),
    }
}

#[derive(Clone, Debug)]
pub struct Inbox {
    directory: PathBuf,
}

impl Inbox {
    #[must_use]
    pub fn in_data_dir(data: &Path) -> Self {
        Self {
            directory: data.join(DIRECTORY),
        }
    }

    /// Mirror a request. Rewriting the same request is harmless.
    ///
    /// # Errors
    /// Returns an error for an unsafe id or an unwritable directory.
    pub fn write_request(&self, request: &Request) -> Result<(), InboxError> {
        let bytes = serde_json::to_vec_pretty(request).map_err(|source| InboxError::Malformed {
            name: "request".into(),
            source,
        })?;
        self.write_atomic(&request.id, REQUEST_SUFFIX, &bytes)
    }

    /// The answer appears whole or not at all: it is written beside the target
    /// and renamed into place.
    ///
    /// # Errors
    /// Returns an error for an unsafe id or an unwritable directory.
    pub fn write_answer(&self, id: &RequestId, reply: &Reply) -> Result<(), InboxError> {
        let bytes = serde_json::to_vec_pretty(reply).map_err(|source| InboxError::Malformed {
            name: "answer".into(),
            source,
        })?;
        self.write_atomic(id, ANSWER_SUFFIX, &bytes)
    }

    /// # Errors
    /// Returns an error for an unsafe id, an oversized file or bad JSON.
    pub fn read_answer(&self, id: &RequestId) -> Result<Option<Reply>, InboxError> {
        self.read(id, ANSWER_SUFFIX)
    }

    /// # Errors
    /// Returns an error for an unsafe id, an oversized file or bad JSON.
    pub fn read_request(&self, id: &RequestId) -> Result<Option<Request>, InboxError> {
        self.read(id, REQUEST_SUFFIX)
    }

    /// Every request file that parses. A file that does not is left alone and
    /// ignored, so one bad file cannot hide the others.
    ///
    /// # Errors
    /// Returns an error if the directory exists but cannot be listed.
    pub fn requests(&self) -> Result<Vec<Request>, InboxError> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.into()),
        };
        let mut found = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_suffix(REQUEST_SUFFIX))
            else {
                continue;
            };
            if let Ok(Some(request)) = self.read_request(&RequestId::from(id)) {
                found.push(request);
            }
        }
        found.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(found)
    }

    /// Ids that have an answer file, whether or not the request file survives.
    ///
    /// # Errors
    /// Returns an error if the directory exists but cannot be listed.
    pub fn answer_ids(&self) -> Result<Vec<RequestId>, InboxError> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.into()),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            if let Some(id) = name.to_str().and_then(|n| n.strip_suffix(ANSWER_SUFFIX)) {
                ids.push(RequestId::from(id));
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Remove both files after the database commit. Missing files are fine.
    ///
    /// # Errors
    /// Returns an error for an unsafe id or a file that cannot be removed.
    pub fn clean_up(&self, id: &RequestId) -> Result<(), InboxError> {
        for suffix in [REQUEST_SUFFIX, ANSWER_SUFFIX] {
            match fs::remove_file(self.path(id, suffix)?) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }
        Ok(())
    }

    // An id comes from a file name or a command line, so it must not be able
    // to name a path outside the inbox.
    fn path(&self, id: &RequestId, suffix: &str) -> Result<PathBuf, InboxError> {
        let text = id.as_str();
        let safe = !text.is_empty()
            && text.len() <= 64
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !safe {
            return Err(InboxError::BadId(text.to_owned()));
        }
        Ok(self.directory.join(format!("{text}{suffix}")))
    }

    fn read<T: serde::de::DeserializeOwned>(
        &self,
        id: &RequestId,
        suffix: &str,
    ) -> Result<Option<T>, InboxError> {
        let path = self.path(id, suffix)?;
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len() > MAX_FILE_BYTES {
            return Err(InboxError::TooLarge(name));
        }
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut io::Read::take(file, MAX_FILE_BYTES), &mut bytes)?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| InboxError::Malformed { name, source })
    }

    fn write_atomic(&self, id: &RequestId, suffix: &str, bytes: &[u8]) -> Result<(), InboxError> {
        let target = self.path(id, suffix)?;
        create_private_directory(&self.directory)?;
        let temporary = self.directory.join(format!("{}{suffix}.tmp", id.as_str()));
        write_private_file(&temporary, bytes)?;
        fs::rename(&temporary, &target)?;
        Ok(())
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

// Windows gives the folder the permissions of the data directory, which the
// user profile already restricts to the owner.
#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question_request(id: &str) -> Request {
        Request {
            id: RequestId::from(id),
            ticket: TicketId::from("t"),
            body: RequestBody::Question {
                questions: vec![OpenQuestion {
                    question: "q".into(),
                    options: vec!["a".into(), "b".into()],
                    recommended: 0,
                }],
            },
        }
    }

    fn approval(forced: bool) -> Request {
        Request {
            id: RequestId::from("approve-1"),
            ticket: TicketId::from("t"),
            body: RequestBody::Approval {
                action: "push".into(),
                forced,
            },
        }
    }

    // Invariant: a request file and an answer file survive a round trip, and
    // cleaning up is idempotent so a crash before cleanup can be retried.
    #[test]
    fn request_and_answer_round_trip_and_clean_up_twice() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::in_data_dir(dir.path());
        let request = question_request("clarify-1");
        assert_eq!(inbox.requests().unwrap(), vec![]);
        inbox.write_request(&request).unwrap();
        assert_eq!(inbox.requests().unwrap(), vec![request.clone()]);
        let reply = Reply::Question {
            choices: vec![Choice::Recommended],
        };
        assert_eq!(inbox.read_answer(&request.id).unwrap(), None);
        inbox.write_answer(&request.id, &reply).unwrap();
        assert_eq!(inbox.read_answer(&request.id).unwrap(), Some(reply));
        inbox.clean_up(&request.id).unwrap();
        inbox.clean_up(&request.id).unwrap();
        assert_eq!(inbox.requests().unwrap(), vec![]);
    }

    // Invariant: an id from outside can never name a path outside the inbox.
    // Allowing ".." would let an answer file overwrite arbitrary files.
    #[test]
    fn hostile_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::in_data_dir(dir.path());
        for id in ["", "..", "../x", "a/b", "a\\b", "a.b", &"x".repeat(65)] {
            let id = RequestId::from(id);
            assert!(
                matches!(inbox.read_answer(&id), Err(InboxError::BadId(_))),
                "{id:?}"
            );
            assert!(
                inbox
                    .write_answer(&id, &Reply::Question { choices: vec![] })
                    .is_err()
            );
        }
    }

    // Invariant: a half-written or garbage answer is an error for that request
    // only, and a torn temporary file is never mistaken for an answer.
    #[test]
    fn malformed_oversized_and_temporary_files_are_not_answers() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::in_data_dir(dir.path());
        let id = RequestId::from("r1");
        inbox.write_request(&question_request("r1")).unwrap();
        let folder = dir.path().join(DIRECTORY);
        fs::write(folder.join("r1.answer.json.tmp"), "{").unwrap();
        assert_eq!(inbox.read_answer(&id).unwrap(), None);
        fs::write(folder.join("r1.answer.json"), "{not json").unwrap();
        assert!(matches!(
            inbox.read_answer(&id),
            Err(InboxError::Malformed { .. })
        ));
        fs::write(folder.join("r1.answer.json"), vec![b' '; 300_000]).unwrap();
        assert!(matches!(
            inbox.read_answer(&id),
            Err(InboxError::TooLarge(_))
        ));
        fs::write(folder.join("bad.request.json"), "nope").unwrap();
        assert_eq!(inbox.requests().unwrap().len(), 1);
    }

    #[test]
    fn replies_must_match_the_request_kind() {
        let question = question_request("q");
        let approve = Reply::Approval {
            decision: Decision::Approve,
            allow_for_project: false,
        };
        assert!(matches!(
            check_reply(&question, &approve),
            Err(InboxError::WrongKind { .. })
        ));
        assert!(check_reply(&approval(false), &approve).is_ok());
        assert!(matches!(
            check_reply(&approval(false), &Reply::Question { choices: vec![] }),
            Err(InboxError::WrongKind { .. })
        ));
    }

    // Invariant: approving a forced confirmation covers that one action only.
    // Without this check one click could silence a protected-path warning.
    #[test]
    fn a_forced_confirmation_cannot_become_a_standing_allowance() {
        let standing = Reply::Approval {
            decision: Decision::Approve,
            allow_for_project: true,
        };
        assert!(matches!(
            check_reply(&approval(true), &standing),
            Err(InboxError::ForcedAllowance)
        ));
        assert!(check_reply(&approval(false), &standing).is_ok());
    }
}
