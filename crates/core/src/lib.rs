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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Understanding {
    pub summary: String,
    pub assumptions: Vec<String>,
    pub needs_design: bool,
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
    RoundStarted {
        round: u32,
    },
    CheckResult {
        check: String,
        passed: bool,
        note: String,
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
