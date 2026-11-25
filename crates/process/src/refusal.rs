//! Why the sandbox is not usable, as data. The window shows each missing tool
//! and each failed row on its own line; it must not recover them by cutting up
//! a sentence, which breaks the day the sentence is reworded.

use std::{error::Error, fmt, io};

use crate::isolation::Report;

/// What stopped the boundary from being handed out. It travels inside the
/// `io::Error` that discovery and validation return; [`Refusal::of`] gets it
/// back out. `Display` is the sentence a person reads.
#[derive(Clone, Debug)]
pub enum Refusal {
    /// Discovery did not find these tools.
    Missing { place: String, tools: Vec<String> },
    /// The probe ran and these rows did not behave as expected. The whole
    /// report is kept, so the rows that held are available too.
    Failed(Report),
}

impl Refusal {
    /// The structured reason inside `error`, if it came from discovery or the
    /// probe. Any other error (a launcher that did not start, an unsupported
    /// system) has no structure and is shown as its text.
    #[must_use]
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { place, tools } => write!(f, "{place} is missing: {}", tools.join(", ")),
            Self::Failed(report) => write!(
                f,
                "the WSL boundary failed {} check(s) with {}: {}",
                report.failures.len(),
                report.versions,
                report.failures.join("; ")
            ),
        }
    }
}

impl Error for Refusal {}

impl From<Refusal> for io::Error {
    fn from(refusal: Refusal) -> Self {
        Self::other(refusal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolation::RowResult;

    // Invariant: a failed probe reaches the caller with every row, passed or
    // not, and with the sentence unchanged. A change that breaks it: building
    // the error from the text again, which leaves the window cutting up words.
    #[test]
    fn a_failed_probe_keeps_its_rows_through_an_io_error() {
        let report = Report {
            versions: "srt 0.0.83".to_owned(),
            rows: vec![
                RowResult {
                    role: "agent",
                    row: "read-sibling",
                    ok: false,
                },
                RowResult {
                    role: "agent",
                    row: "write-worktree",
                    ok: true,
                },
            ],
            failures: vec!["agent: read-sibling was allowed".to_owned()],
        };
        let error = io::Error::from(Refusal::Failed(report));
        assert_eq!(
            error.to_string(),
            "the WSL boundary failed 1 check(s) with srt 0.0.83: agent: read-sibling was allowed"
        );
        let Some(Refusal::Failed(back)) = Refusal::of(&error) else {
            panic!("the report was lost");
        };
        assert_eq!(back.rows.len(), 2);
        assert!(!back.rows[0].ok && back.rows[1].ok);
    }

    // Invariant: an error that is not a refusal has no structure, so the caller
    // falls back to its text instead of showing an empty list.
    #[test]
    fn an_unrelated_error_is_not_a_refusal() {
        assert!(Refusal::of(&io::Error::other("the launcher did not start")).is_none());
        assert!(Refusal::of(&io::Error::from(io::ErrorKind::NotFound)).is_none());
    }
}
