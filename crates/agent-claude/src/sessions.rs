//! Opaque local references for provider sessions.
//!
//! Claude Code's session id is what `--resume` needs, but it is a handle to a
//! stored conversation, so it stays out of events, logs and the database. The
//! engine records only a reference like `session-1a2b3c4d`; this file, in the
//! data directory, maps it back.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

use autopilot_core::SessionId;

const FILE: &str = "claude-sessions.json";

pub struct SessionMap {
    path: PathBuf,
}

fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes)
}

/// A version 4 UUID, which is what `--session-id` accepts.
fn new_uuid() -> io::Result<String> {
    let mut bytes = random::<16>()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

impl SessionMap {
    #[must_use]
    pub fn in_directory(directory: &Path) -> Self {
        Self {
            path: directory.join(FILE),
        }
    }

    fn read(&self) -> io::Result<BTreeMap<String, String>> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error),
        }
    }

    fn write(&self, map: &BTreeMap<String, String>) -> io::Result<()> {
        let temporary = self.path.with_extension("tmp");
        write_private(
            &temporary,
            &serde_json::to_vec(map).map_err(io::Error::other)?,
        )?;
        fs::rename(&temporary, &self.path)
    }

    /// Start a session: a new provider id and the reference that stands for it.
    ///
    /// # Errors
    /// Returns an error if randomness or the file is unavailable.
    pub fn create(&self) -> io::Result<(SessionId, String)> {
        let mut map = self.read()?;
        let reference = format!("session-{}", hex(&random::<4>()?));
        let uuid = new_uuid()?;
        map.insert(reference.clone(), uuid.clone());
        self.write(&map)?;
        Ok((SessionId::from(reference), uuid))
    }

    /// The provider id behind a reference, if this machine has it.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read.
    pub fn lookup(&self, reference: &SessionId) -> io::Result<Option<String>> {
        Ok(self.read()?.get(reference.as_str()).cloned())
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

// Windows gives the file the permissions of the data directory, which the
// user profile already restricts to the owner.
#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: a reference never contains the provider id, and resolves back
    // to it. Two sessions get different references and different ids, so a
    // fresh session after failed rounds is not the old one.
    #[test]
    fn a_reference_resolves_to_its_own_uuid_and_reveals_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let map = SessionMap::in_directory(directory.path());
        let (first, first_uuid) = map.create().unwrap();
        let (second, second_uuid) = map.create().unwrap();
        assert_ne!(first, second);
        assert_ne!(first_uuid, second_uuid);
        assert!(!first.as_str().contains(&first_uuid[..8]));
        assert_eq!(map.lookup(&first).unwrap(), Some(first_uuid.clone()));
        assert_eq!(
            map.lookup(&SessionId::from("session-00000000")).unwrap(),
            None
        );

        let parts: Vec<&str> = first_uuid.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'), "version 4");
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn a_corrupt_map_is_an_error_not_an_empty_one() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(FILE), b"{not json").unwrap();
        let map = SessionMap::in_directory(directory.path());
        assert!(map.lookup(&SessionId::from("session-1")).is_err());
    }
}
