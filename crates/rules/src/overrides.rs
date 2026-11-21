//! What a project may change about the rule packs, and what it has accepted.
//!
//! Every change needs a reason, and a rule never turns off without one. An
//! accepted risk names an owner and an expiry, and stops hiding its finding
//! once the expiry has passed. Dates are plain `YYYY-MM-DD` text so no clock
//! is read here: the caller says what today is.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    Error,
    pack::{Rule, Severity},
};

/// How much a review reports, as the lowest severity it shows.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Everything, nits and information included.
    Strict,
    /// Medium and above.
    Standard,
    /// High and critical only.
    Relaxed,
}

impl Preset {
    #[must_use]
    pub const fn floor(self) -> Severity {
        match self {
            Self::Strict => Severity::Info,
            Self::Standard => Severity::Medium,
            Self::Relaxed => Severity::High,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Change {
    /// `None` leaves the rule on.
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub severity: Option<Severity>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Overrides {
    pub preset: Preset,
    #[serde(default)]
    pub rules: BTreeMap<String, Change>,
}

impl Overrides {
    /// A project that changes nothing.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            preset: Preset::Standard,
            rules: BTreeMap::new(),
        }
    }

    /// Read the project's overrides from JSON.
    ///
    /// # Errors
    /// Returns an error if the text is not valid, a change has no reason, or a
    /// change names a rule that does not exist in `known`.
    pub fn parse(text: &str, known: &[Rule]) -> Result<Self, Error> {
        let parsed: Self = serde_json::from_str(text)?;
        for (id, change) in &parsed.rules {
            if change.reason.trim().is_empty() {
                return Err(Error::Invalid(format!("the change to {id} has no reason")));
            }
            if !known.iter().any(|rule| &rule.id == id) {
                return Err(Error::Invalid(format!("{id} is not a rule in any pack")));
            }
        }
        Ok(parsed)
    }

    /// The rules a review applies, with the project's severities, most serious
    /// first. A rule the project turned off, or whose severity falls under the
    /// preset's floor, is left out.
    #[must_use]
    pub fn apply(&self, rules: &[Rule]) -> Vec<Rule> {
        let mut kept: Vec<Rule> = rules
            .iter()
            .filter_map(|rule| {
                let mut rule = rule.clone();
                if let Some(change) = self.rules.get(&rule.id) {
                    if change.enabled == Some(false) {
                        return None;
                    }
                    if let Some(severity) = change.severity {
                        rule.severity = severity;
                    }
                }
                (rule.severity <= self.preset.floor()).then_some(rule)
            })
            .collect();
        kept.sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| a.id.cmp(&b.id)));
        kept
    }
}

/// A finding the owner chose to live with.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AcceptedRisk {
    /// The rule id and where it was found, as the finding names it.
    pub finding: String,
    pub reason: String,
    pub owner: String,
    /// `YYYY-MM-DD`, the last day the risk is accepted.
    pub expires: String,
}

fn date_ok(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let number = |s: &str, len: usize| {
        (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    matches!(
        (number(year, 4), number(month, 2), number(day, 2)),
        (Some(_), Some(1..=12), Some(1..=31))
    )
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Ledger {
    #[serde(default)]
    pub accepted: Vec<AcceptedRisk>,
}

impl Ledger {
    /// Read the ledger from JSON.
    ///
    /// # Errors
    /// Returns an error if an entry has no reason or owner, or its expiry is
    /// not a date.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let ledger: Self = serde_json::from_str(text)?;
        for risk in &ledger.accepted {
            if risk.reason.trim().is_empty() || risk.owner.trim().is_empty() {
                return Err(Error::Invalid(format!(
                    "the accepted risk {:?} needs a reason and an owner",
                    risk.finding
                )));
            }
            if !date_ok(&risk.expires) {
                return Err(Error::Invalid(format!(
                    "the accepted risk {:?} has the expiry {:?}, which is not YYYY-MM-DD",
                    risk.finding, risk.expires
                )));
            }
        }
        Ok(ledger)
    }

    /// The entry that still hides this finding on `today`, if there is one.
    #[must_use]
    pub fn hides(&self, finding: &str, today: &str) -> Option<&AcceptedRisk> {
        self.accepted
            .iter()
            .find(|risk| risk.finding == finding && risk.expires.as_str() >= today)
    }

    /// The entries whose expiry has passed, which a review must raise again.
    #[must_use]
    pub fn expired(&self, today: &str) -> Vec<&AcceptedRisk> {
        self.accepted
            .iter()
            .filter(|risk| risk.expires.as_str() < today)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::parse_pack;

    fn rules() -> Vec<Rule> {
        parse_pack(
            "p",
            "| RC-01 | a | s | lint | high |\n| RC-02 | b | s | lint | nit |\n| RC-03 | c | s | script | medium |\n",
        )
        .unwrap()
    }

    // Invariant: a rule never changes without a reason and never for a rule
    // that does not exist, so a typo cannot silently switch nothing off.
    #[test]
    fn a_change_needs_a_reason_and_a_real_rule() {
        let known = rules();
        let blank = r#"{"preset":"strict","rules":{"RC-01":{"enabled":false,"reason":" "}}}"#;
        assert!(Overrides::parse(blank, &known).is_err());
        let typo = r#"{"preset":"strict","rules":{"RC-99":{"enabled":false,"reason":"x"}}}"#;
        assert!(Overrides::parse(typo, &known).is_err());
    }

    // Invariant: the preset's floor, a disabled rule and a raised severity all
    // act together, and the result is ordered most serious first.
    #[test]
    fn the_preset_and_the_changes_decide_which_rules_apply() {
        let known = rules();
        let text = r#"{"preset":"standard","rules":{
            "RC-02":{"severity":"high","reason":"we care"},
            "RC-03":{"enabled":false,"reason":"generated code"}}}"#;
        let applied = Overrides::parse(text, &known).unwrap().apply(&known);
        let ids: Vec<&str> = applied.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["RC-01", "RC-02"]);
        assert_eq!(Overrides::standard().apply(&known).len(), 2);
    }

    // Invariant: an accepted risk hides its finding through its expiry day and
    // not a day longer, and an entry without an owner or a real date is refused.
    #[test]
    fn an_accepted_risk_expires() {
        let text = r#"{"accepted":[{"finding":"SE-02 a.rs","reason":"r","owner":"o","expires":"2026-12-01"}]}"#;
        let ledger = Ledger::parse(text).unwrap();
        assert!(ledger.hides("SE-02 a.rs", "2026-12-01").is_some());
        assert!(ledger.hides("SE-02 a.rs", "2026-12-02").is_none());
        assert_eq!(ledger.expired("2026-12-02").len(), 1);
        let no_owner = text.replace("\"o\"", "\"\"");
        assert!(Ledger::parse(&no_owner).is_err());
        let bad_date = text.replace("2026-12-01", "tomorrow");
        assert!(Ledger::parse(&bad_date).is_err());
    }
}
