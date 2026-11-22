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
use autopilot_profile::{Pipeline, Profile, StepMode};
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
}

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

fn steps(value: Option<&Value>, key: &str) -> Result<Vec<Step>, String> {
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
            Step::ALL
                .into_iter()
                .find(|step| step.as_str() == name)
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

/// Check a reply against every rule and resolve it against the profile.
///
/// # Errors
/// Returns a sentence naming the rule the draft broke. Nothing is stored.
pub fn validate_draft(raw: &str, profile: &Profile) -> Result<PipelineDraft, String> {
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
    if profile.pipeline(name).is_some() {
        return Err(format!("a pipeline named {name:?} already exists"));
    }
    let based_on = object
        .get("based_on")
        .and_then(Value::as_str)
        .unwrap_or(autopilot_profile::DEFAULT_PIPELINE);
    let base = profile
        .pipeline(based_on)
        .ok_or_else(|| format!("based_on {based_on:?} is not a known pipeline"))?;
    let on = steps(object.get("on"), "on")?;
    let if_needed = steps(object.get("if_needed"), "if_needed")?;
    let off = steps(object.get("off"), "off")?;
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
    let pipeline = profile.build_pipeline(name, based_on, &on, &if_needed, &off, rounds)?;
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
    })
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
    let known: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
    let run = agent
        .start_draft(DraftRequest {
            description,
            known: &known,
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
    validate_draft(&raw, profile).map_err(Error::Draft)
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
    if current.pipeline(&draft.name).is_some() {
        return Err(Error::Draft(format!(
            "a pipeline named {:?} appeared since the draft",
            draft.name
        )));
    }
    let list = |wanted: StepMode| -> Vec<Step> {
        Step::ALL
            .into_iter()
            .filter(|step| draft.pipeline.mode(*step) == wanted)
            .collect()
    };
    crate::add_pipeline(
        profile_path,
        &draft.name,
        autopilot_profile::DEFAULT_PIPELINE,
        &list(StepMode::On),
        &list(StepMode::IfNeeded),
        &list(StepMode::Off),
        draft.pipeline.rounds,
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
