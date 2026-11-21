//! Measuring how well the pipeline does on a fixed set of tickets.
//!
//! This crate holds the parts that need no model: the corpus format, the rule
//! that a run cannot start without a stated dollar cap, the loop that stops
//! when the cap is reached, and the report. The thing that actually runs a
//! ticket is passed in as a closure, so the same loop drives a fake in tests
//! and a real agent in a measured run.
//!
//! A run starts small (five tickets) unless the caller asks for more, and it
//! never counts a ticket it did not run as passed.

use autopilot_core::Money;
use serde::{Deserialize, Serialize};

/// Tickets a run covers unless the caller says otherwise.
pub const DEFAULT_TICKETS: usize = 5;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum Error {
    #[error("a run needs a dollar cap above zero; there is no default")]
    NoCap,
    #[error("the corpus is not usable: {0}")]
    Corpus(String),
    #[error("the first {tickets} tickets could cost about {estimate}, over the cap of {cap}")]
    OverCap {
        tickets: usize,
        estimate: String,
        cap: String,
    },
}

/// One ticket and what makes its result count as good.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Case {
    pub id: String,
    pub title: String,
    pub description: String,
    /// What a judge checks, each a plain sentence. A case with none cannot be
    /// scored, so it is refused.
    pub criteria: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Corpus {
    pub name: String,
    pub cases: Vec<Case>,
}

impl Corpus {
    /// # Errors
    /// Returns an error if the text is not valid JSON, there are no cases, an
    /// id repeats, or a case has no criteria.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let corpus: Self = serde_json::from_str(text).map_err(|e| Error::Corpus(e.to_string()))?;
        if corpus.cases.is_empty() {
            return Err(Error::Corpus("it has no cases".into()));
        }
        for (index, case) in corpus.cases.iter().enumerate() {
            if case.criteria.iter().all(|c| c.trim().is_empty()) {
                return Err(Error::Corpus(format!("{} has no criteria", case.id)));
            }
            if corpus.cases[..index]
                .iter()
                .any(|other| other.id == case.id)
            {
                return Err(Error::Corpus(format!("{} appears twice", case.id)));
            }
        }
        Ok(corpus)
    }
}

/// What running and judging one case came to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Outcome {
    /// One verdict per criterion, in the case's order.
    pub verdicts: Vec<bool>,
    pub spent: Money,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Row {
    pub case: String,
    pub passed: bool,
    pub criteria_met: usize,
    pub criteria: usize,
    pub spent: Money,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Report {
    pub corpus: String,
    pub rows: Vec<Row>,
    /// Cases left unrun because the cap was reached or the limit was met.
    pub not_run: Vec<String>,
    pub spent: Money,
    pub cap: Money,
    pub stopped_by_cap: bool,
}

impl Report {
    #[must_use]
    pub fn passed(&self) -> usize {
        self.rows.iter().filter(|row| row.passed).count()
    }

    /// Passed over run, never over the whole corpus, so a short run is not
    /// read as a good one.
    #[must_use]
    pub fn pass_rate_percent(&self) -> Option<u64> {
        let run = self.rows.len() as u64;
        (run > 0).then(|| self.passed() as u64 * 100 / run)
    }
}

/// The limits of a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Plan {
    pub cap: Money,
    pub tickets: usize,
    /// A guess per ticket, used to refuse a run that cannot fit before it
    /// spends anything.
    pub estimate_per_ticket: Money,
}

impl Plan {
    /// # Errors
    /// Returns an error if the cap is zero or the estimate for the tickets
    /// already exceeds it.
    pub fn new(
        cap: Money,
        tickets: Option<usize>,
        estimate_per_ticket: Money,
    ) -> Result<Self, Error> {
        if cap.as_microusd() == 0 {
            return Err(Error::NoCap);
        }
        let tickets = tickets.unwrap_or(DEFAULT_TICKETS).max(1);
        let estimate = estimate_per_ticket
            .as_microusd()
            .saturating_mul(tickets as u64);
        if estimate > cap.as_microusd() {
            return Err(Error::OverCap {
                tickets,
                estimate: dollars(estimate),
                cap: dollars(cap.as_microusd()),
            });
        }
        Ok(Self {
            cap,
            tickets,
            estimate_per_ticket,
        })
    }
}

fn dollars(microusd: u64) -> String {
    format!("${}.{:06}", microusd / 1_000_000, microusd % 1_000_000)
}

/// Run the cases in order until the ticket limit or the cap is reached. The
/// cap is checked before each case against what has been spent plus the
/// estimate for the next one, so a case is not started that could not finish
/// inside the cap.
pub fn run(corpus: &Corpus, plan: Plan, mut runner: impl FnMut(&Case) -> Outcome) -> Report {
    let mut rows = Vec::new();
    let mut not_run = Vec::new();
    let mut spent = 0_u64;
    let mut stopped_by_cap = false;
    for case in &corpus.cases {
        let over_limit = rows.len() >= plan.tickets;
        let next = spent.saturating_add(plan.estimate_per_ticket.as_microusd());
        if over_limit || stopped_by_cap || next > plan.cap.as_microusd() {
            stopped_by_cap |= !over_limit;
            not_run.push(case.id.clone());
            continue;
        }
        let outcome = runner(case);
        spent = spent.saturating_add(outcome.spent.as_microusd());
        let met = outcome.verdicts.iter().filter(|verdict| **verdict).count();
        // A judge that returned the wrong number of verdicts did not score
        // the case, so it cannot pass.
        let complete = outcome.verdicts.len() == case.criteria.len();
        rows.push(Row {
            case: case.id.clone(),
            passed: complete && met == case.criteria.len(),
            criteria_met: met,
            criteria: case.criteria.len(),
            spent: outcome.spent,
        });
    }
    Report {
        corpus: corpus.name.clone(),
        rows,
        not_run,
        spent: Money::from_microusd(spent),
        cap: plan.cap,
        stopped_by_cap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(count: usize) -> Corpus {
        Corpus {
            name: "t".into(),
            cases: (0..count)
                .map(|n| Case {
                    id: format!("c{n}"),
                    title: "t".into(),
                    description: "d".into(),
                    criteria: vec!["a".into(), "b".into()],
                })
                .collect(),
        }
    }

    fn micro(value: u64) -> Money {
        Money::from_microusd(value)
    }

    // Invariant: a run cannot start without a cap, and cannot start when the
    // estimate for its tickets already exceeds the cap.
    #[test]
    fn a_run_needs_a_cap_that_fits() {
        assert_eq!(Plan::new(micro(0), None, micro(1)), Err(Error::NoCap));
        assert!(matches!(
            Plan::new(micro(4_999), None, micro(1_000)),
            Err(Error::OverCap { tickets: 5, .. })
        ));
        assert_eq!(
            Plan::new(micro(5_000), None, micro(1_000)).unwrap().tickets,
            DEFAULT_TICKETS
        );
    }

    // Invariant: the loop stops at the ticket limit, the unrun cases are
    // named, and the pass rate counts only cases that ran.
    #[test]
    fn it_stops_at_the_ticket_limit_and_rates_only_what_ran() {
        let plan = Plan::new(micro(100_000), Some(2), micro(1_000)).unwrap();
        let report = run(&corpus(4), plan, |_| Outcome {
            verdicts: vec![true, true],
            spent: micro(500),
        });
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.not_run, ["c2", "c3"]);
        assert_eq!(report.spent, micro(1_000));
        assert_eq!(report.pass_rate_percent(), Some(100));
        assert!(!report.stopped_by_cap);
    }

    // Invariant: the cap stops the run before a case that could not fit, a
    // case with a missing verdict does not pass, and spend never passes the cap
    // by more than one case's overrun.
    #[test]
    fn the_cap_stops_the_run_and_a_short_verdict_list_fails() {
        let plan = Plan::new(micro(2_500), Some(5), micro(500)).unwrap();
        let mut calls = 0;
        let report = run(&corpus(5), plan, |_| {
            calls += 1;
            Outcome {
                verdicts: if calls == 1 {
                    vec![true]
                } else {
                    vec![true, true]
                },
                spent: micro(1_000),
            }
        });
        assert_eq!(report.rows.len(), 3);
        assert!(!report.rows[0].passed);
        assert!(report.rows[1].passed);
        assert!(report.stopped_by_cap);
        assert_eq!(report.not_run.len(), 2);
    }

    // Invariant: a corpus with no cases, repeated ids or an unscorable case is
    // refused at the door.
    #[test]
    fn a_bad_corpus_is_refused() {
        assert!(Corpus::parse(r#"{"name":"x","cases":[]}"#).is_err());
        let no_criteria =
            r#"{"name":"x","cases":[{"id":"a","title":"t","description":"d","criteria":[]}]}"#;
        assert!(Corpus::parse(no_criteria).is_err());
        let twice = r#"{"name":"x","cases":[
            {"id":"a","title":"t","description":"d","criteria":["c"]},
            {"id":"a","title":"t","description":"d","criteria":["c"]}]}"#;
        assert!(Corpus::parse(twice).is_err());
    }
}
