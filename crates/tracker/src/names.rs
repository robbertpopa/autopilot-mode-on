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

impl Repo {
    /// The repository a github.com remote address names, in the https, scp-like
    /// or ssh spelling with or without `.git`. Another host gives `None`.
    #[must_use]
    pub fn from_remote(url: &str) -> Option<Self> {
        let url = url.trim();
        let path = [
            "https://github.com/",
            "git@github.com:",
            "ssh://git@github.com/",
        ]
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix))?;
        let path = path.strip_suffix(".git").unwrap_or(path);
        Self::parse(path).ok()
    }

    /// Whether a remote address points at this repository on github.com, in
    /// the https, scp-like or ssh spelling, with or without `.git`. Anything
    /// else, including another host or a lookalike prefix, is not a match.
    #[must_use]
    pub fn matches_remote(&self, url: &str) -> bool {
        let url = url.trim();
        let path = [
            "https://github.com/",
            "git@github.com:",
            "ssh://git@github.com/",
        ]
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix));
        path.map(|path| path.strip_suffix(".git").unwrap_or(path))
            .is_some_and(|path| {
                let mut parts = path.split('/');
                matches!((parts.next(), parts.next(), parts.next()),
                    (Some(owner), Some(name), None)
                        if owner.eq_ignore_ascii_case(&self.owner) && name.eq_ignore_ascii_case(&self.name))
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

    // Invariant: a push goes only to the approved repository on github.com.
    #[test]
    fn a_remote_must_be_the_approved_repository() {
        let repo = Repo::parse("Owner/name").unwrap();
        for good in [
            "https://github.com/owner/name",
            "https://github.com/Owner/name.git",
            "git@github.com:owner/name.git",
            "ssh://git@github.com/owner/name",
        ] {
            assert!(repo.matches_remote(good), "{good}");
        }
        for bad in [
            "https://github.com/owner/name-other",
            "https://github.com/owner/name/extra",
            "https://evil.example/owner/name",
            "https://github.com.evil.example/owner/name",
            "https://github.com/other/name",
            "/local/path/name",
            "",
        ] {
            assert!(!repo.matches_remote(bad), "{bad}");
        }
    }

    // Invariant: the repository read from a remote is exactly one the push check
    // would accept, and a remote that is not on github.com names none. The
    // window publishes to what this returns, so a reading that disagreed with
    // `matches_remote` would offer a repository the push then refuses.
    #[test]
    fn a_repository_read_from_a_remote_is_one_the_push_check_accepts() {
        for good in [
            "https://github.com/owner/name",
            "https://github.com/owner/name.git",
            "git@github.com:owner/name.git",
            "ssh://git@github.com/owner/name",
            "  https://github.com/owner/name\n",
        ] {
            let repo = Repo::from_remote(good).unwrap();
            assert_eq!(repo.to_string(), "owner/name", "{good}");
            assert!(repo.matches_remote(good), "{good}");
        }
        for bad in [
            "https://github.com/owner/name/extra",
            "https://evil.example/owner/name",
            "https://github.com.evil.example/owner/name",
            "D:/local/remote.git",
            "https://github.com/-x/name",
            "",
        ] {
            assert_eq!(Repo::from_remote(bad), None, "{bad}");
        }
    }

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
