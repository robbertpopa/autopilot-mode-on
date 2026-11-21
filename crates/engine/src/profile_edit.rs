//! Changing `profile.toml` from a client. The file is edited in place, so the
//! owner's comments and layout stay, and nothing is written unless the result
//! still parses as a valid profile.

use std::{fs, io, path::Path};

use autopilot_core::Step;
use autopilot_profile::{Auth, Profile, ProfileDiagnostic};
use toml_edit::{DocumentMut, Item, Table, value};

#[derive(Debug, thiserror::Error)]
pub enum ProfileEditError {
    #[error("could not read or write the profile: {0}")]
    Io(#[from] io::Error),
    #[error("the profile is not valid TOML: {0}")]
    Syntax(String),
    /// The change would make the profile invalid. Nothing was written.
    #[error("{0}")]
    Invalid(#[from] ProfileDiagnostic),
}

/// One setting of one step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepField {
    Auth(Auth),
    Model(String),
    Effort(String),
    Turns(u32),
    Rounds(u32),
    /// `None` removes the dollar budget.
    BudgetMicrousd(Option<u64>),
    AskAfter(bool),
}

/// Read a profile. A missing file is the default profile.
///
/// # Errors
/// Returns the diagnostic of a profile that does not parse.
pub fn load_profile(path: &Path) -> Result<Profile, ProfileEditError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    Ok(Profile::parse(path, &source)?)
}

/// Change one step setting and save the file, returning the new profile.
///
/// # Errors
/// Returns an error for an unreadable file, or the diagnostic when the change
/// would make the profile invalid (a dollar budget on a subscription step, for
/// example). The file is left as it was in that case.
pub fn edit_step(path: &Path, step: Step, field: &StepField) -> Result<Profile, ProfileEditError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut document: DocumentMut = source
        .parse()
        .map_err(|error: toml_edit::TomlError| ProfileEditError::Syntax(error.to_string()))?;
    let table = step_table(&mut document, step)?;
    match field {
        StepField::Auth(auth) => {
            table["auth"] = value(match auth {
                Auth::Subscription => "subscription",
                Auth::Api => "api",
            });
        }
        StepField::Model(model) => table["model"] = value(model.as_str()),
        StepField::Effort(effort) => table["effort"] = value(effort.as_str()),
        StepField::AskAfter(ask) => table["ask_after"] = value(*ask),
        StepField::Turns(turns) => limits(table)?["turns"] = value(i64::from(*turns)),
        StepField::Rounds(rounds) => limits(table)?["rounds"] = value(i64::from(*rounds)),
        StepField::BudgetMicrousd(Some(budget)) => {
            let budget = i64::try_from(*budget)
                .map_err(|_| ProfileEditError::Syntax("budget is too large".into()))?;
            limits(table)?["budget_microusd"] = value(budget);
        }
        StepField::BudgetMicrousd(None) => {
            limits(table)?.remove("budget_microusd");
        }
    }
    let text = document.to_string();
    let profile = Profile::parse(path, &text)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(profile)
}

fn step_table(document: &mut DocumentMut, step: Step) -> Result<&mut Table, ProfileEditError> {
    let steps = child_table(document.as_table_mut(), "steps")?;
    steps.set_implicit(true);
    child_table(steps, step.as_str())
}

fn limits(table: &mut Table) -> Result<&mut Table, ProfileEditError> {
    child_table(table, "limits")
}

fn child_table<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table, ProfileEditError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent[key]
        .as_table_mut()
        .ok_or_else(|| ProfileEditError::Syntax(format!("`{key}` is not a table")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_of(profile: &Profile, step: Step) -> &autopilot_profile::StepSettings {
        profile.step(step).unwrap()
    }

    // Invariant: an edit keeps the owner's comments and other keys, and the
    // saved file parses to what the window shows.
    #[test]
    fn an_edit_keeps_comments_and_is_saved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(&path, "# my notes\nprotected_paths = [\"infra/**\"]\n").unwrap();
        let profile = edit_step(&path, Step::Plan, &StepField::Model("opus-5-5".into())).unwrap();
        assert_eq!(
            step_of(&profile, Step::Plan).model.as_deref(),
            Some("opus-5-5")
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my notes"));
        assert!(text.contains("infra/**"));
        assert_eq!(load_profile(&path).unwrap(), profile);
    }

    // Invariant: a change that would make the profile invalid is refused with
    // the profile's own diagnostic, and the file is not touched.
    #[test]
    fn a_dollar_budget_on_a_subscription_step_is_refused_and_not_written() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        let error = edit_step(
            &path,
            Step::Understand,
            &StepField::BudgetMicrousd(Some(200_000)),
        )
        .unwrap_err();
        assert!(matches!(error, ProfileEditError::Invalid(_)));
        assert!(error.to_string().contains("dollar budgets require auth"));
        assert!(!path.exists());
        // Switching to the API first makes the same budget valid.
        edit_step(&path, Step::Understand, &StepField::Auth(Auth::Api)).unwrap();
        let profile = edit_step(
            &path,
            Step::Understand,
            &StepField::BudgetMicrousd(Some(200_000)),
        )
        .unwrap();
        assert!(
            step_of(&profile, Step::Understand)
                .limits
                .budget_microusd
                .is_some()
        );
    }

    #[test]
    fn limits_and_ask_after_are_written_and_a_budget_can_be_removed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/profile.toml");
        edit_step(&path, Step::BuildCheck, &StepField::Turns(60)).unwrap();
        edit_step(&path, Step::BuildCheck, &StepField::Rounds(2)).unwrap();
        let profile = edit_step(&path, Step::BuildCheck, &StepField::AskAfter(true)).unwrap();
        let build = step_of(&profile, Step::BuildCheck);
        assert_eq!(
            (build.limits.turns, build.limits.rounds, build.ask_after),
            (60, 2, true)
        );
        let profile = edit_step(&path, Step::BuildCheck, &StepField::BudgetMicrousd(None)).unwrap();
        assert!(
            step_of(&profile, Step::BuildCheck)
                .limits
                .budget_microusd
                .is_none()
        );
    }

    #[test]
    fn broken_toml_is_reported_and_left_alone() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(&path, "steps = [").unwrap();
        let error = edit_step(&path, Step::Plan, &StepField::AskAfter(false)).unwrap_err();
        assert!(matches!(error, ProfileEditError::Syntax(_)));
        assert_eq!(fs::read_to_string(&path).unwrap(), "steps = [");
    }
}
