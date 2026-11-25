//! A pipeline drafted by an agent from the owner's description.
//!
//! The draft is untrusted data. It is read from the agent's raw reply by this
//! module, never from a typed struct, so a key the draft should not have is
//! seen instead of silently dropped. Only a few things can come through:
//! a new name, which pipeline it starts from, a mode per step, a build round
//! cap and short reasons. Anything that touches what asks, what is protected
//! or what is spent is refused, and so is a draft that makes Design or the
//! pull request weaker than the pipeline it starts from.
//!
//! AGENTS.md says unknown fields from an agent are kept and ignored. That
//! still holds for harmless keys: an unknown key is ignored. The keys on
//! `FORBIDDEN` are different: they name a policy, so a draft that carries one
//! is refused, because an agent reaching for them is the thing to stop.
//!
//! The owner's description is never an input here. The validator sees only
//! the reply and the profile, so nothing in the description can steer it.

use std::{collections::BTreeMap, path::Path};

use autopilot_agent::{Agent, AgentEvent, DraftRequest};
use autopilot_core::Step;
use autopilot_profile::{CustomStep, Pipeline, Profile, StepMode};
use serde_json::Value;

use crate::Error;

/// The most of a reply that is read. A pipeline is a few lines.
const MAX_REPLY_BYTES: usize = 16 * 1024;
const MAX_REASONS: usize = 12;
const MAX_REASON_CHARS: usize = 240;

/// Keys that name a policy. Any of them, at any depth, refuses the draft.
const FORBIDDEN: &[&str] = &[
    "ask_after",
    "forced",
    "allow_for_project",
    "protected_paths",
    "budgets",
    "spend",
    "limits",
    "turns",
    "model",
    "effort",
    "auth",
    "agent",
    "when_unsure",
    "checks",
    "branch_rules",
];

/// A draft that passed every rule, resolved to the pipeline it would be.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipelineDraft {
    pub name: String,
    pub based_on: String,
    pub pipeline: Pipeline,
    /// One short, plain reason per step the agent explained.
    pub reasons: Vec<(Step, String)>,
    /// Steps the draft defines, each in the slot it would take. They are saved
    /// with the pipeline, and run after the build.
    pub new_steps: Vec<(Step, CustomStep)>,
    /// True when the draft changes the pipeline it names instead of adding one.
    pub replaces: bool,
}

/// The most new steps one draft may add.
const MAX_NEW_STEPS: usize = 3;

fn forbidden_key(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let lower = key.to_lowercase();
                if FORBIDDEN.contains(&lower.as_str()) || lower.ends_with("_microusd") {
                    return Some(key.clone());
                }
                if let Some(found) = forbidden_key(inner) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(forbidden_key),
        _ => None,
    }
}

fn steps(value: Option<&Value>, key: &str, known: &Profile) -> Result<Vec<Step>, String> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    let list = value
        .as_array()
        .ok_or_else(|| format!("{key} must be a list of step names"))?;
    list.iter()
        .map(|item| {
            let name = item
                .as_str()
                .ok_or_else(|| format!("{key} must hold step names"))?;
            Step::EVERY
                .into_iter()
                .find(|step| {
                    step.as_str() == name
                        && (!step.is_custom() || known.custom_step(*step).is_some())
                })
                .ok_or_else(|| format!("{name:?} is not a step"))
        })
        .collect()
}

/// Plain text only: control characters removed, key-shaped words replaced,
/// cut to a short length.
fn clean_reason(text: &str) -> String {
    let plain: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    autopilot_tracker::redact(plain.trim())
        .chars()
        .take(MAX_REASON_CHARS)
        .collect()
}

fn reasons(value: Option<&Value>) -> Vec<(Step, String)> {
    let Some(Value::Object(map)) = value else {
        return vec![];
    };
    Step::ALL
        .into_iter()
        .filter_map(|step| {
            let text = map.get(step.as_str())?.as_str()?;
            let clean = clean_reason(text);
            (!clean.is_empty()).then_some((step, clean))
        })
        .take(MAX_REASONS)
        .collect()
}

/// How much of a step a mode keeps: on, then if needed, then off.
const fn strength(mode: StepMode) -> u8 {
    match mode {
        StepMode::On => 2,
        StepMode::IfNeeded => 1,
        StepMode::Off => 0,
    }
}

/// The steps a draft defines, each given the first free slot. The prompt becomes
/// an instruction once the owner saves it, so it is checked like one the owner
/// typed, and refused if it holds something shaped like a key.
fn new_steps(value: Option<&Value>, profile: &Profile) -> Result<Vec<(Step, CustomStep)>, String> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    let list = value
        .as_array()
        .ok_or("new_steps must be a list of steps")?;
    if list.len() > MAX_NEW_STEPS {
        return Err(format!("a draft may add at most {MAX_NEW_STEPS} new steps"));
    }
    let mut free = (1..=Step::CUSTOM_SLOTS)
        .filter_map(Step::custom)
        .filter(|step| profile.custom_step(*step).is_none());
    let mut added = Vec::new();
    for item in list {
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        let prompt = item
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        CustomStep::check(name, prompt).map_err(|why| format!("a new step: {why}"))?;
        if autopilot_tracker::redact(prompt) != prompt {
            return Err("a new step prompt holds something that looks like a key".into());
        }
        let slot = free
            .next()
            .ok_or("there is no free custom step slot left")?;
        added.push((
            slot,
            CustomStep {
                name: name.trim().to_owned(),
                prompt: prompt.to_owned(),
            },
        ));
    }
    Ok(added)
}

/// Check a reply against every rule and resolve it against the profile.
///
/// # Errors
/// Returns a sentence naming the rule the draft broke. Nothing is stored.
pub fn validate_draft(raw: &str, profile: &Profile) -> Result<PipelineDraft, String> {
    validate_draft_for(raw, profile, None)
}

/// Like [`validate_draft`], for a draft that changes the custom pipeline
/// `editing` instead of adding one. It must keep that name, and it may not
/// turn Design or the pull request down from what that pipeline does now.
///
/// # Errors
/// Returns a sentence naming the rule the draft broke. Nothing is stored.
#[allow(
    clippy::too_many_lines,
    reason = "one check per rule, in the order a reader meets them"
)]
pub fn validate_draft_for(
    raw: &str,
    profile: &Profile,
    editing: Option<&str>,
) -> Result<PipelineDraft, String> {
    if raw.len() > MAX_REPLY_BYTES {
        return Err(format!("the reply is over {MAX_REPLY_BYTES} bytes"));
    }
    let value: Value =
        serde_json::from_str(raw).map_err(|error| format!("the reply is not JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or("the reply must be one JSON object")?;
    if let Some(key) = forbidden_key(&value) {
        return Err(format!(
            "the draft names {key:?}, which a pipeline cannot change"
        ));
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or("the draft needs a name")?;
    autopilot_profile::check_name(name)?;
    // Two arms with different checks read more clearly as a match than as
    // an `if let` with a long else.
    #[allow(clippy::single_match_else)]
    let replaces = match editing {
        Some(wanted) => {
            let existing = profile
                .pipeline(wanted)
                .filter(|pipeline| !pipeline.builtin)
                .ok_or_else(|| {
                    format!("{wanted:?} is not a custom pipeline that can be changed")
                })?;
            if existing.name != name {
                return Err(format!("the draft must keep the name {wanted:?}"));
            }
            true
        }
        None => {
            if profile.pipeline(name).is_some() {
                return Err(format!("a pipeline named {name:?} already exists"));
            }
            false
        }
    };
    // Changing a pipeline starts from what it is now, so a step the draft does
    // not mention stays as it was.
    let based_on = if replaces {
        name
    } else {
        object
            .get("based_on")
            .and_then(Value::as_str)
            .unwrap_or(autopilot_profile::DEFAULT_PIPELINE)
    };
    let base = profile
        .pipeline(based_on)
        .ok_or_else(|| format!("based_on {based_on:?} is not a known pipeline"))?;
    let added = new_steps(object.get("new_steps"), profile)?;
    let staged = profile.with_custom_steps(&added);
    let mut on = steps(object.get("on"), "on", &staged)?;
    for (step, _) in &added {
        if !on.contains(step) {
            on.push(*step);
        }
    }
    let if_needed = steps(object.get("if_needed"), "if_needed", &staged)?;
    let off = steps(object.get("off"), "off", &staged)?;
    let mut seen: BTreeMap<Step, usize> = BTreeMap::new();
    for step in on.iter().chain(&if_needed).chain(&off) {
        *seen.entry(*step).or_default() += 1;
    }
    if let Some((step, _)) = seen.iter().find(|(_, count)| **count > 1) {
        return Err(format!("{} is listed more than once", step.as_str()));
    }
    let rounds = match object.get("rounds") {
        None | Some(Value::Null) => None,
        Some(number) => {
            let rounds = number
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("rounds must be a whole number")?;
            let allowed = profile
                .step(Step::BuildCheck)
                .map_or(3, |settings| settings.limits.rounds);
            if rounds == 0 || rounds > allowed {
                return Err(format!("rounds must be between 1 and {allowed}"));
            }
            Some(rounds)
        }
    };
    let pipeline = staged.build_pipeline(name, based_on, &on, &if_needed, &off, rounds)?;
    // The agent may not weaken what asks. Design and the pull request are the
    // steps that always ask when they run, so it cannot make them run less
    // than the pipeline it starts from does.
    for step in [Step::Design, Step::PullRequest] {
        if strength(pipeline.mode(step)) < strength(base.mode(step)) {
            return Err(format!(
                "a draft cannot turn {} down from {based_on}",
                step.as_str()
            ));
        }
    }
    Ok(PipelineDraft {
        name: name.to_owned(),
        based_on: based_on.to_owned(),
        pipeline,
        reasons: reasons(object.get("reasons")),
        new_steps: added,
        replaces,
    })
}

/// What the agent is told about the pipeline being changed and the steps that
/// exist, as plain text. The first line names the pipeline, so the draft keeps it.
#[must_use]
pub fn describe_current(profile: &Profile, editing: Option<&str>) -> String {
    let mut lines = Vec::new();
    if let Some(pipeline) = editing.and_then(|name| profile.pipeline(name)) {
        lines.push(format!("editing: {}", pipeline.name));
        let modes: Vec<String> = Step::EVERY
            .into_iter()
            .filter(|step| !step.is_custom() || profile.custom_step(*step).is_some())
            .map(|step| {
                let mode = match pipeline.mode(step) {
                    StepMode::On => "on",
                    StepMode::IfNeeded => "if_needed",
                    StepMode::Off => "off",
                };
                format!("{} {mode}", step.as_str())
            })
            .collect();
        lines.push(format!("steps: {}", modes.join(", ")));
    }
    for (step, definition) in profile.custom_steps() {
        lines.push(format!(
            "custom step {}: {}",
            step.as_str(),
            definition.name
        ));
    }
    lines.join("\n")
}

/// Ask the agent for a draft and validate what it answers.
///
/// # Errors
/// Returns an error if the agent cannot draft or fails, and a refusal
/// sentence if the draft breaks a rule.
pub fn draft_pipeline(
    profile: &Profile,
    description: &str,
    agent: &impl Agent,
) -> Result<PipelineDraft, Error> {
    draft_pipeline_for(profile, description, None, agent)
}

/// Like [`draft_pipeline`], for a draft that changes the custom pipeline
/// `editing`.
///
/// # Errors
/// Returns an error if the agent cannot draft or fails, and a refusal
/// sentence if the draft breaks a rule.
pub fn draft_pipeline_for(
    profile: &Profile,
    description: &str,
    editing: Option<&str>,
    agent: &impl Agent,
) -> Result<PipelineDraft, Error> {
    let known: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
    let current = describe_current(profile, editing);
    let run = agent
        .start_draft(DraftRequest {
            description,
            known: &known,
            current: &current,
        })
        .map_err(Error::Agent)?;
    let mut raw = None;
    for item in run {
        match item {
            AgentEvent::DraftResult(text) => {
                raw = Some(text);
                break;
            }
            AgentEvent::Failed(error) => return Err(Error::Agent(error)),
            AgentEvent::Stopped(reason) => return Err(Error::Stopped(reason)),
            _ => {}
        }
    }
    let raw = raw.ok_or_else(|| Error::Draft("the agent gave no draft".into()))?;
    validate_draft_for(&raw, profile, editing).map_err(Error::Draft)
}

/// Save a draft the owner approved. It is checked again against the profile as
/// it is now, since the file may have changed since the draft was made, and
/// written only through `add_pipeline`, which parses the result first.
///
/// # Errors
/// Returns an error if the draft no longer passes or the file cannot be written.
pub fn save_draft(profile_path: &Path, draft: &PipelineDraft) -> Result<Profile, Error> {
    let current =
        crate::load_profile(profile_path).map_err(|error| Error::Draft(error.to_string()))?;
    let existing = current.pipeline(&draft.name);
    if draft.replaces {
        if existing.is_none_or(|pipeline| pipeline.builtin) {
            return Err(Error::Draft(format!(
                "{:?} is no longer a custom pipeline that can be changed",
                draft.name
            )));
        }
    } else if existing.is_some() {
        return Err(Error::Draft(format!(
            "a pipeline named {:?} appeared since the draft",
            draft.name
        )));
    }
    if let Some((step, _)) = draft
        .new_steps
        .iter()
        .find(|(step, _)| current.custom_step(*step).is_some())
    {
        return Err(Error::Draft(format!(
            "{} was taken since the draft",
            step.as_str()
        )));
    }
    // Custom steps listed only when on: an unlisted one is off, and a listed
    // off step that no longer exists would only be noise.
    let list = |wanted: StepMode| -> Vec<Step> {
        Step::EVERY
            .into_iter()
            .filter(|step| !step.is_custom() || wanted == StepMode::On)
            .filter(|step| draft.pipeline.mode(*step) == wanted)
            .collect()
    };
    crate::profile_edit::save_pipeline(
        profile_path,
        &draft.name,
        autopilot_profile::DEFAULT_PIPELINE,
        (
            &list(StepMode::On),
            &list(StepMode::IfNeeded),
            &list(StepMode::Off),
        ),
        draft.pipeline.rounds,
        &draft.new_steps,
    )
    .map_err(|error| Error::Draft(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::parse("p.toml", "").unwrap()
    }

    // Invariant: each of these replies is refused, whatever else is in it, so
    // an agent that reaches for a policy, sends junk or sends too much never
    // gets a pipeline. None may panic.
    #[test]
    fn hostile_replies_are_refused_without_a_panic() {
        let profile = profile();
        let huge = format!("{{\"name\":\"x\",\"pad\":\"{}\"}}", "a".repeat(20_000));
        let cases: Vec<String> = vec![
            String::new(),
            "not json".into(),
            "[1,2]".into(),
            "null".into(),
            "{\"name\":3}".into(),
            "{\"name\":\"Bad Name\"}".into(),
            "{\"name\":\"full\"}".into(),
            "{\"name\":\"ok\",\"ask_after\":false}".into(),
            "{\"name\":\"ok\",\"nested\":{\"deep\":[{\"protected_paths\":[]}]}}".into(),
            "{\"name\":\"ok\",\"extra_microusd\":1}".into(),
            "{\"name\":\"ok\",\"spend\":{\"total_microusd\":99000000}}".into(),
            "{\"name\":\"ok\",\"nested\":{\"call_microusd\":99000000}}".into(),
            "{\"name\":\"ok\",\"ASK_AFTER\":1}".into(),
            "{\"name\":\"ok\",\"based_on\":\"nope\"}".into(),
            "{\"name\":\"ok\",\"off\":[\"plan\"]}".into(),
            "{\"name\":\"ok\",\"off\":[\"nothing\"]}".into(),
            "{\"name\":\"ok\",\"off\":\"design\"}".into(),
            "{\"name\":\"ok\",\"on\":[\"clarify\"],\"off\":[\"clarify\"]}".into(),
            "{\"name\":\"ok\",\"rounds\":0}".into(),
            "{\"name\":\"ok\",\"rounds\":9}".into(),
            "{\"name\":\"ok\",\"rounds\":-1}".into(),
            "{\"name\":\"ok\",\"rounds\":1.5}".into(),
            huge,
        ];
        for raw in cases {
            assert!(validate_draft(&raw, &profile).is_err(), "{raw:.80}");
        }
    }

    // Invariant: an agent cannot turn Design or the pull request down from the
    // pipeline it starts from, but may turn other steps down, and a harmless
    // unknown key is ignored. This is what keeps "the agent cannot unlock
    // Design or PR" true without refusing every honest draft.
    #[test]
    fn a_draft_cannot_weaken_design_or_the_pull_request() {
        let profile = profile();
        for off in ["design", "pull_request"] {
            let raw = format!("{{\"name\":\"ok\",\"based_on\":\"ui-change\",\"off\":[\"{off}\"]}}");
            // ui-change keeps Design on; turning it off is a weaker draft. The
            // pull request is on in every built-in, so turning it off is too.
            assert!(validate_draft(&raw, &profile).is_err(), "{off}");
        }
        let fine = r#"{"name":"quick-docs","based_on":"bug-fix","off":["clarify"],"rounds":2,
            "comment":"ignored","reasons":{"clarify":"Docs changes rarely need questions.\u0007"}}"#;
        let draft = validate_draft(fine, &profile).unwrap();
        assert_eq!(draft.name, "quick-docs");
        assert_eq!(draft.pipeline.rounds, Some(2));
        assert_eq!(draft.pipeline.mode(Step::Clarify), StepMode::Off);
        assert_eq!(draft.pipeline.mode(Step::Design), StepMode::Off);
        assert_eq!(draft.reasons.len(), 1);
        assert!(!draft.reasons[0].1.contains('\u{7}'));
    }

    // Invariant: reasons are plain, bounded and key-free, so the preview can
    // never carry a credential an agent echoed or a control sequence.
    #[test]
    fn reasons_are_redacted_and_bounded() {
        let token = format!("ghp_{}", "a".repeat(36));
        let raw = format!(
            "{{\"name\":\"ok\",\"reasons\":{{\"plan\":\"use {token} here\",\"clarify\":\"{}\"}}}}",
            "x".repeat(1_000)
        );
        let draft = validate_draft(&raw, &profile()).unwrap();
        for (_, text) in &draft.reasons {
            assert!(!text.contains(&token), "{text}");
            assert!(text.chars().count() <= MAX_REASON_CHARS);
        }
    }
}

#[cfg(test)]
mod new_step_tests {
    use super::*;

    fn profile() -> Profile {
        Profile::parse("p.toml", "").unwrap()
    }

    // Invariant: a draft may define new steps, which are saved with the
    // pipeline in the first free slot and run on it, so "add a security
    // review" works without the owner typing TOML. Without this the agent
    // could only reshuffle the six steps that already exist.
    #[test]
    fn a_draft_can_add_steps_and_they_take_free_slots() {
        let raw = r#"{"name":"careful","new_steps":[
            {"name":"Security review","prompt":"Look for injection."},
            {"name":"Docs check","prompt":"Check the docs match."}]}"#;
        let draft = validate_draft(raw, &profile()).unwrap();
        let slots: Vec<Step> = draft.new_steps.iter().map(|(step, _)| *step).collect();
        assert_eq!(slots, [Step::Custom(1), Step::Custom(2)]);
        assert_eq!(draft.pipeline.custom_steps(), slots);
        assert!(!draft.replaces);
    }

    // Invariant: every way a new step could smuggle something in is refused:
    // a key-shaped prompt, a policy key, too many steps, an empty prompt, a
    // reference to a step nobody defined. The prompt becomes an instruction
    // once saved, so it gets the strictest reading.
    #[test]
    fn bad_new_steps_are_refused() {
        let token = format!("ghp_{}", "a".repeat(36));
        let many = r#"{"name":"x","new_steps":[
            {"name":"a","prompt":"p"},{"name":"b","prompt":"p"},
            {"name":"c","prompt":"p"},{"name":"d","prompt":"p"}]}"#;
        let cases = [
            format!(
                "{{\"name\":\"x\",\"new_steps\":[{{\"name\":\"a\",\"prompt\":\"use {token}\"}}]}}"
            ),
            "{\"name\":\"x\",\"new_steps\":[{\"name\":\"a\",\"prompt\":\"p\",\"model\":\"opus\"}]}"
                .into(),
            "{\"name\":\"x\",\"new_steps\":[{\"name\":\"a\",\"prompt\":\"\"}]}".into(),
            "{\"name\":\"x\",\"new_steps\":[{\"name\":\"\",\"prompt\":\"p\"}]}".into(),
            "{\"name\":\"x\",\"new_steps\":\"a\"}".into(),
            "{\"name\":\"x\",\"on\":[\"custom_4\"]}".into(),
            many.into(),
        ];
        for raw in cases {
            assert!(validate_draft(&raw, &profile()).is_err(), "{raw:.90}");
        }
    }

    // Invariant: changing a pipeline keeps its name, works only on a custom
    // one, and starts from what it does now, so a step the agent does not
    // mention is not quietly reset. A built-in cannot be changed in place.
    #[test]
    fn changing_a_pipeline_keeps_its_name_and_its_other_steps() {
        let source = "[pipelines.mine]\nbased_on = \"bug-fix\"\noff = [\"clarify\"]\n";
        let profile = Profile::parse("p.toml", source).unwrap();
        let raw = r#"{"name":"mine","off":["plan"]}"#;
        // Plan cannot be off while build runs, so this one is refused.
        assert!(validate_draft_for(raw, &profile, Some("mine")).is_err());

        let raw = r#"{"name":"mine","if_needed":["design"]}"#;
        let draft = validate_draft_for(raw, &profile, Some("mine")).unwrap();
        assert!(draft.replaces);
        assert_eq!(draft.pipeline.mode(Step::Design), StepMode::IfNeeded);
        assert_eq!(draft.pipeline.mode(Step::Clarify), StepMode::Off);

        for (raw, editing) in [
            (r#"{"name":"other"}"#, "mine"),
            (r#"{"name":"full"}"#, "full"),
            (r#"{"name":"mine"}"#, "missing"),
        ] {
            assert!(
                validate_draft_for(raw, &profile, Some(editing)).is_err(),
                "{raw}"
            );
        }
        // Without "editing", an existing name is still refused.
        assert!(validate_draft(r#"{"name":"mine"}"#, &profile).is_err());
    }

    // Invariant: saving a draft with new steps writes the steps and the
    // pipeline together, and a later change replaces the pipeline in place
    // while keeping what it did not touch, so what the page shows is what is
    // on disk.
    #[test]
    fn saving_writes_steps_and_pipeline_together_and_replaces_cleanly() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        let raw = r#"{"name":"careful","rounds":2,"new_steps":[{"name":"Security review","prompt":"Look."}]}"#;
        let draft = validate_draft(raw, &crate::load_profile(&path).unwrap()).unwrap();
        let saved = save_draft(&path, &draft).unwrap();
        let step = Step::Custom(1);
        assert_eq!(saved.custom_step(step).unwrap().name, "Security review");
        let pipeline = saved.pipeline("careful").unwrap();
        assert_eq!(pipeline.custom_steps(), [step]);
        assert_eq!(pipeline.rounds, Some(2));
        // The same draft again is refused: the name now exists.
        assert!(save_draft(&path, &draft).is_err());

        let again = r#"{"name":"careful"}"#;
        let change = validate_draft_for(again, &saved, Some("careful")).unwrap();
        let saved = save_draft(&path, &change).unwrap();
        // A change starts from what the pipeline is now: the step is still on
        // and the cap the draft did not mention is kept, not reset.
        assert_eq!(saved.pipeline("careful").unwrap().custom_steps(), [step]);
        assert_eq!(saved.pipeline("careful").unwrap().rounds, Some(2));
    }
}
