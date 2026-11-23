//! "Ask for a change": plain words turned into a typed pipeline patch.
//!
//! The reading is a small fixed grammar, done by code, so it costs nothing and
//! can be tested. A clause it does not understand is reported back as a note
//! and never guessed at. Nothing here writes anything: the caller shows the
//! patch as a diff and applies it only when the owner says so.

use std::path::Path;

use autopilot_core::{SettingsScope, Step, StepChange, StepOverride, TicketId};
use autopilot_profile::Profile;

use crate::{Error, ProfileEditError, StepField, edit_step, override_settings, step_name};

/// One setting that would change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub step: Step,
    pub field: StepField,
    pub before: String,
    pub after: String,
}

impl Proposal {
    /// `[steps.plan] model = "default" -> "opus-5-5"`, as one line.
    #[must_use]
    pub fn describe(&self) -> String {
        let name = match &self.field {
            StepField::Model(_) => "model",
            StepField::Effort(_) => "effort",
            StepField::Turns(_) => "turns",
            StepField::Rounds(_) => "refine rounds",
            StepField::AskAfter(_) => "ask after",
            StepField::Auth(_) => "auth",
            StepField::BudgetMicrousd(_) => "budget",
        };
        format!(
            "{}: {name} {} -> {}",
            step_name(self.step),
            self.before,
            self.after
        )
    }

    /// Whether this setting can be changed for one ticket. Turns and rounds
    /// are project settings only.
    #[must_use]
    pub const fn fits_one_ticket(&self) -> bool {
        matches!(
            self.field,
            StepField::Model(_) | StepField::Effort(_) | StepField::AskAfter(_)
        )
    }
}

/// What the words were understood as, and what was not.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Reading {
    pub proposals: Vec<Proposal>,
    /// Clauses that were refused or not understood, each with the reason.
    pub notes: Vec<String>,
}

const STEPS: [(&str, Step); 7] = [
    ("understand", Step::Understand),
    ("clarify", Step::Clarify),
    ("design", Step::Design),
    ("plan", Step::Plan),
    ("build", Step::BuildCheck),
    ("check", Step::BuildCheck),
    ("pull request", Step::PullRequest),
];

const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const MODELS: [&str; 3] = ["haiku", "sonnet", "opus"];

fn step_in(clause: &str) -> Option<Step> {
    STEPS
        .iter()
        .find(|(word, _)| clause.contains(word))
        .map(|(_, step)| *step)
}

fn number_before(clause: &str, word: &str) -> Option<u32> {
    let words: Vec<&str> = clause.split_whitespace().collect();
    let at = words.iter().position(|w| w.starts_with(word))?;
    words[..at].iter().rev().find_map(|w| w.parse().ok())
}

fn current(profile: &Profile, step: Step) -> Option<&autopilot_profile::StepSettings> {
    profile.step(step)
}

fn clause(text: &str, profile: &Profile) -> Result<Proposal, String> {
    if ["budget", "spend", "cost", "money", "dollar"]
        .iter()
        .any(|word| text.contains(word))
    {
        return Err("a spending limit cannot be changed from here".into());
    }
    if text.contains("review") {
        return Err(
            "the review runs inside Build and Check and is not a separate step with its own settings yet"
                .into(),
        );
    }
    let find = |step: Step| {
        current(profile, step).ok_or_else(|| format!("{} has no settings", step_name(step)))
    };
    if let Some(model) = MODELS.iter().find(|word| text.contains(*word)) {
        let step = step_in(text).ok_or("say which step it is for")?;
        let after = format!("{model}-5-5");
        let before = find(step)?
            .model
            .clone()
            .unwrap_or_else(|| "default".into());
        return Ok(Proposal {
            step,
            field: StepField::Model(after.clone()),
            before,
            after,
        });
    }
    if text.contains("effort")
        && let Some(level) = EFFORTS
            .iter()
            .find(|word| text.split_whitespace().any(|w| w == **word))
    {
        let step = step_in(text).ok_or("say which step it is for")?;
        let before = find(step)?
            .effort
            .clone()
            .unwrap_or_else(|| "default".into());
        return Ok(Proposal {
            step,
            field: StepField::Effort((*level).to_owned()),
            before,
            after: (*level).to_owned(),
        });
    }
    if let Some(rounds) = number_before(text, "round") {
        if rounds == 0 {
            return Err("at least one round is needed".into());
        }
        let before = find(Step::BuildCheck)?.limits.rounds.to_string();
        return Ok(Proposal {
            step: Step::BuildCheck,
            field: StepField::Rounds(rounds),
            before,
            after: rounds.to_string(),
        });
    }
    if let Some(turns) = number_before(text, "turn") {
        if turns == 0 {
            return Err("at least one turn is needed".into());
        }
        let step = step_in(text).unwrap_or(Step::BuildCheck);
        let before = find(step)?.limits.turns.to_string();
        return Ok(Proposal {
            step,
            field: StepField::Turns(turns),
            before,
            after: turns.to_string(),
        });
    }
    if text.contains("ask") {
        let step = step_in(text).ok_or("say which step it is for")?;
        let off = ["don't", "do not", "stop", "no longer", "never", "without"]
            .iter()
            .any(|word| text.contains(word));
        let before = find(step)?.ask_after;
        return Ok(Proposal {
            step,
            field: StepField::AskAfter(!off),
            before: before.to_string(),
            after: (!off).to_string(),
        });
    }
    Err("not understood".into())
}

/// Read a request. Settings that already have the asked value are left out.
///
/// # Errors
/// Returns a plain sentence when nothing in the text could be understood.
pub fn propose(text: &str, profile: &Profile) -> Result<Reading, String> {
    let lower = text.to_lowercase();
    let mut reading = Reading::default();
    for part in lower
        .split([',', ';', '.'])
        .flat_map(|piece| piece.split(" and "))
        .flat_map(|piece| piece.split(" then "))
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
    {
        match clause(part, profile) {
            Ok(proposal) if proposal.before != proposal.after => reading.proposals.push(proposal),
            Ok(_) => reading
                .notes
                .push(format!("\"{part}\": already set that way")),
            Err(why) => reading.notes.push(format!("\"{part}\": {why}")),
        }
    }
    if reading.proposals.is_empty() && reading.notes.is_empty() {
        return Err("write what should change, for example \"use opus for the plan\"".into());
    }
    if reading.proposals.is_empty() {
        return Err(reading.notes.join("; "));
    }
    Ok(reading)
}

/// Apply to every future ticket: written to `profile.toml`.
///
/// # Errors
/// Returns the profile error, or the diagnostic if the result is invalid.
pub fn apply_to_project(
    profile_path: &Path,
    proposals: &[Proposal],
) -> Result<Profile, ProfileEditError> {
    let mut latest = crate::load_profile(profile_path)?;
    for proposal in proposals {
        latest = edit_step(profile_path, proposal.step, &proposal.field)?;
    }
    Ok(latest)
}

/// Apply to one ticket: recorded in its history, used when a step next starts.
///
/// # Errors
/// Returns an error for a setting that is not per-ticket, or if recording fails.
pub fn apply_to_ticket(
    directory: &Path,
    ticket: &TicketId,
    proposals: &[Proposal],
) -> Result<(), Error> {
    let mut changes = Vec::new();
    for proposal in proposals {
        let mut change = StepOverride::default();
        match &proposal.field {
            StepField::Model(model) => change.model = Some(model.clone()),
            StepField::Effort(effort) => change.effort = Some(effort.clone()),
            StepField::AskAfter(ask) => change.ask_after = Some(*ask),
            StepField::Turns(turns) => change.turns = Some(*turns),
            _ => {
                return Err(Error::NotReady(format!(
                    "{} can only change for all future tickets",
                    proposal.describe()
                )));
            }
        }
        changes.push(StepChange {
            step: proposal.step,
            change,
        });
    }
    override_settings(directory, ticket, SettingsScope::Ticket, &changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::parse("p.toml", "").unwrap()
    }

    fn one(text: &str) -> Proposal {
        propose(text, &profile()).unwrap().proposals.remove(0)
    }

    // Invariant: the common requests become exactly the typed change they
    // say, with the old value shown.
    #[test]
    fn plain_requests_become_typed_changes() {
        let p = one("use opus for the plan");
        assert_eq!(
            (p.step, p.field),
            (Step::Plan, StepField::Model("opus-5-5".into()))
        );
        assert_eq!(p.before, "opus");
        let p = one("allow only 2 refine rounds");
        assert_eq!(
            (p.step, p.field, p.before.as_str()),
            (Step::BuildCheck, StepField::Rounds(2), "3")
        );
        let p = one("set high effort for design");
        assert_eq!(
            (p.step, p.field),
            (Step::Design, StepField::Effort("high".into()))
        );
        let p = one("30 turns for the build");
        assert_eq!((p.step, p.field), (Step::BuildCheck, StepField::Turns(30)));
    }

    // Invariant: asking after a step can be switched on and off in words.
    #[test]
    fn ask_after_can_be_turned_on_and_off() {
        assert_eq!(
            one("ask me after understand").field,
            StepField::AskAfter(true)
        );
        assert_eq!(
            one("don't ask me after the plan").field,
            StepField::AskAfter(false)
        );
    }

    // Invariant: a spending limit is never changed by words, and a clause
    // that is not understood is reported, not guessed.
    #[test]
    fn limits_and_unknown_clauses_are_refused_with_a_reason() {
        let reading = propose(
            "use sonnet for the plan and raise the budget to $5",
            &profile(),
        )
        .unwrap();
        assert_eq!(reading.proposals.len(), 1);
        assert!(reading.notes[0].contains("spending limit cannot be changed"));
        let error = propose("make it faster", &profile()).unwrap_err();
        assert!(error.contains("not understood"));
        assert!(propose("   ", &profile()).is_err());
    }

    // Invariant: the design's own example is read as far as it can be, and the
    // part that has no setting yet says so.
    #[test]
    fn the_review_clause_is_reported_not_guessed() {
        let reading = propose(
            "Use sonnet for the review and allow only 2 refine rounds",
            &profile(),
        )
        .unwrap();
        assert_eq!(reading.proposals.len(), 1);
        assert_eq!(reading.proposals[0].field, StepField::Rounds(2));
        assert!(reading.notes[0].contains("not a separate step"));
    }

    // Invariant: a project change lands in profile.toml, and a ticket change
    // lands in the ticket's history. Turns and rounds are project-only.
    #[test]
    fn changes_apply_to_the_project_or_to_one_ticket() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.toml");
        let reading = propose("use opus for the plan and 2 rounds", &profile()).unwrap();
        let applied = apply_to_project(&path, &reading.proposals).unwrap();
        assert_eq!(
            applied.step(Step::Plan).unwrap().model.as_deref(),
            Some("opus-5-5")
        );
        assert_eq!(applied.step(Step::BuildCheck).unwrap().limits.rounds, 2);

        let rounds = one("2 rounds");
        assert!(!rounds.fits_one_ticket());
        let ticket = TicketId::from("t");
        let error = apply_to_ticket(directory.path(), &ticket, &[rounds]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only change for all future tickets")
        );
    }
}
