//! The refine loop: build, check, and hand the failure back until the checks
//! pass or the loop is stuck. Code decides when to stop, never the agent, and a
//! stop is always checked before paying for another build turn.

use std::fmt::Display;

use autopilot_checks::CheckRun;
use autopilot_core::{Event, Money};
use autopilot_profile::Limits as ProfileLimits;

/// Failed rounds after which the next turn starts a fresh session.
const FRESH_SESSION_AFTER: u32 = 2;
/// Consecutive turns that leave the worktree as it was.
const IDLE_TURNS_TO_STOP: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub rounds: u32,
    pub turns: u32,
    pub budget: Option<Money>,
}

impl From<&ProfileLimits> for Limits {
    fn from(limits: &ProfileLimits) -> Self {
        Self {
            rounds: limits.rounds,
            turns: limits.turns,
            budget: limits.budget_microusd,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Session {
    /// Send the failure to the session that wrote the code.
    Continue,
    /// A new session that has never seen the failed attempts, so it is told
    /// what failed instead.
    Fresh { summary: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRequest {
    pub round: u32,
    pub session: Session,
    /// The last failure note. `None` in the first round.
    pub failed: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildTurn {
    pub cost: Money,
    /// Agent turns the build used, for the turn cap.
    pub turns: u32,
}

/// One side of the loop that needs the outside world: the agent and the checks.
pub trait Round {
    type Error: Display;

    /// The worktree's fingerprint, to tell whether a turn changed anything.
    ///
    /// # Errors
    /// Returns the reason the fingerprint could not be taken.
    fn tree(&mut self) -> Result<String, Self::Error>;

    /// # Errors
    /// Returns the reason the agent could not run.
    fn build(&mut self, request: &BuildRequest) -> Result<BuildTurn, Self::Error>;

    /// # Errors
    /// Returns the reason the checks could not run at all.
    fn check(&mut self) -> Result<Vec<CheckRun>, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Stop {
    RoundCap {
        rounds: u32,
    },
    TurnCap {
        turns: u32,
    },
    BudgetCap {
        spent: Money,
    },
    /// The agent changed nothing for two turns in a row.
    NotWorking,
    /// A failure no further build turn can fix, such as a criterion that
    /// cannot be verified.
    NeedsOwner,
}

impl Stop {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::RoundCap { rounds } => format!("still failing after {rounds} rounds"),
            Self::TurnCap { turns } => format!("used all {turns} agent turns"),
            Self::BudgetCap { spent } => format!(
                "the step budget is used up (${:.2} spent)",
                micro_to_dollars(spent.as_microusd())
            ),
            Self::NotWorking => "the agent changed nothing for two turns".into(),
            Self::NeedsOwner => "a check failed that more building cannot fix".into(),
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn micro_to_dollars(microusd: u64) -> f64 {
    microusd as f64 / 1_000_000.0
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Ready,
    /// The owner is needed. `note` is the last failure, word for word.
    NeedsYou {
        stop: Stop,
        note: String,
    },
}

/// The log events for one check run.
#[must_use]
pub fn check_event(run: &CheckRun) -> Event {
    Event::CheckResult {
        check: run.id.clone(),
        passed: run.report.passed(),
        note: run.report.note.clone(),
        artifacts: run.report.artifacts.clone(),
        unknowns: run.report.unknowns.clone(),
        reused: run.reused,
    }
}

fn failure_note(runs: &[CheckRun]) -> String {
    runs.last()
        .map(|run| format!("{}: {}", run.id, run.report.note))
        .unwrap_or_default()
}

/// Run rounds until the checks pass or a stop applies. `emit` receives each
/// event as it happens, so a crash loses nothing already done.
///
/// # Errors
/// Returns the error of a build or check that could not run. That is a fault
/// of the machinery, not a failed round, and it does not count as one.
pub fn refine<R: Round>(
    limits: Limits,
    round: &mut R,
    emit: &mut dyn FnMut(Event),
) -> Result<Outcome, R::Error> {
    let mut number = 0_u32;
    let mut turns_used = 0_u32;
    let mut spent = Money::default();
    let mut idle_turns = 0_u32;
    let mut failed_rounds = 0_u32;
    let mut last_note: Option<String> = None;
    let mut history: Vec<String> = Vec::new();

    loop {
        let stuck = |stop: Stop, note: &Option<String>| Outcome::NeedsYou {
            stop,
            note: note.clone().unwrap_or_default(),
        };
        // Every stop is decided here, before the next build turn is paid for.
        if number >= limits.rounds {
            return Ok(stuck(Stop::RoundCap { rounds: number }, &last_note));
        }
        if turns_used >= limits.turns {
            return Ok(stuck(Stop::TurnCap { turns: turns_used }, &last_note));
        }
        if limits.budget.is_some_and(|budget| spent >= budget) {
            return Ok(stuck(Stop::BudgetCap { spent }, &last_note));
        }
        if idle_turns >= IDLE_TURNS_TO_STOP {
            return Ok(stuck(Stop::NotWorking, &last_note));
        }

        number += 1;
        emit(Event::RoundStarted { round: number });
        let session = if failed_rounds == FRESH_SESSION_AFTER {
            Session::Fresh {
                summary: history.join("\n"),
            }
        } else {
            Session::Continue
        };
        let before = round.tree()?;
        let turn = round.build(&BuildRequest {
            round: number,
            session,
            failed: last_note.clone(),
        })?;
        turns_used = turns_used.saturating_add(turn.turns);
        spent = spent.checked_add(turn.cost).unwrap_or(spent);
        if round.tree()? == before {
            idle_turns += 1;
        } else {
            idle_turns = 0;
        }

        let runs = round.check()?;
        for run in &runs {
            emit(check_event(run));
        }
        let Some(last) = runs.last().filter(|run| !run.report.passed()) else {
            return Ok(Outcome::Ready);
        };
        let note = failure_note(&runs);
        // An unknown is "cannot verify", which no edit to the code resolves, so
        // it goes to the owner now instead of burning the remaining rounds.
        if !last.can_refine || !last.report.unknowns.is_empty() {
            return Ok(Outcome::NeedsYou {
                stop: Stop::NeedsOwner,
                note,
            });
        }
        failed_rounds += 1;
        history.push(format!("Round {number} failed. {note}"));
        last_note = Some(note);
    }
}
