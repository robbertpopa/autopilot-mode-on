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

/// One setting of one project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectField {
    Name(String),
    /// `None` removes the default and the ticket falls back to `full`.
    DefaultPipeline(Option<String>),
    GithubRepo(Option<String>),
}

/// Add a project for a workspace folder, or change one of its settings. The
/// key is the normalised folder path, so the same folder written two ways is
/// one project. The file keeps its comments, and nothing is written unless the
/// result still parses.
///
/// # Errors
/// Returns an error for an unreadable file, or the diagnostic when the result
/// is invalid (for example a default pipeline that does not exist).
pub fn edit_project(
    path: &Path,
    workspace: &str,
    field: Option<&ProjectField>,
) -> Result<Profile, ProfileEditError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut document: DocumentMut = source
        .parse()
        .map_err(|error: toml_edit::TomlError| ProfileEditError::Syntax(error.to_string()))?;
    let key = autopilot_profile::normalize_path(workspace);
    let projects = child_table(document.as_table_mut(), "projects")?;
    projects.set_implicit(true);
    let table = child_table(projects, &key)?;
    match field {
        Some(ProjectField::Name(name)) => table["name"] = value(name.as_str()),
        Some(ProjectField::DefaultPipeline(Some(name))) => {
            table["default_pipeline"] = value(name.as_str());
        }
        Some(ProjectField::GithubRepo(Some(repo))) => table["github_repo"] = value(repo.as_str()),
        Some(ProjectField::DefaultPipeline(None)) => {
            table.remove("default_pipeline");
        }
        Some(ProjectField::GithubRepo(None)) => {
            table.remove("github_repo");
        }
        None => {}
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

/// One setting of the pipeline review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewField {
    Mode(autopilot_profile::ReviewMode),
    WindowRuns(u32),
    PeakContextPermille(u32),
    MinHits(u32),
    CostMultiplePercent(u32),
}

/// Change one review setting. A value that cannot be a threshold is refused by
/// the profile when it is read back, and the file is left as it was.
///
/// # Errors
/// Returns an error for an unreadable file or the diagnostic for a bad value.
pub fn edit_review(path: &Path, field: ReviewField) -> Result<Profile, ProfileEditError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut document: DocumentMut = source
        .parse()
        .map_err(|error: toml_edit::TomlError| ProfileEditError::Syntax(error.to_string()))?;
    let table = child_table(document.as_table_mut(), "review")?;
    match field {
        ReviewField::Mode(mode) => {
            table["mode"] = value(match mode {
                autopilot_profile::ReviewMode::Off => "off",
                autopilot_profile::ReviewMode::NotifyOnly => "notify_only",
                autopilot_profile::ReviewMode::NotifyAndTune => "notify_and_tune",
            });
        }
        ReviewField::WindowRuns(n) => table["window_runs"] = value(i64::from(n)),
        ReviewField::PeakContextPermille(n) => {
            table["peak_context_permille"] = value(i64::from(n));
        }
        ReviewField::MinHits(n) => table["min_hits"] = value(i64::from(n)),
        ReviewField::CostMultiplePercent(n) => {
            table["cost_multiple_percent"] = value(i64::from(n));
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

/// Save a custom pipeline. A built-in name is refused by the profile when it
/// is read back, and so is a pipeline the engine could not run in order, so
/// nothing invalid is ever written.
///
/// # Errors
/// Returns an error for an unreadable file, or the diagnostic when the result
/// is invalid. The file is left as it was in that case.
pub fn add_pipeline(
    path: &Path,
    name: &str,
    based_on: &str,
    on: &[Step],
    if_needed: &[Step],
    off: &[Step],
    rounds: Option<u32>,
) -> Result<Profile, ProfileEditError> {
    save_pipeline(path, name, based_on, (on, if_needed, off), rounds, &[])
}

/// Save a pipeline together with the custom steps it introduces, in one
/// write, so a pipeline never points at a step that was not saved.
///
/// # Errors
/// Returns an error for an unreadable file, or the diagnostic when the result
/// is invalid. The file is left as it was in that case.
pub fn save_pipeline(
    path: &Path,
    name: &str,
    based_on: &str,
    (on, if_needed, off): (&[Step], &[Step], &[Step]),
    rounds: Option<u32>,
    new_steps: &[(Step, autopilot_profile::CustomStep)],
) -> Result<Profile, ProfileEditError> {
    let mut document = open_document(path)?;
    for (step, definition) in new_steps {
        let custom = child_table(document.as_table_mut(), "custom_steps")?;
        custom.set_implicit(true);
        let table = child_table(custom, step.as_str())?;
        table["name"] = value(definition.name.as_str());
        table["prompt"] = value(definition.prompt.as_str());
    }
    let pipelines = child_table(document.as_table_mut(), "pipelines")?;
    pipelines.set_implicit(true);
    let table = child_table(pipelines, name)?;
    table["based_on"] = value(based_on);
    for (key, steps) in [("on", on), ("if_needed", if_needed), ("off", off)] {
        let mut list = toml_edit::Array::new();
        for step in steps {
            list.push(step.as_str());
        }
        table[key] = value(list);
    }
    if let Some(rounds) = rounds {
        table["rounds"] = value(i64::from(rounds));
    } else {
        // Saving over a pipeline must not keep a cap the new one dropped.
        table.remove("rounds");
    }
    write_checked(path, &document)
}

/// The first free custom slot, or `None` when all of them are used.
#[must_use]
pub fn free_custom_slot(profile: &Profile) -> Option<Step> {
    (1..=Step::CUSTOM_SLOTS)
        .filter_map(Step::custom)
        .find(|step| profile.custom_step(*step).is_none())
}

/// Define a custom step, or change its name and prompt. Its other settings
/// (model, effort, limits, ask after) are edited like any step's.
///
/// # Errors
/// Returns an error for an unreadable file, or the diagnostic when the name or
/// prompt cannot be used or the step is not a custom one. The file is left as
/// it was in that case.
pub fn set_custom_step(
    path: &Path,
    step: Step,
    name: &str,
    prompt: &str,
) -> Result<Profile, ProfileEditError> {
    let mut document = open_document(path)?;
    let custom = child_table(document.as_table_mut(), "custom_steps")?;
    custom.set_implicit(true);
    let table = child_table(custom, step.as_str())?;
    table["name"] = value(name.trim());
    table["prompt"] = value(prompt);
    write_checked(path, &document)
}

/// Remove a custom step and everything that points at it: its settings and
/// its place in every pipeline. Tickets that already ran it keep their history.
///
/// # Errors
/// Returns an error for an unreadable file or when the result is invalid.
pub fn remove_custom_step(path: &Path, step: Step) -> Result<Profile, ProfileEditError> {
    let mut document = open_document(path)?;
    let key = step.as_str();
    for group in ["custom_steps", "steps"] {
        if let Some(table) = document.get_mut(group).and_then(Item::as_table_mut) {
            table.remove(key);
        }
    }
    if let Some(pipelines) = document.get_mut("pipelines").and_then(Item::as_table_mut) {
        for (_, pipeline) in pipelines.iter_mut() {
            let Some(pipeline) = pipeline.as_table_mut() else {
                continue;
            };
            for list in ["on", "if_needed", "off"] {
                if let Some(array) = pipeline.get_mut(list).and_then(Item::as_array_mut) {
                    array.retain(|entry| entry.as_str() != Some(key));
                }
            }
        }
    }
    write_checked(path, &document)
}

fn open_document(path: &Path) -> Result<DocumentMut, ProfileEditError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    source
        .parse()
        .map_err(|error: toml_edit::TomlError| ProfileEditError::Syntax(error.to_string()))
}

/// Save the document only if it still parses as a valid profile.
fn write_checked(path: &Path, document: &DocumentMut) -> Result<Profile, ProfileEditError> {
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

    // Invariant: adding a project keeps the owner's comments and other keys,
    // the same folder written two ways is one entry, and a default pipeline
    // that does not exist is refused with the file left as it was.
    #[test]
    fn a_project_is_added_once_and_a_bad_default_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(
            &path,
            "# my notes
protected_paths = [\"infra/**\"]
",
        )
        .unwrap();
        edit_project(&path, "D:/work/app", None).unwrap();
        let profile = edit_project(
            &path,
            r"D:\work\app\",
            Some(&ProjectField::DefaultPipeline(Some("bug-fix".into()))),
        )
        .unwrap();
        assert_eq!(profile.projects.len(), 1);
        assert_eq!(
            profile
                .project("D:/work/app")
                .unwrap()
                .default_pipeline
                .as_deref(),
            Some("bug-fix")
        );
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with("# my notes"), "{saved}");
        let before = saved;
        let refused = edit_project(
            &path,
            "D:/work/app",
            Some(&ProjectField::DefaultPipeline(Some("nope".into()))),
        );
        assert!(refused.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    // Invariant: a saved custom pipeline reads back with the steps it names,
    // a built-in name or an impossible order is refused, and the file keeps
    // its comments and is left as it was after a refusal.
    #[test]
    fn a_custom_pipeline_is_saved_and_a_bad_one_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(&path, "# my notes\n").unwrap();
        let profile = add_pipeline(
            &path,
            "mine",
            "bug-fix",
            &[],
            &[],
            &[Step::Clarify],
            Some(2),
        )
        .unwrap();
        let mine = profile.pipeline("mine").unwrap();
        assert_eq!(mine.off(), [Step::Clarify, Step::Design]);
        assert_eq!(mine.rounds, Some(2));
        assert!(fs::read_to_string(&path).unwrap().contains("# my notes"));
        let before = fs::read_to_string(&path).unwrap();
        assert!(add_pipeline(&path, "full", "full", &[], &[], &[], None).is_err());
        assert!(add_pipeline(&path, "bad", "full", &[], &[], &[Step::Plan], None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    // Invariant: a review setting is written once and read back, and a value
    // that cannot be a threshold is refused with the file unchanged. The
    // default mode is off, so a profile without a [review] section never runs
    // a review.
    #[test]
    fn review_settings_default_off_and_bad_values_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(&path, "# notes\nprotected_paths = []\n").unwrap();
        assert_eq!(
            Profile::parse(&path, "protected_paths = []\n")
                .unwrap()
                .review
                .mode,
            autopilot_profile::ReviewMode::Off
        );
        let profile = edit_review(
            &path,
            ReviewField::Mode(autopilot_profile::ReviewMode::NotifyOnly),
        )
        .unwrap();
        assert_eq!(
            profile.review.mode,
            autopilot_profile::ReviewMode::NotifyOnly
        );
        assert_eq!(profile.review.min_hits, 3);
        let before = fs::read_to_string(&path).unwrap();
        for bad in [
            ReviewField::MinHits(0),
            ReviewField::MinHits(9),
            ReviewField::WindowRuns(0),
            ReviewField::PeakContextPermille(1001),
            ReviewField::PeakContextPermille(0),
            ReviewField::CostMultiplePercent(50),
        ] {
            assert!(edit_review(&path, bad).is_err(), "{bad:?}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
}

#[cfg(test)]
mod custom_step_edit_tests {
    use super::*;

    // Invariant: adding a step writes the definition and keeps the owner's
    // comments, a refused definition writes nothing, and removing a step also
    // takes it out of every pipeline. Without the last, deleting a step the
    // owner had already put in a pipeline would leave a profile that no longer
    // parses.
    #[test]
    fn a_custom_step_is_added_edited_and_removed_with_its_pipeline_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(&path, "# my notes\nprotected_paths = [\"infra/**\"]\n").unwrap();
        let step = Step::Custom(1);

        let profile =
            set_custom_step(&path, step, "Security review", "Look for injection.").unwrap();
        assert_eq!(profile.custom_step(step).unwrap().name, "Security review");
        assert!(fs::read_to_string(&path).unwrap().contains("# my notes"));
        assert_eq!(free_custom_slot(&profile), Some(Step::Custom(2)));

        let before = fs::read_to_string(&path).unwrap();
        assert!(set_custom_step(&path, step, "", "x").is_err());
        assert!(set_custom_step(&path, Step::Plan, "Plan", "x").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        let profile = set_custom_step(&path, step, "Security audit", "Look harder.").unwrap();
        assert_eq!(profile.custom_step(step).unwrap().prompt, "Look harder.");

        add_pipeline(&path, "careful", "full", &[step], &[], &[], None).unwrap();
        let profile = remove_custom_step(&path, step).unwrap();
        assert!(profile.custom_step(step).is_none());
        assert_eq!(profile.pipeline("careful").unwrap().custom_steps(), []);
        assert!(!fs::read_to_string(&path).unwrap().contains("custom_1"));
    }
}
