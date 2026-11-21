//! Provider-independent types shared by the pipeline and its event store.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::Error};

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.into())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

identifier!(TicketId);
identifier!(AttemptId);
identifier!(SessionId);
identifier!(RequestId);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Ticket {
    pub id: TicketId,
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Understanding {
    pub summary: String,
    pub ambiguity: Ambiguity,
    #[schemars(length(max = 5))]
    pub open_questions: Vec<OpenQuestion>,
    pub assumptions: Vec<String>,
    pub needs_design: bool,
    // Understand runs with repository tools off, so the model can only guess
    // these; they stay optional and absence is not an error.
    #[serde(default)]
    pub ui_files: Vec<String>,
    #[serde(default)]
    pub files_touched: Vec<String>,
}

/// How a criterion is shown to hold. `Unknown` can never pass a check.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verify {
    Test,
    Command,
    Unknown,
}

/// One acceptance criterion. Whether it holds comes from executed evidence, so
/// there is no `passes` field an agent could set to true.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Criterion {
    pub id: String,
    pub text: String,
    pub verify: Verify,
    /// Name of the test or command that verifies it. Empty when `verify` is unknown.
    pub verification_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Plan {
    #[schemars(length(min = 1, max = 30))]
    pub criteria: Vec<Criterion>,
    pub test_plan: Vec<String>,
    #[schemars(length(max = 50))]
    pub files: Vec<String>,
}

impl Plan {
    /// Reject a plan the schema cannot express or that cannot be checked later.
    ///
    /// # Errors
    /// Returns a description of the first violation.
    pub fn validate(&self) -> Result<(), String> {
        if self.criteria.is_empty() || self.criteria.len() > 30 {
            return Err(format!("{} criteria, need 1 to 30", self.criteria.len()));
        }
        if self.files.len() > 50 {
            return Err(format!("{} files, at most 50 allowed", self.files.len()));
        }
        let mut seen = std::collections::BTreeSet::new();
        for criterion in &self.criteria {
            if criterion.id.trim().is_empty() || criterion.text.trim().is_empty() {
                return Err("a criterion needs an id and text".into());
            }
            if !seen.insert(criterion.id.as_str()) {
                return Err(format!("criterion id {:?} appears twice", criterion.id));
            }
            let needs_id = criterion.verify != Verify::Unknown;
            if needs_id && criterion.verification_id.trim().is_empty() {
                return Err(format!(
                    "criterion {:?} names no verification",
                    criterion.id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Clarity {
    Clear,
    Partial,
    Missing,
}

/// How well the ticket pins down each area. A named field per category makes
/// the schema require all ten, so the model cannot skip one.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Ambiguity {
    pub functional_scope: Clarity,
    pub domain_and_data: Clarity,
    pub interaction_and_ux: Clarity,
    pub non_functional: Clarity,
    pub integrations: Clarity,
    pub edge_cases_and_failures: Clarity,
    pub constraints_and_trade_offs: Clarity,
    pub terminology: Clarity,
    pub completion_signals: Clarity,
    pub misc: Clarity,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct OpenQuestion {
    pub question: String,
    #[schemars(length(min = 2, max = 5))]
    pub options: Vec<String>,
    /// Index into `options` of the answer the model recommends.
    pub recommended: usize,
}

impl Ambiguity {
    #[must_use]
    pub fn all(clarity: Clarity) -> Self {
        Self {
            functional_scope: clarity,
            domain_and_data: clarity,
            interaction_and_ux: clarity,
            non_functional: clarity,
            integrations: clarity,
            edge_cases_and_failures: clarity,
            constraints_and_trade_offs: clarity,
            terminology: clarity,
            completion_signals: clarity,
            misc: clarity,
        }
    }
}

impl Understanding {
    /// Reject a result the schema cannot express: more than five questions,
    /// an option count outside 2 to 5, or a recommendation that points nowhere.
    ///
    /// # Errors
    /// Returns a description of the first violation.
    pub fn validate(&self) -> Result<(), String> {
        if self.open_questions.len() > 5 {
            return Err(format!(
                "{} open questions, at most 5 allowed",
                self.open_questions.len()
            ));
        }
        for (index, question) in self.open_questions.iter().enumerate() {
            let count = question.options.len();
            if !(2..=5).contains(&count) {
                return Err(format!("question {index} has {count} options, need 2 to 5"));
            }
            if question.recommended >= count {
                return Err(format!("question {index} recommends a missing option"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Understand,
    Clarify,
    Design,
    Plan,
    BuildCheck,
    PullRequest,
}

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize table keys as strings so profile diagnostics can name the step.
        let name = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|step| step.as_str() == name)
            .ok_or_else(|| D::Error::custom(format!("unknown step {name:?}")))
    }
}

impl Step {
    pub const ALL: [Self; 6] = [
        Self::Understand,
        Self::Clarify,
        Self::Design,
        Self::Plan,
        Self::BuildCheck,
        Self::PullRequest,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Understand => "understand",
            Self::Clarify => "clarify",
            Self::Design => "design",
            Self::BuildCheck => "build_check",
            Self::Plan => "plan",
            Self::PullRequest => "pull_request",
        }
    }
}

pub type StepId = Step;

/// An unsigned amount measured in millionths of a US dollar.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Money(u64);

impl Money {
    #[must_use]
    pub const fn from_microusd(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_microusd(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsScope {
    Ticket,
    Project,
}

/// A change to one step's settings. A missing field leaves that setting alone.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StepOverride {
    #[serde(default)]
    pub ask_after: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl StepOverride {
    /// Newer values win; anything the newer override leaves unset is kept.
    pub fn merge(&mut self, newer: &Self) {
        if newer.ask_after.is_some() {
            self.ask_after = newer.ask_after;
        }
        if newer.model.is_some() {
            self.model.clone_from(&newer.model);
        }
        if newer.effort.is_some() {
            self.effort.clone_from(&newer.effort);
        }
    }
}

/// One step's override, as recorded in the log and in the project file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StepChange {
    pub step: Step,
    #[serde(flatten)]
    pub change: StepOverride,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TicketCreated {
        title: String,
    },
    StepStarted {
        attempt: AttemptId,
        session: Option<SessionId>,
    },
    StepFinished,
    StepSkipped {
        reason: String,
    },
    AgentActivity {
        summary: String,
    },
    UnderstandResult {
        result: Understanding,
    },
    PlanResult {
        plan: Plan,
    },
    /// The owner approved this exact plan. The hash ties the approval to the
    /// copy stored outside the agent's reach.
    PlanApproved {
        sha256: String,
    },
    RoundStarted {
        round: u32,
    },
    CheckResult {
        check: String,
        passed: bool,
        note: String,
        /// Files a check produced, such as a log. Absent in older histories.
        #[serde(default)]
        artifacts: Vec<String>,
        /// What the check could not verify. Anything here means "not yet".
        #[serde(default)]
        unknowns: Vec<String>,
        /// True when a kept green result stood in for a run.
        #[serde(default)]
        reused: bool,
    },
    Asked {
        request: RequestId,
    },
    Answered {
        request: RequestId,
        answer: String,
    },
    CostTick {
        amount: Money,
        estimated: bool,
    },
    SettingsOverridden {
        scope: SettingsScope,
        #[serde(default)]
        changes: Vec<StepChange>,
    },
    Paused,
    Resumed,
    Interrupted {
        reason: String,
    },
    Failed {
        reason: String,
    },
    Done,
}

/// A timestamp comes from the caller's clock. Replaying never reads the clock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NewEvent {
    pub at_ms: u64,
    pub step: Option<Step>,
    pub kind: Event,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordedEvent {
    pub ticket: TicketId,
    pub seq: u64,
    pub event: NewEvent,
}

#[cfg(test)]
mod understanding_tests {
    use super::*;

    fn with_question(options: usize, recommended: usize) -> Understanding {
        Understanding {
            summary: "s".into(),
            ambiguity: Ambiguity::all(Clarity::Partial),
            open_questions: vec![OpenQuestion {
                question: "q".into(),
                options: (0..options).map(|i| i.to_string()).collect(),
                recommended,
            }],
            assumptions: vec![],
            needs_design: false,
            ui_files: vec![],
            files_touched: vec![],
        }
    }

    // Invariant: a model result that breaks the question bounds is rejected,
    // because Clarify would offer a recommendation that does not exist.
    #[test]
    fn question_bounds_are_enforced() {
        assert!(with_question(2, 1).validate().is_ok());
        assert!(with_question(1, 0).validate().is_err());
        assert!(with_question(6, 0).validate().is_err());
        assert!(with_question(3, 3).validate().is_err());
        let mut many = with_question(2, 0);
        many.open_questions = vec![many.open_questions[0].clone(); 6];
        assert!(many.validate().is_err());
    }

    #[test]
    fn missing_file_lists_default_to_empty() {
        let mut value = serde_json::to_value(with_question(2, 0)).unwrap();
        value.as_object_mut().unwrap().remove("ui_files");
        value.as_object_mut().unwrap().remove("files_touched");
        let parsed: Understanding = serde_json::from_value(value).unwrap();
        assert!(parsed.ui_files.is_empty() && parsed.files_touched.is_empty());
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    fn criterion(id: &str, verify: Verify, verification: &str) -> Criterion {
        Criterion {
            id: id.into(),
            text: "Something observable".into(),
            verify,
            verification_id: verification.into(),
        }
    }

    fn plan(criteria: Vec<Criterion>) -> Plan {
        Plan {
            criteria,
            test_plan: vec![],
            files: vec![],
        }
    }

    // Invariant: a plan the later checks cannot use is refused when it arrives,
    // not when Build is already running on it.
    #[test]
    fn a_plan_must_be_checkable() {
        assert!(
            plan(vec![criterion("a", Verify::Test, "t")])
                .validate()
                .is_ok()
        );
        assert!(plan(vec![]).validate().is_err());
        assert!(
            plan(vec![
                criterion("a", Verify::Test, "t"),
                criterion("a", Verify::Test, "u")
            ])
            .validate()
            .is_err()
        );
        assert!(
            plan(vec![criterion("a", Verify::Command, " ")])
                .validate()
                .is_err()
        );
        // Unknown is allowed to arrive: Analyze flags it, the owner decides.
        assert!(
            plan(vec![criterion("a", Verify::Unknown, "")])
                .validate()
                .is_ok()
        );
    }

    // Invariant: nothing an agent writes can mark a criterion as passing, so
    // a model reply carrying `passes: true` is not a way to skip a check.
    #[test]
    fn a_plan_has_no_passes_field_to_set() {
        let schema = serde_json::to_value(schemars::schema_for!(Plan)).unwrap();
        let properties = &schema["$defs"]["Criterion"]["properties"];
        assert!(properties.get("verification_id").is_some());
        assert!(properties.get("passes").is_none());
        let reply = serde_json::json!({
            "criteria": [{"id":"a","text":"t","verify":"test","verification_id":"v","passes":true}],
            "test_plan": [], "files": []
        });
        let parsed: Plan = serde_json::from_value(reply).unwrap();
        assert!(!serde_json::to_string(&parsed).unwrap().contains("passes"));
    }
}
