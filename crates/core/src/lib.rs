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

/// One static HTML mockup: a name and the page.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Mockup {
    /// A file name like `home.html`: lower-case letters, digits and `-`.
    pub name: String,
    pub html: String,
}

/// What the Design step produces: a few mockups and a note on the choices.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Design {
    #[schemars(length(min = 1, max = 6))]
    pub mockups: Vec<Mockup>,
    pub notes: String,
}

/// What a custom step reports: a short account and whether the agent found
/// nothing that should stop the ticket.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct CustomOutcome {
    /// What the step found or did, in plain words.
    pub summary: String,
    /// False when the step found something that should be looked at first.
    pub ok: bool,
}

/// The longest summary a custom step keeps.
pub const MAX_CUSTOM_SUMMARY: usize = 4000;

impl CustomOutcome {
    /// # Errors
    /// Returns a description when the summary is empty, too long or holds a
    /// control character, since it is shown and kept as plain text.
    pub fn validate(&self) -> Result<(), String> {
        if self.summary.trim().is_empty() {
            return Err("the summary is empty".into());
        }
        if self.summary.chars().count() > MAX_CUSTOM_SUMMARY {
            return Err(format!(
                "the summary is over {MAX_CUSTOM_SUMMARY} characters"
            ));
        }
        if self
            .summary
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err("the summary has a control character".into());
        }
        Ok(())
    }
}

/// The longest mockup page kept.
pub const MAX_MOCKUP_BYTES: usize = 200 * 1024;

impl Design {
    /// Reject a design the host must not save or open. A mockup is plain
    /// static HTML: no script, no frame or embed, no event handler attribute
    /// and no address that would load something from the network, so opening
    /// it in a browser can neither run code nor reach out.
    ///
    /// # Errors
    /// Returns a description of the first violation.
    pub fn validate(&self) -> Result<(), String> {
        if self.mockups.is_empty() || self.mockups.len() > 6 {
            return Err(format!("{} mockups, need 1 to 6", self.mockups.len()));
        }
        let mut seen = std::collections::BTreeSet::new();
        for mockup in &self.mockups {
            let stem = mockup.name.strip_suffix(".html").unwrap_or("");
            let plain = !stem.is_empty()
                && stem.len() <= 40
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !plain {
                return Err(format!("mockup name {:?} is not allowed", mockup.name));
            }
            if !seen.insert(mockup.name.as_str()) {
                return Err(format!("mockup {:?} appears twice", mockup.name));
            }
            if mockup.html.len() > MAX_MOCKUP_BYTES {
                return Err(format!("mockup {:?} is too large", mockup.name));
            }
            let lower = mockup.html.to_ascii_lowercase();
            for banned in [
                "<script",
                "<iframe",
                "<object",
                "<embed",
                "<link",
                "<base",
                "http-equiv",
                "@import",
                "javascript:",
                "http://",
                "https://",
                "//cdn",
                "url(//",
                "srcset",
                "data:text/html",
            ] {
                if lower.contains(banned) {
                    return Err(format!("mockup {:?} contains {banned}", mockup.name));
                }
            }
            // An event handler attribute such as onclick= or onload=.
            if has_handler(&lower) {
                return Err(format!("mockup {:?} has an event handler", mockup.name));
            }
        }
        Ok(())
    }
}

/// True when the page has an attribute that starts with `on` and is followed
/// by letters and `=`, such as `onclick=`.
fn has_handler(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        let boundary = index == 0
            || bytes[index - 1].is_ascii_whitespace()
            || bytes[index - 1] == b'"'
            || bytes[index - 1] == b'\'';
        if boundary && bytes[index] == b'o' && bytes[index + 1] == b'n' {
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_lowercase() {
                end += 1;
            }
            let mut after = end;
            while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                after += 1;
            }
            if end > index + 2 && after < bytes.len() && bytes[after] == b'=' {
                return true;
            }
        }
        index += 1;
    }
    false
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

/// One stage of a ticket. The six named steps are built in; `Custom` is a step
/// the owner defines in the profile (a name and a prompt), run between Build
/// and Check and the pull request. The slot number is 1 to
/// [`Step::CUSTOM_SLOTS`], so a step stays `Copy` and keeps a fixed name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    Understand,
    Clarify,
    Design,
    Plan,
    BuildCheck,
    Custom(u8),
    PullRequest,
}

impl Step {
    /// How many custom steps a profile may define.
    pub const CUSTOM_SLOTS: u8 = 8;

    /// The six built-in steps, in pipeline order. Custom steps are not here:
    /// code that must handle every step asks the profile for them.
    pub const ALL: [Self; 6] = [
        Self::Understand,
        Self::Clarify,
        Self::Design,
        Self::Plan,
        Self::BuildCheck,
        Self::PullRequest,
    ];

    /// Every step a ticket can hold, custom ones included, in pipeline order.
    pub const EVERY: [Self; 14] = [
        Self::Understand,
        Self::Clarify,
        Self::Design,
        Self::Plan,
        Self::BuildCheck,
        Self::Custom(1),
        Self::Custom(2),
        Self::Custom(3),
        Self::Custom(4),
        Self::Custom(5),
        Self::Custom(6),
        Self::Custom(7),
        Self::Custom(8),
        Self::PullRequest,
    ];

    /// A custom step by slot, or `None` outside 1 to [`Self::CUSTOM_SLOTS`].
    #[must_use]
    pub const fn custom(slot: u8) -> Option<Self> {
        if slot >= 1 && slot <= Self::CUSTOM_SLOTS {
            Some(Self::Custom(slot))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn is_custom(self) -> bool {
        matches!(self, Self::Custom(_))
    }

    /// The slot of a custom step.
    #[must_use]
    pub const fn slot(self) -> Option<u8> {
        match self {
            Self::Custom(slot) => Some(slot),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Understand => "understand",
            Self::Clarify => "clarify",
            Self::Design => "design",
            Self::BuildCheck => "build_check",
            Self::Plan => "plan",
            Self::PullRequest => "pull_request",
            Self::Custom(1) => "custom_1",
            Self::Custom(2) => "custom_2",
            Self::Custom(3) => "custom_3",
            Self::Custom(4) => "custom_4",
            Self::Custom(5) => "custom_5",
            Self::Custom(6) => "custom_6",
            Self::Custom(7) => "custom_7",
            Self::Custom(8) => "custom_8",
            Self::Custom(_) => "custom_0",
        }
    }

    /// Where the step sits in a pipeline. A custom step runs after Build and
    /// Check and before the pull request, in slot order.
    const fn rank(self) -> u16 {
        match self {
            Self::Understand => 0,
            Self::Clarify => 1,
            Self::Design => 2,
            Self::Plan => 3,
            Self::BuildCheck => 4,
            Self::Custom(slot) => 4 + slot as u16,
            Self::PullRequest => 100,
        }
    }
}

impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Pipeline order, so "every later step" is `>`.
impl Ord for Step {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl Serialize for Step {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize table keys as strings so profile diagnostics can name the step.
        let name = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .chain((1..=Self::CUSTOM_SLOTS).map(Self::Custom))
            .find(|step| step.as_str() == name)
            .ok_or_else(|| D::Error::custom(format!("unknown step {name:?}")))
    }
}

impl JsonSchema for Step {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Step".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let names: Vec<&str> = Step::ALL
            .into_iter()
            .chain((1..=Step::CUSTOM_SLOTS).map(Step::Custom))
            .map(Step::as_str)
            .collect();
        schemars::json_schema!({ "type": "string", "enum": names })
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

/// What a ticket does when it is unsure: stop and ask, or pick the
/// recommended option and say so. Mirrors the profile setting; the profile
/// crate sits above core, so the ticket-level choice is its own type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsureMode {
    Ask,
    DecideAndTell,
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
    /// The agent turn cap for this step. Absent in older histories.
    #[serde(default)]
    pub turns: Option<u32>,
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
        if newer.turns.is_some() {
            self.turns = newer.turns;
        }
    }
}

/// How hard the orchestrator judged a ticket to be.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Low,
    Medium,
    High,
}

/// The orchestrator's choice of model and effort for one later step.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StepAdvice {
    pub step: Step,
    pub model: String,
    pub effort: String,
    /// One sentence on why this much, and no more, for this step.
    pub why: String,
}

/// What the orchestrator decided after reading the ticket and its Understand
/// result: how complex the work is, and the cheapest settings it believes will
/// do each later step well.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Triage {
    pub complexity: Complexity,
    pub reasoning: String,
    pub steps: Vec<StepAdvice>,
}

/// One step's override, as recorded in the log and in the project file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StepChange {
    pub step: Step,
    #[serde(flatten)]
    pub change: StepOverride,
}

/// What a step consumed, as the agent reported it. Every figure is optional
/// where an adapter may report nothing: a missing figure is "not reported",
/// never zero and never a guess.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct UsageReport {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// The largest prompt any single turn sent, in tokens.
    #[serde(default)]
    pub peak_context_tokens: Option<u64>,
    /// The model's context window as the agent reported it.
    #[serde(default)]
    pub context_window_tokens: Option<u64>,
}

impl UsageReport {
    /// The peak as a share of the window in thousandths, when both are known.
    #[must_use]
    pub fn peak_context_permille(&self) -> Option<u32> {
        let peak = self.peak_context_tokens?;
        let window = self.context_window_tokens.filter(|window| *window > 0)?;
        u32::try_from(peak.saturating_mul(1000) / window).ok()
    }
}

/// The owner's correction of one assumption Understand made, given when the
/// owner approves. Later steps treat it as data to take into account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AssumptionCorrection {
    pub assumption: String,
    pub correction: String,
}

/// What an attachment is, which decides how the window shows it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    File,
    Image,
    Folder,
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
    /// The opaque local reference of the agent session a step is using. The
    /// provider's own identifier is never logged.
    SessionStarted {
        session: SessionId,
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
        /// The ticket's own `when_unsure`. Absent in older histories, where
        /// the profile setting applies.
        #[serde(default)]
        when_unsure: Option<UnsureMode>,
    },
    /// The words the owner wrote for the ticket, kept so a step can run again
    /// later without asking for them twice. Absent in older histories.
    Described {
        text: String,
    },
    /// A file, image or folder the owner attached. Its contents are data:
    /// nothing opens or runs them. A file was copied and hashed; a folder is
    /// only recorded by its path, item count and size.
    Attached {
        name: String,
        kind: AttachmentKind,
        bytes: u64,
        items: u64,
        sha256: String,
        stored: String,
    },
    /// The design the Design step produced, kept in the history so a review
    /// of the step shows exactly what was approved.
    DesignResult {
        design: Design,
    },
    /// What a custom step reported, kept so a review shows exactly what it said.
    CustomResult {
        outcome: CustomOutcome,
    },
    /// A short plain-words explanation of a step's result, asked of a small
    /// model once and kept so it is never asked again.
    Explained {
        text: String,
    },
    /// The project folder the ticket is about. The read-only steps look at its
    /// files; a later run of any step uses the same folder.
    Workspace {
        path: String,
    },
    /// The pipeline the ticket runs, resolved when it was chosen so the log
    /// stays readable if the profile changes. A ticket with none recorded ran
    /// the `full` pipeline.
    PipelineChosen {
        name: String,
        /// Steps the pipeline never runs.
        #[serde(default)]
        off: Vec<Step>,
        /// A lower cap on build rounds, if the pipeline sets one.
        #[serde(default)]
        rounds: Option<u32>,
        /// The custom steps the pipeline runs, so the pull request waits for
        /// them even if the profile changes later.
        #[serde(default)]
        custom: Vec<Step>,
    },
    /// The orchestrator's decision for the later steps. The settings it chose
    /// are recorded next to it as a ticket-scope `SettingsOverridden`.
    Triaged {
        triage: Triage,
    },
    /// The owner asked to run a step again. The named step goes back to idle,
    /// and with `and_after` so does every later step that already ran, because
    /// their results came from the old one. What was spent stays spent.
    StepReset {
        #[serde(default)]
        and_after: bool,
    },
    /// What the agent reported using for a step's run, kept next to its cost.
    UsageReported {
        usage: UsageReport,
    },
    /// The owner approved a step and corrected assumptions on the way. Kept
    /// with the plan so a review sees what was changed and why.
    AssumptionsCorrected {
        corrections: Vec<AssumptionCorrection>,
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

#[cfg(test)]
mod design_tests {
    use super::*;

    fn design(html: &str) -> Design {
        Design {
            mockups: vec![Mockup {
                name: "home.html".into(),
                html: html.into(),
            }],
            notes: "n".into(),
        }
    }

    // Invariant: a plain page, with the usual charset and viewport tags, is
    // allowed; anything that runs code, loads from elsewhere or redirects is
    // refused. This is what lets the saved mockups be opened without risk.
    #[test]
    fn a_mockup_may_not_run_code_or_reach_out() {
        let fine = "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><style>p{color:red}</style><p>Hi</p>";
        assert_eq!(design(fine).validate(), Ok(()));
        for bad in [
            "<script>1</script>",
            "<iframe src=a></iframe>",
            "<meta http-equiv=\"refresh\" content=\"0;url=x\">",
            "<link rel=stylesheet href=a.css>",
            "<img src=\"https://example.com/a.png\">",
            "<style>@import 'a.css';</style>",
            "<a href=\"javascript:alert(1)\">x</a>",
            "<p onclick=\"x()\">x</p>",
        ] {
            assert!(design(bad).validate().is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
mod custom_step_tests {
    use super::Step;

    // Reset "and every later step" and the pipeline view both rely on this
    // order: a custom step must sit after Build and Check and before the pull
    // request, whatever its slot.
    #[test]
    fn a_custom_step_runs_after_build_and_before_the_pull_request() {
        for slot in 1..=Step::CUSTOM_SLOTS {
            let step = Step::Custom(slot);
            assert!(Step::BuildCheck < step && step < Step::PullRequest);
        }
        assert!(Step::Custom(1) < Step::Custom(2));
    }

    // A custom step is stored as text in events and in profile.toml, so every
    // slot must read back as itself and a slot that does not exist must not.
    #[test]
    fn every_step_name_reads_back_and_unknown_slots_are_refused() {
        for step in Step::ALL
            .into_iter()
            .chain((1..=Step::CUSTOM_SLOTS).map(Step::Custom))
        {
            let text = serde_json::to_string(&step).unwrap();
            assert_eq!(serde_json::from_str::<Step>(&text).unwrap(), step);
        }
        assert!(serde_json::from_str::<Step>("\"custom_9\"").is_err());
        assert!(serde_json::from_str::<Step>("\"custom_0\"").is_err());
        assert_eq!(Step::custom(0), None);
        assert_eq!(Step::custom(9), None);
        assert_eq!(Step::custom(3), Some(Step::Custom(3)));
    }
}
