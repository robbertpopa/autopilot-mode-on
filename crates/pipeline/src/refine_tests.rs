use autopilot_checks::{CheckRun, Report};
use autopilot_core::{Event, Money};

use crate::refine::{BuildRequest, BuildTurn, Limits, Outcome, Round, Session, Stop, refine};

/// A scripted agent and checks. `fixes_on` is the round whose build makes the
/// checks pass; `changes_tree` says whether a turn edits anything at all.
struct Script {
    fixes_on: Option<u32>,
    changes_tree: bool,
    turn: BuildTurn,
    requests: Vec<BuildRequest>,
    tree: u32,
    unrefinable: bool,
    fail_build: bool,
    /// What each round's checks cost, as a review would.
    check_cost: Money,
}

impl Script {
    fn never_fixes() -> Self {
        Self {
            fixes_on: None,
            changes_tree: true,
            turn: BuildTurn {
                cost: Money::from_microusd(10),
                turns: 1,
            },
            requests: vec![],
            tree: 0,
            unrefinable: false,
            fail_build: false,
            check_cost: Money::default(),
        }
    }
}

impl Round for Script {
    type Error = String;

    fn tree(&mut self) -> Result<String, String> {
        Ok(self.tree.to_string())
    }

    fn build(&mut self, request: &BuildRequest) -> Result<BuildTurn, String> {
        if self.fail_build {
            return Err("agent did not start".into());
        }
        self.requests.push(request.clone());
        if self.changes_tree {
            self.tree += 1;
        }
        Ok(self.turn)
    }

    fn check_cost(&mut self) -> Money {
        self.check_cost
    }

    fn check(&mut self) -> Result<Vec<CheckRun>, String> {
        let round = u32::try_from(self.requests.len()).unwrap();
        let build = CheckRun {
            id: "build".into(),
            report: Report::pass("build passed"),
            reused: false,
            can_refine: true,
        };
        if self.fixes_on.is_some_and(|fix| round >= fix) {
            return Ok(vec![build]);
        }
        let mut report = Report::fail(format!("1 test failed in round {round}"));
        if self.unrefinable {
            report = report.with_unknowns(vec!["c1 cannot be verified".into()]);
        }
        Ok(vec![
            build,
            CheckRun {
                id: "test".into(),
                report,
                reused: false,
                can_refine: true,
            },
        ])
    }
}

fn limits(rounds: u32, turns: u32, budget: Option<u64>) -> Limits {
    Limits {
        rounds,
        turns,
        budget: budget.map(Money::from_microusd),
    }
}

fn go(limits: Limits, script: &mut Script) -> (Outcome, Vec<Event>) {
    let mut events = Vec::new();
    let outcome = refine(limits, script, &mut |event| events.push(event)).unwrap();
    (outcome, events)
}

fn needs_you(outcome: &Outcome) -> (&Stop, &str) {
    match outcome {
        Outcome::NeedsYou { stop, note } => (stop, note),
        Outcome::Ready => panic!("expected the loop to stop"),
    }
}

// Invariant: the failed note goes back to the session that wrote the code, and
// the loop ends as soon as the checks pass.
#[test]
fn a_failure_is_sent_back_and_a_fix_ends_the_loop() {
    let mut script = Script::never_fixes();
    script.fixes_on = Some(2);
    let (outcome, events) = go(limits(3, 20, None), &mut script);
    assert_eq!(outcome, Outcome::Ready);
    assert_eq!(script.requests.len(), 2);
    assert_eq!(script.requests[0].failed, None);
    assert_eq!(
        script.requests[1].failed.as_deref(),
        Some("test: 1 test failed in round 1")
    );
    assert_eq!(script.requests[1].session, Session::Continue);
    let rounds: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::RoundStarted { round } => Some(*round),
            _ => None,
        })
        .collect();
    assert_eq!(rounds, [1, 2]);
}

// Invariant: rounds are capped. The agent changes something every turn here,
// so only the round cap can stop it.
#[test]
fn the_round_cap_stops_the_loop_with_the_last_note() {
    let mut script = Script::never_fixes();
    let (outcome, _) = go(limits(3, 100, None), &mut script);
    assert_eq!(script.requests.len(), 3);
    let (stop, note) = needs_you(&outcome);
    assert_eq!(stop, &Stop::RoundCap { rounds: 3 });
    assert_eq!(note, "test: 1 test failed in round 3");
}

#[test]
fn the_turn_cap_stops_before_another_build_turn() {
    let mut script = Script::never_fixes();
    script.turn.turns = 5;
    let (outcome, _) = go(limits(10, 7, None), &mut script);
    assert_eq!(script.requests.len(), 2, "5 turns is under 7, 10 is not");
    let left: Vec<u32> = script.requests.iter().map(|r| r.turns_left).collect();
    assert_eq!(left, [7, 2], "each call is launched with what remains");
    assert_eq!(needs_you(&outcome).0, &Stop::TurnCap { turns: 10 });
}

// Invariant: what the checks spend counts against the step's dollar cap. A
// review that calls a model each round would otherwise spend without limit:
// the loop saw only the build turns. A change that breaks it: reading only
// `turn.cost`.
#[test]
fn what_the_checks_spend_counts_against_the_budget() {
    let mut script = Script::never_fixes();
    script.turn.cost = Money::from_microusd(10);
    script.check_cost = Money::from_microusd(600);
    let (outcome, _) = go(limits(10, 100, Some(1000)), &mut script);
    assert_eq!(
        script.requests.len(),
        2,
        "610 after round one, 1220 after two"
    );
    let left: Vec<Option<u64>> = script
        .requests
        .iter()
        .map(|r| r.budget_left.map(Money::as_microusd))
        .collect();
    assert_eq!(
        left,
        [Some(1000), Some(390)],
        "round two sees the review's spend"
    );
    assert!(matches!(
        needs_you(&outcome).0,
        Stop::BudgetCap { spent } if spent.as_microusd() == 1220
    ));
}

#[test]
fn the_budget_cap_stops_before_another_build_turn() {
    let mut script = Script::never_fixes();
    script.turn.cost = Money::from_microusd(400);
    let (outcome, _) = go(limits(10, 100, Some(1000)), &mut script);
    assert_eq!(script.requests.len(), 3, "400, 800, then 1200 reaches 1000");
    let left: Vec<Option<u64>> = script
        .requests
        .iter()
        .map(|r| r.budget_left.map(Money::as_microusd))
        .collect();
    assert_eq!(left, [Some(1000), Some(600), Some(200)]);
    assert_eq!(
        needs_you(&outcome).0,
        &Stop::BudgetCap {
            spent: Money::from_microusd(1200)
        }
    );
}

// Invariant: two turns that change nothing stop the build before a third is
// paid for. The other caps are far away, so only the counter can stop it.
#[test]
fn the_not_working_counter_stops_a_loop_that_changes_nothing() {
    let mut script = Script::never_fixes();
    script.changes_tree = false;
    let (outcome, _) = go(limits(10, 100, None), &mut script);
    assert_eq!(script.requests.len(), 2);
    assert_eq!(needs_you(&outcome).0, &Stop::NotWorking);
}

#[test]
fn one_idle_turn_between_changes_does_not_stop_the_loop() {
    // The counter counts turns in a row, so a turn that changes something resets it.
    struct Alternating(Script, u32);
    impl Round for Alternating {
        type Error = String;
        fn tree(&mut self) -> Result<String, String> {
            self.0.tree()
        }
        fn build(&mut self, request: &BuildRequest) -> Result<BuildTurn, String> {
            self.1 += 1;
            self.0.changes_tree = self.1 % 2 == 1;
            self.0.build(request)
        }
        fn check(&mut self) -> Result<Vec<CheckRun>, String> {
            self.0.check()
        }
    }
    let mut script = Alternating(Script::never_fixes(), 0);
    let outcome = refine(limits(5, 100, None), &mut script, &mut |_| {}).unwrap();
    assert_eq!(needs_you(&outcome).0, &Stop::RoundCap { rounds: 5 });
}

// Invariant: after two failed rounds the next turn is a fresh session told
// what failed, and only that one. Earlier and later turns continue.
#[test]
fn a_fresh_session_starts_after_two_failed_rounds() {
    let mut script = Script::never_fixes();
    let _ = go(limits(4, 100, None), &mut script);
    let sessions: Vec<bool> = script
        .requests
        .iter()
        .map(|r| matches!(r.session, Session::Fresh { .. }))
        .collect();
    assert_eq!(sessions, [false, false, true, false]);
    let Session::Fresh { summary } = &script.requests[2].session else {
        panic!("round 3 is fresh");
    };
    assert!(summary.contains("Round 1 failed"), "{summary}");
    assert!(summary.contains("Round 2 failed"), "{summary}");
}

// Invariant: a failure that cannot be verified goes to the owner at once.
// More rounds would only spend money on something code cannot change.
#[test]
fn an_unknown_goes_to_the_owner_without_another_turn() {
    let mut script = Script::never_fixes();
    script.unrefinable = true;
    let (outcome, _) = go(limits(5, 100, None), &mut script);
    assert_eq!(script.requests.len(), 1);
    assert_eq!(needs_you(&outcome).0, &Stop::NeedsOwner);
}

#[test]
fn every_check_run_is_logged_with_its_evidence() {
    let mut script = Script::never_fixes();
    script.unrefinable = true;
    let (_, events) = go(limits(5, 100, None), &mut script);
    let results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::CheckResult {
                check,
                passed,
                unknowns,
                ..
            } => Some((check.as_str(), *passed, unknowns.len())),
            _ => None,
        })
        .collect();
    assert_eq!(results, [("build", true, 0), ("test", false, 1)]);
}

#[test]
fn a_machinery_error_is_returned_not_counted_as_a_round() {
    let mut script = Script::never_fixes();
    script.fail_build = true;
    let error = refine(limits(3, 10, None), &mut script, &mut |_| {}).unwrap_err();
    assert_eq!(error, "agent did not start");
}
