//! Which one thing to do next about an open pull request. Pure: facts in, action out.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeState {
    Clean,
    Conflicting,
    /// GitHub has not computed it yet; guessing either way would act on nothing.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    None,
    Approved,
    ChangesRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksState {
    None,
    Running,
    Passing,
    Failing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSummary {
    pub state: PrState,
    pub merge_state: MergeState,
    pub unresolved_threads: u32,
    pub review_decision: ReviewDecision,
    pub checks: ChecksState,
    pub local_dirty: bool,
    pub local_ahead: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrAction {
    ResolveConflicts,
    FixReviewFeedback,
    FixFailingChecks,
    /// The local branch holds work the pull request does not have yet.
    Unsynced,
}

/// Conflicts come first because the other fixes would be built on a branch that cannot merge;
/// feedback before CI because a review fix usually changes what CI sees.
#[must_use]
pub fn next_action(pr: &PrSummary) -> Option<PrAction> {
    if pr.state != PrState::Open {
        return None;
    }
    if pr.merge_state == MergeState::Conflicting {
        return Some(PrAction::ResolveConflicts);
    }
    if pr.unresolved_threads > 0 || pr.review_decision == ReviewDecision::ChangesRequested {
        return Some(PrAction::FixReviewFeedback);
    }
    // A running check may still turn red or green; acting now would race it.
    if pr.checks == ChecksState::Running {
        return None;
    }
    if pr.checks == ChecksState::Failing {
        return Some(PrAction::FixFailingChecks);
    }
    if pr.local_dirty || pr.local_ahead {
        return Some(PrAction::Unsynced);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> PrSummary {
        PrSummary {
            state: PrState::Open,
            merge_state: MergeState::Clean,
            unresolved_threads: 0,
            review_decision: ReviewDecision::None,
            checks: ChecksState::Passing,
            local_dirty: false,
            local_ahead: false,
        }
    }

    #[test]
    fn every_branch_names_one_action() {
        type Edit = fn(&mut PrSummary);
        let cases: [(&str, Edit, Option<PrAction>); 17] = [
            ("healthy", |_| {}, None),
            ("no checks at all", |p| p.checks = ChecksState::None, None),
            (
                "approved",
                |p| p.review_decision = ReviewDecision::Approved,
                None,
            ),
            ("merged", |p| p.state = PrState::Merged, None),
            ("closed", |p| p.state = PrState::Closed, None),
            (
                "merged with conflicts",
                |p| {
                    p.state = PrState::Merged;
                    p.merge_state = MergeState::Conflicting;
                },
                None,
            ),
            (
                "closed with feedback and failing ci",
                |p| {
                    p.state = PrState::Closed;
                    p.unresolved_threads = 3;
                    p.checks = ChecksState::Failing;
                },
                None,
            ),
            (
                "merge state unknown",
                |p| p.merge_state = MergeState::Unknown,
                None,
            ),
            (
                "conflict",
                |p| p.merge_state = MergeState::Conflicting,
                Some(PrAction::ResolveConflicts),
            ),
            (
                "unresolved thread",
                |p| p.unresolved_threads = 1,
                Some(PrAction::FixReviewFeedback),
            ),
            (
                "changes requested",
                |p| p.review_decision = ReviewDecision::ChangesRequested,
                Some(PrAction::FixReviewFeedback),
            ),
            (
                "failing ci",
                |p| p.checks = ChecksState::Failing,
                Some(PrAction::FixFailingChecks),
            ),
            ("running ci", |p| p.checks = ChecksState::Running, None),
            (
                "running ci does not hide feedback",
                |p| {
                    p.checks = ChecksState::Running;
                    p.unresolved_threads = 2;
                },
                Some(PrAction::FixReviewFeedback),
            ),
            (
                "dirty worktree",
                |p| p.local_dirty = true,
                Some(PrAction::Unsynced),
            ),
            (
                "local commits ahead",
                |p| p.local_ahead = true,
                Some(PrAction::Unsynced),
            ),
            (
                "running ci waits even when unsynced",
                |p| {
                    p.checks = ChecksState::Running;
                    p.local_ahead = true;
                },
                None,
            ),
        ];
        for (name, edit, want) in cases {
            let mut pr = ok();
            edit(&mut pr);
            assert_eq!(next_action(&pr), want, "{name}");
        }
    }

    #[test]
    fn priority_is_conflict_then_feedback_then_ci_then_unsynced() {
        let mut pr = ok();
        pr.merge_state = MergeState::Conflicting;
        pr.unresolved_threads = 1;
        pr.checks = ChecksState::Failing;
        pr.local_ahead = true;
        assert_eq!(next_action(&pr), Some(PrAction::ResolveConflicts));

        pr.merge_state = MergeState::Clean;
        assert_eq!(next_action(&pr), Some(PrAction::FixReviewFeedback));

        pr.unresolved_threads = 0;
        assert_eq!(next_action(&pr), Some(PrAction::FixFailingChecks));

        pr.checks = ChecksState::Passing;
        assert_eq!(next_action(&pr), Some(PrAction::Unsynced));

        pr.local_ahead = false;
        assert_eq!(next_action(&pr), None);
    }
}
