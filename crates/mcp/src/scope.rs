//! Which files the server may touch. Paths are resolved to where they really
//! lead before they are compared with the roots, so `..` and links cannot
//! step outside.

use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use crate::{Error, MAX_READ_BYTES, MAX_WRITE_BYTES};

#[derive(Clone, Debug)]
pub struct Scope {
    reads: Vec<PathBuf>,
    writes: Vec<PathBuf>,
}

fn roots(list: &[PathBuf], what: &str) -> Result<Vec<PathBuf>, Error> {
    if list.is_empty() {
        return Err(Error::Refused(format!(
            "give at least one {what} root; there is no default"
        )));
    }
    list.iter()
        .map(|root| {
            fs::canonicalize(root)
                .map_err(|error| Error::Refused(format!("{what} root {}: {error}", root.display())))
        })
        .collect()
}

fn refused(path: &str, why: &str) -> Error {
    Error::Refused(format!("{path}: {why}"))
}

impl Scope {
    /// # Errors
    /// Returns an error if either list is empty or a root does not exist.
    pub fn new(reads: &[PathBuf], writes: &[PathBuf]) -> Result<Self, Error> {
        Ok(Self {
            reads: roots(reads, "read")?,
            writes: roots(writes, "write")?,
        })
    }

    pub(crate) fn read(&self, path: &str) -> Result<String, Error> {
        let real = fs::canonicalize(path).map_err(|_| refused(path, "no such file"))?;
        if !self.reads.iter().any(|root| real.starts_with(root)) {
            return Err(refused(path, "outside the read roots"));
        }
        let meta = fs::metadata(&real)?;
        if !meta.is_file() {
            return Err(refused(path, "not a file"));
        }
        if meta.len() > MAX_READ_BYTES {
            return Err(refused(path, "larger than the read limit of 256 KiB"));
        }
        let mut text = String::new();
        fs::File::open(&real)?
            .take(MAX_READ_BYTES)
            .read_to_string(&mut text)
            .map_err(|_| refused(path, "not UTF-8 text"))?;
        Ok(text)
    }

    pub(crate) fn write(&self, path: &str, text: &str) -> Result<String, Error> {
        if text.len() > MAX_WRITE_BYTES {
            return Err(refused(path, "larger than the write limit of 1 MiB"));
        }
        let target = Path::new(path);
        let name = target
            .file_name()
            .ok_or_else(|| refused(path, "no file name"))?;
        if target
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(refused(path, "a path may not contain .."));
        }
        let folder = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| refused(path, "give the whole path"))?;
        let folder =
            fs::canonicalize(folder).map_err(|_| refused(path, "the folder does not exist"))?;
        if !self.writes.iter().any(|root| folder.starts_with(root)) {
            return Err(refused(path, "outside the write roots"));
        }
        let real = folder.join(name);
        // A link in the last place would lead the write somewhere else.
        if fs::symlink_metadata(&real).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(refused(path, "is a link"));
        }
        fs::write(&real, text)?;
        Ok(format!("wrote {} bytes", text.len()))
    }
}
