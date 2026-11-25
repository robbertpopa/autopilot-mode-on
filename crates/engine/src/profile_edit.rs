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
    /// The request was not made from something the scan found, or would
    /// replace a check the owner already has. Nothing was written.
    #[error("{0}")]
    Refused(String),
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
        // "default" is the word the window shows for no choice, so picking it
        // takes the key out and the step goes back to the profile's model.
        StepField::Model(model) if model == "default" => {
            table.remove("model");
        }
        StepField::Effort(effort) if effort == "default" => {
            table.remove("effort");
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

/// One spend limit, in micro-dollars.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendField {
    /// The most all paid calls may cost together.
    Total(u64),
    /// The most one call may cost when its step sets no budget.
    PerCall(u64),
}

/// Change a spend limit. This is the only writer of `[spend]`: it is reached
/// from Settings, never from a pipeline draft, a proposal or a tuning.
///
/// # Errors
/// Returns an error for an unreadable file, or a refusal for a zero limit. The
/// file is left as it was in that case.
pub fn edit_spend(path: &Path, field: SpendField) -> Result<Profile, ProfileEditError> {
    let (key, micro) = match field {
        SpendField::Total(micro) => ("total_microusd", micro),
        SpendField::PerCall(micro) => ("call_microusd", micro),
    };
    if micro == 0 {
        return Err(ProfileEditError::Refused(
            "a spend limit must be more than zero".into(),
        ));
    }
    let micro = i64::try_from(micro)
        .map_err(|_| ProfileEditError::Syntax("the limit is too large".into()))?;
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut document: DocumentMut = source
        .parse()
        .map_err(|error: toml_edit::TomlError| ProfileEditError::Syntax(error.to_string()))?;
    child_table(document.as_table_mut(), "spend")?[key] = value(micro);
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

/// Give a custom pipeline a new name, and carry it to everything that names
/// it: a project's default, another pipeline's base and the list order.
///
/// # Errors
/// Returns an error for a built-in or unknown pipeline, a name already used,
/// or an invalid result. The file is left as it was in that case.
pub fn rename_pipeline(path: &Path, from: &str, to: &str) -> Result<Profile, ProfileEditError> {
    let mut document = open_document(path)?;
    let pipelines = child_table(document.as_table_mut(), "pipelines")?;
    if pipelines.contains_key(to) {
        return Err(ProfileEditError::Refused(format!(
            "{to:?} is already a pipeline"
        )));
    }
    let Some(item) = pipelines.remove(from) else {
        return Err(ProfileEditError::Refused(format!(
            "{from:?} is not a custom pipeline, so it cannot be renamed"
        )));
    };
    pipelines.insert(to, item);
    for (_, other) in pipelines.iter_mut() {
        if let Some(table) = other.as_table_mut()
            && table.get("based_on").and_then(Item::as_str) == Some(from)
        {
            table["based_on"] = value(to);
        }
    }
    replace_references(&mut document, from, Some(to));
    write_checked(path, &document)
}

/// Remove a custom pipeline. A project that used it as its default goes back
/// to the profile's default; tickets keep the steps they were made with.
///
/// # Errors
/// Returns an error for a built-in or unknown pipeline, one another pipeline
/// is based on, or an invalid result. The file is left as it was in that case.
pub fn delete_pipeline(path: &Path, name: &str) -> Result<Profile, ProfileEditError> {
    let mut document = open_document(path)?;
    let pipelines = child_table(document.as_table_mut(), "pipelines")?;
    if pipelines.remove(name).is_none() {
        return Err(ProfileEditError::Refused(format!(
            "{name:?} is not a custom pipeline, so it cannot be deleted"
        )));
    }
    let based: Vec<String> = pipelines
        .iter()
        .filter(|(_, each)| each.get("based_on").and_then(Item::as_str) == Some(name))
        .map(|(key, _)| key.to_owned())
        .collect();
    if !based.is_empty() {
        return Err(ProfileEditError::Refused(format!(
            "{} is based on {name}: delete or change that one first",
            based.join(", ")
        )));
    }
    replace_references(&mut document, name, None);
    write_checked(path, &document)
}

/// Save the order the Pipelines list shows.
///
/// # Errors
/// Returns an error for an unreadable file or an invalid result.
pub fn set_pipeline_order(path: &Path, order: &[String]) -> Result<Profile, ProfileEditError> {
    let mut document = open_document(path)?;
    let mut list = toml_edit::Array::new();
    for name in order {
        list.push(name.as_str());
    }
    document["pipeline_order"] = value(list);
    write_checked(path, &document)
}

/// Point every project default and the list order at `to`, or drop them when
/// there is none.
fn replace_references(document: &mut DocumentMut, from: &str, to: Option<&str>) {
    if let Some(projects) = document.get_mut("projects").and_then(Item::as_table_mut) {
        for (_, project) in projects.iter_mut() {
            let Some(table) = project.as_table_mut() else {
                continue;
            };
            if table.get("default_pipeline").and_then(Item::as_str) != Some(from) {
                continue;
            }
            match to {
                Some(to) => table["default_pipeline"] = value(to),
                None => {
                    table.remove("default_pipeline");
                }
            }
        }
    }
    if let Some(order) = document
        .get_mut("pipeline_order")
        .and_then(Item::as_array_mut)
    {
        let kept: Vec<String> = order
            .iter()
            .filter_map(|entry| entry.as_str())
            .filter_map(|entry| match (entry == from, to) {
                (false, _) => Some(entry.to_owned()),
                (true, Some(to)) => Some(to.to_owned()),
                (true, None) => None,
            })
            .collect();
        order.clear();
        for entry in kept {
            order.push(entry);
        }
    }
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

/// Add a command the scan found as a check in `profile.toml`. The command is
/// matched against a fresh scan of the workspace, so only a command the scan
/// itself offers can be added, never text a client supplies. A check the owner
/// already has under that name is never replaced.
///
/// # Errors
/// Returns an error when the scan does not offer the command, the name is
/// taken, the file cannot be read or written, or the result is invalid.
pub fn add_detected_check(
    path: &Path,
    workspace: &str,
    command: &str,
) -> Result<Profile, ProfileEditError> {
    let card = crate::scan_workspace(Path::new(&autopilot_profile::normalize_path(workspace)));
    let Some(found) = card.commands.iter().find(|found| found.command == command) else {
        return Err(ProfileEditError::Refused(
            "the scan does not offer that command".into(),
        ));
    };
    // Detected commands are plain words, so splitting on spaces is exact.
    let mut words = found.command.split(' ');
    let Some(executable) = words.next() else {
        return Err(ProfileEditError::Refused("the command is empty".into()));
    };
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut document: DocumentMut = source
        .parse()
        .map_err(|error: toml_edit::TomlError| ProfileEditError::Syntax(error.to_string()))?;
    let checks = child_table(document.as_table_mut(), "checks")?;
    checks.set_implicit(true);
    if checks.contains_key(found.kind) {
        return Err(ProfileEditError::Refused(format!(
            "a check named {} is already set; edit it in profile.toml",
            found.kind
        )));
    }
    let table = child_table(checks, found.kind)?;
    table["executable"] = value(executable);
    let mut args = toml_edit::Array::new();
    for word in words {
        args.push(word);
    }
    table["args"] = value(args);
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

    const WITH_PIPELINES: &str = "[pipelines.mine]\nbased_on = \"full\"\noff = [\"design\"]\n\n[pipelines.child]\nbased_on = \"mine\"\n\n[projects.\"/work/app\"]\ndefault_pipeline = \"mine\"\n";

    // Invariant: a built-in pipeline can be neither renamed nor deleted, and
    // deleting a custom one leaves no project pointing at a pipeline that is
    // gone. A change that dropped the built-in check would let the window
    // delete `full`, which tickets without a recorded choice fall back to.
    #[test]
    fn pipelines_are_renamed_and_deleted_without_leaving_dangling_names() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(&path, WITH_PIPELINES).unwrap();
        assert!(delete_pipeline(&path, "full").is_err());
        assert!(rename_pipeline(&path, "full", "other").is_err());
        assert!(rename_pipeline(&path, "mine", "full").is_err());
        assert!(
            delete_pipeline(&path, "mine").is_err(),
            "child is based on it"
        );
        let profile = rename_pipeline(&path, "mine", "theirs").unwrap();
        assert!(profile.pipeline("theirs").is_some());
        assert_eq!(
            profile.pipeline("child").map(|each| each.off()),
            profile.pipeline("theirs").map(|each| each.off())
        );
        let project = profile.projects.values().next().unwrap();
        assert_eq!(project.default_pipeline.as_deref(), Some("theirs"));
        delete_pipeline(&path, "child").unwrap();
        let profile = delete_pipeline(&path, "theirs").unwrap();
        assert!(profile.pipeline("theirs").is_none());
        assert_eq!(
            profile.projects.values().next().unwrap().default_pipeline,
            None
        );
    }

    // Invariant: the saved order is what the list shows, after a restart too,
    // and a name that no longer exists in it is ignored rather than an error.
    #[test]
    fn the_pipeline_order_is_kept_and_forgives_unknown_names() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        fs::write(&path, WITH_PIPELINES).unwrap();
        let order: Vec<String> = ["mine", "gone", "ask-only"].map(String::from).to_vec();
        let profile = set_pipeline_order(&path, &order).unwrap();
        let names: Vec<String> = load_profile(&path)
            .unwrap()
            .pipelines()
            .into_iter()
            .map(|each| each.name)
            .collect();
        assert_eq!(names[..3], ["mine", "ask-only", "full"]);
        assert_eq!(profile.pipelines().len(), names.len());
    }

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

    // Invariant: choosing "default" for a model or an effort removes the key,
    // so a step can go back to the profile's choice. Without it the window's
    // list had no way back once a model was picked, and "default" written as a
    // model name would be rejected by every agent.
    #[test]
    fn default_takes_the_model_and_effort_keys_out() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        edit_step(&path, Step::Plan, &StepField::Model("opus-5-5".into())).unwrap();
        edit_step(&path, Step::Plan, &StepField::Effort("high".into())).unwrap();
        edit_step(&path, Step::Plan, &StepField::Model("default".into())).unwrap();
        let profile = edit_step(&path, Step::Plan, &StepField::Effort("default".into())).unwrap();
        let plan = step_of(&profile, Step::Plan);
        assert_eq!(
            plan.model.as_deref(),
            Some("opus"),
            "back to the step's own default"
        );
        assert_eq!(plan.effort, None);
        assert!(!fs::read_to_string(&path).unwrap().contains("default"));
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

    // Invariant: only a command the scan offers becomes a check, it is written
    // as an executable and its arguments, and a check the owner already has is
    // kept. Accepting any string would let a client add a command the project
    // never suggested; replacing would overwrite the owner's own check.
    #[test]
    fn a_detected_command_becomes_a_check_and_never_replaces_one() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("app");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("Cargo.toml"), "").unwrap();
        let path = directory.path().join("profile.toml");
        let name = workspace.to_string_lossy().into_owned();
        let profile = add_detected_check(&path, &name, "cargo test").unwrap();
        let check = &profile.checks["test"];
        assert_eq!(check.executable, "cargo");
        assert_eq!(check.args, ["test"]);
        let lint = "cargo clippy --all-targets -- -D warnings";
        let profile = add_detected_check(&path, &name, lint).unwrap();
        assert_eq!(
            profile.checks["lint"].args,
            ["clippy", "--all-targets", "--", "-D", "warnings"]
        );
        let again = add_detected_check(&path, &name, "cargo test");
        assert!(
            matches!(again, Err(ProfileEditError::Refused(_))),
            "{again:?}"
        );
        let other = add_detected_check(&path, &name, "rm -rf /");
        assert!(
            matches!(other, Err(ProfileEditError::Refused(_))),
            "{other:?}"
        );
        assert_eq!(load_profile(&path).unwrap().checks.len(), 2);
    }
}
