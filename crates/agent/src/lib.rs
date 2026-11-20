//! Provider-independent Understand action.

use autopilot_core::{Ticket, Understanding};

pub type AgentError = Box<dyn std::error::Error + Send + Sync>;

pub struct UnderstandRun {
    pub activity: Vec<String>,
    pub result: Understanding,
}

pub trait Agent {
    /// # Errors
    /// Returns an error if the agent fails to produce a structured result.
    fn understand(&self, ticket: &Ticket) -> Result<UnderstandRun, AgentError>;
}
