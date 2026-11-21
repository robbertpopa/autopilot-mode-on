use std::sync::Arc;

use autopilot_core::Money;
use autopilot_policy::{DiffSignals, Facts, Policy, Reason, Risk};

use crate::{Change, Check, Context, Error, Report};

/// Protected paths and diff size. Touching a protected path fails the check:
/// the agent was told to leave it alone, and a note names the paths so the
/// next turn can put them back. Lockfiles, scripts and workflows are allowed
/// here and only force a question at approval time.
pub struct DiffCheck {
    policy: Arc<Policy>,
}

impl DiffCheck {
    #[must_use]
    pub fn new(policy: Arc<Policy>) -> Self {
        Self { policy }
    }
}

/// `(files, added, removed)` over the changes. Binary files add no lines.
fn size(changes: &[Change]) -> (usize, u64, u64) {
    let mut added = 0_u64;
    let mut removed = 0_u64;
    for (a, r) in changes.iter().filter_map(|change| change.lines) {
        added = added.saturating_add(a);
        removed = removed.saturating_add(r);
    }
    (changes.len(), added, removed)
}

impl Check for DiffCheck {
    fn id(&self) -> &'static str {
        "diff"
    }

    fn applies(&self, _: &Context<'_>) -> bool {
        true
    }

    fn can_refine(&self) -> bool {
        true
    }

    fn configuration(&self) -> String {
        self.policy.protected_patterns().join("\n")
    }

    fn run(&self, context: &Context<'_>) -> Result<Report, Error> {
        let touched: Vec<String> = context
            .changes
            .iter()
            .map(|change| change.path.clone())
            .collect();
        let decision = self.policy.evaluate(
            false,
            &Facts {
                touched: &touched,
                risk: Risk::Standard,
                labels: &[],
                ticket_spent: Money::default(),
                day_spent: Money::default(),
                signals: DiffSignals::default(),
            },
        );
        let blocked: Vec<String> = decision
            .forced
            .iter()
            .filter(|reason| {
                matches!(
                    reason,
                    Reason::ProtectedPath(_) | Reason::OutsideRepository(_)
                )
            })
            .map(Reason::describe)
            .collect();
        let (files, added, removed) = size(context.changes);
        let summary = format!("{files} files changed, +{added} -{removed}");
        if blocked.is_empty() {
            Ok(Report::pass(summary))
        } else {
            Ok(Report::fail(format!(
                "{summary}. Revert changes to: {}",
                blocked.join(", ")
            )))
        }
    }
}
