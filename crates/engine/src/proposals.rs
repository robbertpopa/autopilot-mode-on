//! Tier 2 of the pipeline review: a change the owner must approve. Nothing
//! here writes `profile.toml` or changes a run; it turns an open finding into a
//! `Proposal` the window shows, and records a dismissal so the same proposal
//! does not come back. Approving applies it through `apply_to_project`, the
//! same path as a plain-words change, so the profile is re-parsed before it is
//! written.
//!
//! Only one kind exists so far: lower a step's effort one notch for every
//! pipeline. It can only reduce token use, so it cannot loosen a safety rule.
//! Anything that adds rounds, raises a model, turns a step on or off or touches
//! `ask_after`, Design, the pull request, budgets or `protected_paths` is not
//! proposed, so a finding about those has no button.

use std::{fs, io, io::Write as _, path::Path};

use autopilot_core::Step;
use autopilot_profile::{Profile, ReviewMode};
use serde::{Deserialize, Serialize};

use crate::{ActiveTuning, Error, Finding, Proposal, Rule, StepField};

const FILE: &str = "review-dismissed.jsonl";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Dismissed {
    pipeline: String,
    step: Step,
    rule: Rule,
}

/// A finding and the change that would address it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Suggestion {
    pub finding: Finding,
    pub proposal: Proposal,
}

fn read(directory: &Path) -> Result<Vec<Dismissed>, Error> {
    let text = match fs::read_to_string(directory.join(FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    // A damaged line only brings one proposal back, which is harmless.
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// Remember that the owner does not want this proposal.
///
/// # Errors
/// Returns an error if the record cannot be written.
pub fn dismiss_proposal(directory: &Path, finding: &Finding) -> Result<(), Error> {
    fs::create_dir_all(directory)?;
    let line = serde_json::to_string(&Dismissed {
        pipeline: finding.pipeline.clone(),
        step: finding.step,
        rule: finding.rule,
    })
    .map_err(|error| Error::History(error.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(FILE))?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// The proposals the open findings support. Pure apart from reading the
/// dismissals. With the review off there are none.
///
/// # Errors
/// Returns an error if the dismissal record exists but cannot be read.
pub fn proposals_for(
    directory: &Path,
    findings: &[Finding],
    profile: &Profile,
    tuned: &[ActiveTuning],
) -> Result<Vec<Suggestion>, Error> {
    if profile.review.mode == ReviewMode::Off {
        return Ok(vec![]);
    }
    let dismissed = read(directory)?;
    let mut found: Vec<Suggestion> = Vec::new();
    for finding in findings {
        let is_dismissed = dismissed.iter().any(|each| {
            each.pipeline == finding.pipeline
                && each.step == finding.step
                && each.rule == finding.rule
        });
        // A step already tuned waits for its guard; two changes to one step
        // would hide which one helped.
        let is_tuned = tuned.iter().any(|each| {
            each.tuning.pipeline == finding.pipeline && each.tuning.step == finding.step
        });
        // The same change from two findings (peak and cost) is one proposal.
        let repeated = found.iter().any(|each| each.proposal.step == finding.step);
        if is_dismissed || is_tuned || repeated {
            continue;
        }
        let current = profile
            .step(finding.step)
            .and_then(|settings| settings.effort.clone());
        // An unset effort is left alone: the review cannot see the agent's own
        // default, so it cannot say what "one notch lower" means.
        let Some(before) = current else { continue };
        let Some(after) = crate::tuning::lower(Some(&before)) else {
            continue;
        };
        found.push(Suggestion {
            finding: finding.clone(),
            proposal: Proposal {
                step: finding.step,
                field: StepField::Effort(after.to_owned()),
                before,
                after: after.to_owned(),
            },
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(step: Step, rule: Rule) -> Finding {
        Finding {
            pipeline: "full".into(),
            step,
            rule,
            evidence: "e".into(),
            estimated: false,
        }
    }

    // Invariant: a finding gives a proposal only when the profile sets an
    // effort that can go down, two findings on one step give one proposal, a
    // dismissed proposal stays away, and the review being off gives none. A
    // proposal for an unset effort would be a guess, and one that came back
    // after Dismiss would nag; no other test covers either.
    #[test]
    fn a_proposal_needs_a_lowerable_effort_and_a_dismissal_sticks() {
        let dir = tempfile::tempdir().unwrap();
        let on = "[review]\nmode = \"notify_only\"\n\n[steps.plan]\neffort = \"high\"\n\n[steps.understand]\neffort = \"low\"\n";
        let profile = Profile::parse("profile.toml", on).unwrap();
        let findings = [
            finding(Step::Plan, Rule::PeakContext),
            finding(Step::Plan, Rule::StepCost),
            finding(Step::Understand, Rule::PeakContext),
            finding(Step::Clarify, Rule::PeakContext),
        ];
        let got = proposals_for(dir.path(), &findings, &profile, &[]).unwrap();
        assert_eq!(got.len(), 1, "low and unset have nothing to lower: {got:?}");
        assert_eq!(got[0].proposal.after, "medium");

        dismiss_proposal(dir.path(), &got[0].finding).unwrap();
        // The cost finding on the same step is a different rule, so it may
        // still offer the change; the dismissed one must not.
        let again = proposals_for(dir.path(), &findings, &profile, &[]).unwrap();
        assert!(
            again
                .iter()
                .all(|each| each.finding.rule != Rule::PeakContext)
        );

        let off = Profile::parse("profile.toml", "[steps.plan]\neffort = \"high\"\n").unwrap();
        assert_eq!(
            proposals_for(dir.path(), &findings, &off, &[])
                .unwrap()
                .len(),
            0
        );
    }
}
