//! Standing allowances: steps whose optional "ask me after this step"
//! confirmation the owner turned off for the whole project.
//!
//! Only a confirmation that is not forced can be allowed. A forced one (a
//! plan that cannot be checked, a moved branch, a publish) always asks. The
//! file lives in the data directory, is read fresh each time, and a missing or
//! damaged file means nothing is allowed, which is the safe reading.

use std::{collections::BTreeSet, fs, path::Path};

use autopilot_core::Step;

use crate::{Error, step_name};

const FILE: &str = "allowances.json";

fn read(directory: &Path) -> BTreeSet<String> {
    fs::read_to_string(directory.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<BTreeSet<String>>(&text).ok())
        .unwrap_or_default()
}

fn key(step: Step) -> String {
    step_name(step).to_lowercase()
}

/// Whether the owner turned off the optional confirmation after `step`.
#[must_use]
pub fn allowed(directory: &Path, step: Step) -> bool {
    read(directory).contains(&key(step))
}

/// The steps with a standing allowance, in pipeline order.
#[must_use]
pub fn list(directory: &Path) -> Vec<Step> {
    let set = read(directory);
    Step::ALL
        .into_iter()
        .filter(|step| set.contains(&key(*step)))
        .collect()
}

fn write(directory: &Path, set: &BTreeSet<String>) -> Result<(), Error> {
    fs::create_dir_all(directory).map_err(|error| Error::History(error.to_string()))?;
    let text =
        serde_json::to_string_pretty(set).map_err(|error| Error::History(error.to_string()))?;
    fs::write(directory.join(FILE), text).map_err(|error| Error::History(error.to_string()))
}

/// Stop asking after `step` in this project.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn allow(directory: &Path, step: Step) -> Result<(), Error> {
    let mut set = read(directory);
    if set.insert(key(step)) {
        write(directory, &set)?;
    }
    Ok(())
}

/// Ask after `step` again.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn revoke(directory: &Path, step: Step) -> Result<(), Error> {
    let mut set = read(directory);
    if set.remove(&key(step)) {
        write(directory, &set)?;
    }
    Ok(())
}
