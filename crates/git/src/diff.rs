use std::path::{Component, PathBuf};

use crate::{Error, ObjectId, Repository, git_os_value};

#[derive(Debug, Default, Eq, PartialEq)]
pub struct DiffStats {
    pub files: Vec<FileStats>,
    pub lines: LineStats,
}

#[derive(Debug, Eq, PartialEq)]
pub struct FileStats {
    pub path: PathBuf,
    /// Git does not report line counts for binary changes.
    pub lines: Option<LineStats>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LineStats {
    pub added: u64,
    pub removed: u64,
}

impl Repository {
    /// Compare two local commits and report every changed repository-relative path.
    ///
    /// Renames are reported as removal and addition so policy checks see both paths.
    ///
    /// # Errors
    /// Returns an error if Git fails or cannot return a complete, bounded summary.
    pub fn diff_stats(&self, base: &ObjectId, head: &ObjectId) -> Result<DiffStats, Error> {
        let output = self.read_git(
            "diff",
            &[
                "--numstat",
                "-z",
                "--no-ext-diff",
                "--no-textconv",
                "--no-relative",
                "--no-renames",
                "--ignore-submodules=none",
                base.as_str(),
                head.as_str(),
                "--",
            ],
        )?;
        parse_numstat(&output)
    }
}

impl Repository {
    /// The unified diff between two commits, for a reader that judges what the
    /// agent wrote. Binary changes appear as Git prints them, without content.
    ///
    /// # Errors
    /// Returns an error if Git fails or the patch exceeds the bounded read, so a
    /// review never judges a partial diff.
    pub fn patch(&self, base: &ObjectId, head: &ObjectId) -> Result<String, Error> {
        let output = self.read_git(
            "diff",
            &[
                "--unified=3",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-relative",
                "--no-renames",
                "--ignore-submodules=none",
                base.as_str(),
                head.as_str(),
                "--",
            ],
        )?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }
}

/// The lines a diff adds to one file, without the leading `+`.
#[derive(Debug, Eq, PartialEq)]
pub struct FileAdditions {
    pub path: String,
    pub lines: Vec<String>,
}

impl Repository {
    /// The added lines of every text file changed between two commits, for
    /// scans that look at what the agent wrote. Binary files have none.
    ///
    /// # Errors
    /// Returns an error if Git fails or the patch is larger than the bounded
    /// read allows, so a scan never runs on a partial diff.
    pub fn added_lines(
        &self,
        base: &ObjectId,
        head: &ObjectId,
    ) -> Result<Vec<FileAdditions>, Error> {
        let output = self.read_git(
            "diff",
            &[
                "--unified=0",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-relative",
                "--no-renames",
                "--ignore-submodules=none",
                base.as_str(),
                head.as_str(),
                "--",
            ],
        )?;
        Ok(parse_additions(&String::from_utf8_lossy(&output)))
    }
}

fn parse_additions(patch: &str) -> Vec<FileAdditions> {
    let mut files: Vec<FileAdditions> = Vec::new();
    let mut current: Option<usize> = None;
    // Header lines (`--- a/x`, `+++ b/x`) sit between `diff --git` and the
    // first `@@`; after that a line starting `+++` is an added line.
    let mut in_header = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            current = None;
            in_header = true;
        } else if in_header && line.starts_with("@@") {
            in_header = false;
        } else if in_header {
            if let Some(path) = line.strip_prefix("+++ ") {
                current = path.strip_prefix("b/").map(|name| {
                    files.push(FileAdditions {
                        path: name.trim_matches('"').to_owned(),
                        lines: Vec::new(),
                    });
                    files.len() - 1
                });
            }
        } else if let (Some(index), Some(added)) = (current, line.strip_prefix('+')) {
            files[index].lines.push(added.to_owned());
        }
    }
    files
}

fn parse_numstat(bytes: &[u8]) -> Result<DiffStats, Error> {
    let mut result = DiffStats::default();
    if bytes.is_empty() {
        return Ok(result);
    }
    let records = bytes.strip_suffix(b"\0").ok_or(Error::InvalidDiff)?;
    for record in records.split(|&byte| byte == 0) {
        let mut fields = record.splitn(3, |&byte| byte == b'\t');
        let added = fields.next().ok_or(Error::InvalidDiff)?;
        let removed = fields.next().ok_or(Error::InvalidDiff)?;
        let file = fields.next().ok_or(Error::InvalidDiff)?;
        if file
            .split(|&byte| byte == b'/')
            .any(|component| matches!(component, b"" | b"." | b".."))
        {
            return Err(Error::InvalidDiff);
        }
        let path = PathBuf::from(git_os_value(file).map_err(|_| Error::InvalidDiff)?);
        if !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(Error::InvalidDiff);
        }
        let lines = match (added, removed) {
            (b"-", b"-") => None,
            (added, removed) => Some(LineStats {
                added: line_count(added)?,
                removed: line_count(removed)?,
            }),
        };
        if let Some(lines) = lines {
            result.lines.added = result
                .lines
                .added
                .checked_add(lines.added)
                .ok_or(Error::InvalidDiff)?;
            result.lines.removed = result
                .lines
                .removed
                .checked_add(lines.removed)
                .ok_or(Error::InvalidDiff)?;
        }
        result.files.push(FileStats { path, lines });
    }
    Ok(result)
}

fn line_count(bytes: &[u8]) -> Result<u64, Error> {
    if !bytes.iter().all(u8::is_ascii_digit) {
        return Err(Error::InvalidDiff);
    }
    let number = std::str::from_utf8(bytes).map_err(|_| Error::InvalidDiff)?;
    number.parse().map_err(|_| Error::InvalidDiff)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Error, LineStats, parse_additions, parse_numstat};

    // Invariant: only lines a file gains are returned, per file; a deleted
    // file (no b/ side) and context lines add nothing.
    #[test]
    fn additions_are_read_per_file_and_skip_deleted_files() {
        let patch = "diff --git a/Cargo.toml b/Cargo.toml
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -3,0 +4,2 @@
+serde = \"1\"
+++ looks like a header but is a line
diff --git a/old.rs b/old.rs
--- a/old.rs
+++ /dev/null
@@ -1 +0,0 @@
-gone
diff --git a/new.rs b/new.rs
--- /dev/null
+++ b/new.rs
@@ -0,0 +1 @@
+fn main() {}
";
        let files = parse_additions(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "Cargo.toml");
        assert_eq!(
            files[0].lines,
            ["serde = \"1\"", "++ looks like a header but is a line"]
        );
        assert_eq!(files[1].path, "new.rs");
        assert_eq!(files[1].lines, ["fn main() {}"]);
    }

    #[test]
    fn numstat_keeps_unusual_names_and_binary_files_without_guessing_counts() {
        let result =
            parse_numstat(b"12\t3\tdirectory/file with\tand\nseparator\0-\t-\tbinary.dat\0")
                .unwrap();
        assert_eq!(
            result.lines,
            LineStats {
                added: 12,
                removed: 3
            }
        );
        assert_eq!(result.files.len(), 2);
        assert_eq!(
            result.files[0].path,
            Path::new("directory/file with\tand\nseparator")
        );
        assert_eq!(result.files[0].lines, Some(result.lines));
        assert_eq!(result.files[1].path, Path::new("binary.dat"));
        assert_eq!(result.files[1].lines, None);
    }

    #[test]
    fn malformed_stats_cannot_appear_as_a_partial_successful_diff() {
        for bad in [
            b"1\t2\ta\0-\t2\tb\0".as_slice(),
            b"1\t2\ta",
            b"1\t2\t../outside\0",
            b"18446744073709551615\t0\ta\x001\t0\tb\0",
        ] {
            assert!(matches!(parse_numstat(bad), Err(Error::InvalidDiff)));
        }
    }
}
