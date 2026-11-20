use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Component, PathBuf},
};

use crate::Error;

#[derive(Debug, Eq, PartialEq)]
pub struct PathOverlap {
    pub proposed: PathBuf,
    pub active: PathBuf,
}

/// Compare two tickets' repository-relative path predictions. An ancestor path
/// overlaps its descendants. Windows comparison also matches case variants.
///
/// # Errors
/// Returns an error if a prediction is empty, absolute, or traverses a parent.
pub fn overlapping_paths(
    proposed: &[PathBuf],
    active: &[PathBuf],
) -> Result<Vec<PathOverlap>, Error> {
    let proposed = normalize_paths(proposed)?;
    let active = normalize_paths(active)?;
    let mut overlaps = Vec::new();
    for (segments, proposed) in proposed {
        for depth in 1..segments.len() {
            if let Some(other) = active.get(&segments[..depth]) {
                overlaps.push(PathOverlap {
                    proposed: proposed.clone(),
                    active: other.clone(),
                });
            }
        }
        for (candidate, other) in active.range(segments.clone()..) {
            if !candidate.starts_with(&segments) {
                break;
            }
            overlaps.push(PathOverlap {
                proposed: proposed.clone(),
                active: other.clone(),
            });
        }
    }
    Ok(overlaps)
}

fn normalize_paths(paths: &[PathBuf]) -> Result<BTreeMap<Vec<OsString>, PathBuf>, Error> {
    let mut normalized = BTreeMap::new();
    for path in paths {
        let mut key = Vec::new();
        let mut display = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(name) => {
                    let comparison = comparison_name(name);
                    if comparison.is_empty() {
                        return Err(Error::InvalidPlannedPath(path.clone()));
                    }
                    key.push(comparison);
                    display.push(name);
                }
                Component::CurDir => {}
                _ => return Err(Error::InvalidPlannedPath(path.clone())),
            }
        }
        if key.is_empty() {
            return Err(Error::InvalidPlannedPath(path.clone()));
        }
        normalized.entry(key).or_insert(display);
    }
    Ok(normalized)
}

fn comparison_name(name: &OsStr) -> OsString {
    if cfg!(windows) {
        let text = name.to_string_lossy();
        if text.contains(':') {
            return OsString::new();
        }
        return text.trim_end_matches(['.', ' ']).to_uppercase().into();
    }
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Error, overlapping_paths};

    #[test]
    fn overlapping_segments_are_found_without_matching_siblings_or_repeated_entries() {
        let proposed = ["./src/app", "src/app", "config.toml"].map(PathBuf::from);
        let active = [
            "src/application.rs",
            "src/app/routes.rs",
            "config.toml",
            "other",
        ]
        .map(PathBuf::from);
        let overlaps = overlapping_paths(&proposed, &active).unwrap();
        assert_eq!(overlaps.len(), 2);
        assert!(
            overlaps
                .iter()
                .any(|overlap| overlap.proposed == Path::new("src/app")
                    && overlap.active == Path::new("src/app/routes.rs"))
        );
        assert!(
            overlaps
                .iter()
                .any(|overlap| overlap.proposed == Path::new("config.toml")
                    && overlap.active == Path::new("config.toml"))
        );
        assert_eq!(
            overlapping_paths(
                &[PathBuf::from("src/app/routes.rs")],
                &[PathBuf::from("src")]
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn an_invalid_plan_path_never_silently_counts_as_disjoint() {
        for path in ["", ".", "../outside", "src/../app", "/root"] {
            assert!(matches!(
                overlapping_paths(&[PathBuf::from(path)], &[]),
                Err(Error::InvalidPlannedPath(_))
            ));
        }
    }

    #[test]
    fn path_casing_follows_the_host_checkout() {
        let first = [PathBuf::from("Src/Code.rs")];
        let second = [PathBuf::from("src/code.rs")];
        assert_eq!(
            overlapping_paths(&first, &second).unwrap().len(),
            usize::from(cfg!(windows))
        );
    }
}
