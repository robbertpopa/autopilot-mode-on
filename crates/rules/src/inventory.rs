//! A model-free map of a repository: which files there are, in what language,
//! how big, and how to cut them into review units that each fit a token budget.
//!
//! Generated and vendored files are skipped and named, links are never
//! followed, and binary files are skipped. Sizes are bytes; tokens are
//! estimated as one per four bytes, which is rough on purpose and says so.

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::Error;

const SKIPPED_DIRECTORIES: [&str; 12] = [
    ".git",
    "target",
    "node_modules",
    "vendor",
    "dist",
    "build",
    ".venv",
    "__pycache__",
    ".next",
    ".gradle",
    "third_party",
    ".idea",
];
const GENERATED_NAMES: [&str; 6] = [
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "go.sum",
];
const MAX_FILES: usize = 100_000;
const BYTES_PER_TOKEN: u64 = 4;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FileEntry {
    /// Relative to the repository, with `/` separators.
    pub path: String,
    pub language: &'static str,
    pub bytes: u64,
    pub tokens: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: &'static str,
}

/// Files an agent reads together. A file larger than the budget stands alone
/// and is marked `oversized`, so the caller can split or refuse it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Unit {
    pub files: Vec<String>,
    pub tokens: u64,
    pub oversized: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Inventory {
    pub files: Vec<FileEntry>,
    pub skipped: Vec<Skipped>,
    pub by_language: BTreeMap<&'static str, u64>,
    pub units: Vec<Unit>,
    pub tokens: u64,
}

impl Inventory {
    /// A cost estimate in micro-dollars for reading everything once, at the
    /// given price per million input tokens (also in micro-dollars).
    #[must_use]
    pub fn estimate_microusd(&self, price_per_million_tokens: u64) -> u64 {
        self.tokens.saturating_mul(price_per_million_tokens) / 1_000_000
    }
}

fn language(path: &str) -> &'static str {
    let extension = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    match extension.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "cs" => "csharp",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "rb" => "ruby",
        "swift" => "swift",
        "sh" | "bash" => "shell",
        "ps1" => "powershell",
        "toml" | "yaml" | "yml" | "json" => "config",
        "md" => "markdown",
        "html" | "css" => "web",
        "sql" => "sql",
        _ => "other",
    }
}

/// The first bytes say a file is binary (a zero byte) or generated.
fn sniff(path: &Path) -> Result<Option<&'static str>, Error> {
    let mut head = Vec::with_capacity(8192);
    fs::File::open(path)?.take(8192).read_to_end(&mut head)?;
    if head.contains(&0) {
        return Ok(Some("binary"));
    }
    let text = String::from_utf8_lossy(&head);
    let lower = text.to_ascii_lowercase();
    if lower.contains("@generated") || lower.contains("do not edit") && lower.contains("generated")
    {
        return Ok(Some("generated"));
    }
    Ok(None)
}

fn walk(
    root: &Path,
    directory: &Path,
    files: &mut Vec<FileEntry>,
    skipped: &mut Vec<Skipped>,
) -> Result<(), Error> {
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            skipped.push(Skipped {
                path: relative,
                reason: "link",
            });
        } else if meta.is_dir() {
            if SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                skipped.push(Skipped {
                    path: relative,
                    reason: "vendored or build output",
                });
            } else {
                walk(root, &path, files, skipped)?;
            }
        } else if meta.is_file() {
            if files.len() + skipped.len() >= MAX_FILES {
                return Err(Error::Invalid(format!(
                    "more than {MAX_FILES} files; point the review at a smaller folder"
                )));
            }
            let reason = if GENERATED_NAMES.contains(&name.as_str()) || name.ends_with(".min.js") {
                Some("generated")
            } else {
                sniff(&path)?
            };
            match reason {
                Some(reason) => skipped.push(Skipped {
                    path: relative,
                    reason,
                }),
                None => files.push(FileEntry {
                    language: language(&relative),
                    bytes: meta.len(),
                    tokens: meta.len().div_ceil(BYTES_PER_TOKEN),
                    path: relative,
                }),
            }
        }
    }
    Ok(())
}

/// Map a repository and cut it into units of at most `unit_tokens` tokens.
/// Files stay in path order, so neighbours in a folder usually share a unit.
///
/// # Errors
/// Returns an error if a folder cannot be read, the budget is zero, or the
/// repository holds more than 100,000 files.
pub fn inventory(root: &Path, unit_tokens: u64) -> Result<Inventory, Error> {
    if unit_tokens == 0 {
        return Err(Error::Invalid("a unit needs a token budget".into()));
    }
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    walk(root, root, &mut files, &mut skipped)?;

    let mut by_language: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut units: Vec<Unit> = Vec::new();
    let mut current = Unit {
        files: Vec::new(),
        tokens: 0,
        oversized: false,
    };
    for file in &files {
        *by_language.entry(file.language).or_default() += file.bytes;
        if file.tokens > unit_tokens {
            if !current.files.is_empty() {
                units.push(std::mem::replace(
                    &mut current,
                    Unit {
                        files: Vec::new(),
                        tokens: 0,
                        oversized: false,
                    },
                ));
            }
            units.push(Unit {
                files: vec![file.path.clone()],
                tokens: file.tokens,
                oversized: true,
            });
            continue;
        }
        if current.tokens + file.tokens > unit_tokens {
            units.push(std::mem::replace(
                &mut current,
                Unit {
                    files: Vec::new(),
                    tokens: 0,
                    oversized: false,
                },
            ));
        }
        current.files.push(file.path.clone());
        current.tokens += file.tokens;
    }
    if !current.files.is_empty() {
        units.push(current);
    }
    let tokens = files.iter().map(|file| file.tokens).sum();
    Ok(Inventory {
        files,
        skipped,
        by_language,
        units,
        tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, name: &str, bytes: &[u8]) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    // Invariant: vendored folders, lock files, generated headers and binary
    // files stay out of the review, each named with its reason, while source
    // files stay in with their language.
    #[test]
    fn generated_vendored_and_binary_files_are_skipped_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", b"fn main() {}\n");
        write(root, "target/x.rs", b"fn x() {}\n");
        write(root, "Cargo.lock", b"# lock\n");
        write(root, "gen.rs", b"// @generated by tool\nfn g() {}\n");
        write(root, "pic.png", &[0x89, 0, 1, 2]);
        let found = inventory(root, 1000).unwrap();
        let kept: Vec<&str> = found.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(kept, ["src/a.rs"]);
        assert_eq!(found.files[0].language, "rust");
        let why: BTreeMap<&str, &str> = found
            .skipped
            .iter()
            .map(|s| (s.path.as_str(), s.reason))
            .collect();
        assert_eq!(why["target"], "vendored or build output");
        assert_eq!(why["Cargo.lock"], "generated");
        assert_eq!(why["gen.rs"], "generated");
        assert_eq!(why["pic.png"], "binary");
    }

    // Invariant: no unit exceeds the budget unless it is one oversized file
    // that says so, and every kept file lands in exactly one unit.
    #[test]
    fn units_respect_the_budget_and_cover_every_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.rs", &[b'a'; 400]); // 100 tokens
        write(root, "b.rs", &[b'b'; 400]);
        write(root, "c.rs", &[b'c'; 400]);
        write(root, "huge.rs", &[b'h'; 4000]); // 1000 tokens
        let found = inventory(root, 250).unwrap();
        let mut seen: Vec<&str> = found
            .units
            .iter()
            .flat_map(|u| u.files.iter().map(String::as_str))
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, ["a.rs", "b.rs", "c.rs", "huge.rs"]);
        for unit in &found.units {
            assert!(unit.oversized || unit.tokens <= 250, "{unit:?}");
        }
        assert_eq!(found.units.iter().filter(|u| u.oversized).count(), 1);
        assert!(inventory(root, 0).is_err());
        assert_eq!(found.estimate_microusd(3_000_000), found.tokens * 3);
    }
}
