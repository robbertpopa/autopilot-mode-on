use std::fmt;

use crate::Error;

/// `owner/name`, checked so it can never be read as an option or a path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repo {
    owner: String,
    name: String,
}

fn plain(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && !part.starts_with(['-', '.'])
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

impl Repo {
    /// # Errors
    /// Returns an error unless the text is `owner/name` with plain parts.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (owner, name) = text
            .split_once('/')
            .ok_or_else(|| Error::InvalidName(text.into()))?;
        if !plain(owner) || !plain(name) {
            return Err(Error::InvalidName(text.into()));
        }
        Ok(Self {
            owner: owner.into(),
            name: name.into(),
        })
    }
}

impl fmt::Display for Repo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.owner, self.name)
    }
}

/// A branch name that is safe to pass as one argument: it cannot start with a
/// dash, hold spaces, control characters or `..`, or end the way Git refuses.
#[must_use]
pub fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with(['-', '/'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("//")
        && !name.to_ascii_lowercase().ends_with(".lock")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: a name from outside can never become an option or leave its slot.
    #[test]
    fn names_that_look_like_options_or_paths_are_refused() {
        assert!(Repo::parse("owner/name").is_ok());
        for bad in [
            "owner",
            "/name",
            "owner/",
            "-x/name",
            "owner/--help",
            "a/b/c",
            "o w/n",
            ".a/b",
        ] {
            assert!(Repo::parse(bad).is_err(), "{bad}");
        }
        assert!(valid_branch("ticket/gh-12"));
        for bad in [
            "", "-x", "a..b", "a b", "a/", "/a", "a//b", "x.lock", "a;b", "a\nb",
        ] {
            assert!(!valid_branch(bad), "{bad:?}");
        }
    }
}
