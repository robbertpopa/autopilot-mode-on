//! The review rule packs and the code inventory that review units come from.
//!
//! - [`pack`]: reads the rule tables in `docs/rules/*.md` (id, rule, source,
//!   check, severity) into typed rules, so the tables stay the one source of
//!   truth and nothing is copied by hand.
//! - [`overrides`]: a project turns a rule off or changes its severity, and
//!   must give a reason for each; accepted risks carry an owner and an expiry.
//! - [`inventory`]: walks a repository without a model, skips generated and
//!   vendored folders, and cuts the files into units one agent can read whole.
//!
//! Nothing here runs a tool, opens a network connection or calls a model.

pub mod inventory;
pub mod overrides;
pub mod pack;

pub use inventory::{Inventory, Unit, inventory};
pub use overrides::{AcceptedRisk, Ledger, Overrides, Preset};
pub use pack::{CheckKind, Rule, Severity, load_dir, parse_pack};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{pack}: line {line}: {reason}")]
    Row {
        pack: String,
        line: usize,
        reason: String,
    },
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
