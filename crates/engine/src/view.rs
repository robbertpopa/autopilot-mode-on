//! What a window shows, worked out from a ticket's history and nothing else.
//!
//! These are plain structs with no drawing code, so every display rule is
//! covered by a test here and a client only paints them.

use std::path::Path;

use autopilot_core::{Event, Money, RecordedEvent, RequestId, Step, TicketId};
use autopilot_pipeline::{StepState, fold, summary};
use autopilot_store::SqliteStore;

use crate::{DATABASE, Error, watch};

/// The chip next to a ticket in the sidebar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Badge {
    /// A step is waiting for the owner.
    NeedsYou,
    Running,
    Paused,
    Failed,
    Interrupted,
    Done,
    /// Created, nothing running now.
    Idle,
}

impl Badge {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Failed => "Failed",
            Self::Interrupted => "Interrupted",
            Self::Done => "Done",
            Self::Idle => "Idle",
        }
    }
}

/// One card of the pipeline strip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepView {
    pub step: Step,
    pub state: StepState,
    /// The "In short" line, made by code from the step's result. Empty until it has one.
    pub in_short: Option<String>,
    pub api: Money,
    pub subscription: Money,
}

/// One check inside a round.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckRow {
    pub name: String,
    pub passed: bool,
    pub note: String,
    pub unknowns: Vec<String>,
}

/// One pass of Build ⟲ Check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundView {
    pub round: u32,
    pub checks: Vec<CheckRow>,
    pub started_ms: u64,
}

impl RoundView {
    #[must_use]
    pub fn passed(&self) -> usize {
        self.checks.iter().filter(|check| check.passed).count()
    }

    #[must_use]
    pub fn failure(&self) -> Option<&CheckRow> {
        self.checks.iter().find(|check| !check.passed)
    }
}

/// One line of the timeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineLine {
    pub at_ms: u64,
    pub text: String,
}

/// One note the agent made while a step ran.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogLine {
    pub step: Option<Step>,
    pub at_ms: u64,
    pub text: String,
}

/// One cost tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CostLine {
    pub step: Option<Step>,
    pub at_ms: u64,
    pub amount: Money,
    pub estimated: bool,
}

/// One attachment as the window lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentView {
    pub name: String,
    pub kind: autopilot_core::AttachmentKind,
    pub bytes: u64,
    /// Files counted, for a folder; 1 for a file.
    pub items: u64,
    /// Where a copied file lives, relative to the data directory, or the folder's own path.
    pub stored: String,
}

/// Everything the Run and Step views need for one ticket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketView {
    pub id: TicketId,
    pub title: String,
    pub badge: Badge,
    pub steps: Vec<StepView>,
    pub rounds: Vec<RoundView>,
    pub timeline: Vec<TimelineLine>,
    pub log: Vec<LogLine>,
    pub costs: Vec<CostLine>,
    /// The recorded result of a step, as readable JSON.
    pub outputs: Vec<(Step, String)>,
    pub attachments: Vec<AttachmentView>,
    /// Understand said the ticket changes a screen, so Design must run.
    pub needs_design: bool,
    /// Plain-words explanations kept in the history, by step.
    pub explained: Vec<(Step, String)>,
    /// Exact spend through an API key.
    pub api: Money,
    /// Estimated spend on a subscription. Never added to `api`.
    pub subscription: Money,
    pub started_ms: u64,
    pub last_ms: u64,
    pub asking: Option<(Step, RequestId)>,
    pub last_seq: u64,
}

impl TicketView {
    #[must_use]
    pub fn step(&self, step: Step) -> Option<&StepView> {
        self.steps.iter().find(|view| view.step == step)
    }

    /// The step that holds the ticket, if any.
    #[must_use]
    pub fn active(&self) -> Option<Step> {
        self.steps
            .iter()
            .find(|view| view.state.is_active())
            .map(|view| view.step)
    }
}

/// Plain name of a step, as the design words it.
#[must_use]
pub const fn step_name(step: Step) -> &'static str {
    match step {
        Step::Understand => "Understand",
        Step::Clarify => "Clarify",
        Step::Design => "Design",
        Step::Plan => "Plan",
        Step::BuildCheck => "Build ⟲ Check",
        Step::PullRequest => "Pull request",
    }
}

/// Build the view of one ticket from its full history.
///
/// # Errors
/// Returns an error if the history is empty or not a valid history.
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the history, one arm per event"
)]
pub fn ticket_view(history: &[RecordedEvent]) -> Result<TicketView, Error> {
    let first = history.first().ok_or(Error::UnknownTicket)?;
    let state = fold(history).map_err(|error| Error::History(error.to_string()))?;
    let title = history
        .iter()
        .find_map(|record| match &record.event.kind {
            Event::TicketCreated { title } => Some(title.clone()),
            _ => None,
        })
        .unwrap_or_default();

    let mut api = Money::default();
    let mut subscription = Money::default();
    let mut per_step: Vec<(Step, Money, Money)> = Vec::new();
    let mut rounds: Vec<RoundView> = Vec::new();
    let mut timeline = Vec::new();
    let mut log = Vec::new();
    let mut costs = Vec::new();
    let mut outputs: Vec<(Step, String)> = Vec::new();
    let mut attachments = Vec::new();
    let mut needs_design = false;
    let mut explained: Vec<(Step, String)> = Vec::new();
    for record in history {
        let at_ms = record.event.at_ms;
        let step = record.event.step;
        match &record.event.kind {
            Event::AgentActivity { summary } => log.push(LogLine {
                step,
                at_ms,
                text: summary.clone(),
            }),
            Event::UnderstandResult { result } => {
                keep_output(&mut outputs, Step::Understand, result);
                needs_design = result.needs_design;
            }
            Event::PlanResult { plan } => keep_output(&mut outputs, Step::Plan, plan),
            Event::DesignResult { design } => {
                keep_output(&mut outputs, Step::Design, &design.notes);
            }
            Event::Explained { text } => {
                if let Some(step) = record.event.step {
                    explained.retain(|(kept, _)| *kept != step);
                    explained.push((step, text.clone()));
                }
            }
            Event::Attached {
                name,
                kind,
                bytes,
                items,
                stored,
                ..
            } => {
                attachments.push(AttachmentView {
                    name: name.clone(),
                    kind: *kind,
                    bytes: *bytes,
                    items: *items,
                    stored: stored.clone(),
                });
                timeline.push(line(at_ms, format!("Attached {name}")));
            }
            Event::CostTick { amount, estimated } => {
                costs.push(CostLine {
                    step,
                    at_ms,
                    amount: *amount,
                    estimated: *estimated,
                });
                let total = if *estimated {
                    &mut subscription
                } else {
                    &mut api
                };
                *total = total.checked_add(*amount).unwrap_or(*total);
                if let Some(step) = step {
                    spend(&mut per_step, step, *amount, *estimated);
                }
            }
            Event::RoundStarted { round } => {
                rounds.push(RoundView {
                    round: *round,
                    checks: vec![],
                    started_ms: at_ms,
                });
                timeline.push(line(at_ms, format!("Build round {round} started")));
            }
            Event::CheckResult {
                check,
                passed,
                note,
                unknowns,
                ..
            } => {
                if let Some(round) = rounds.last_mut() {
                    round.checks.push(CheckRow {
                        name: check.clone(),
                        passed: *passed,
                        note: note.clone(),
                        unknowns: unknowns.clone(),
                    });
                    let outcome = if *passed { "passed" } else { "failed" };
                    let text = if note.is_empty() {
                        format!("Check {check} {outcome} (round {})", round.round)
                    } else {
                        format!("Check {check} {outcome} · {note}")
                    };
                    timeline.push(line(at_ms, text));
                }
            }
            other => {
                if let Some(text) = timeline_text(step, other) {
                    timeline.push(line(at_ms, text));
                }
            }
        }
    }

    let lines = summary::from_history(history);
    let steps = Step::ALL
        .into_iter()
        .map(|step| {
            let (step_api, step_sub) = per_step
                .iter()
                .find(|(s, _, _)| *s == step)
                .map_or((Money::default(), Money::default()), |(_, a, b)| (*a, *b));
            StepView {
                step,
                state: state.step(step).clone(),
                in_short: lines
                    .iter()
                    .find(|(s, _)| *s == step)
                    .map(|(_, text)| text.clone()),
                api: step_api,
                subscription: step_sub,
            }
        })
        .collect::<Vec<_>>();

    let asking = steps.iter().find_map(|view| match &view.state {
        StepState::Asking(request) => Some((view.step, request.clone())),
        _ => None,
    });
    let badge = badge(&steps, state.is_done());
    Ok(TicketView {
        id: first.ticket.clone(),
        title,
        badge,
        steps,
        rounds,
        timeline,
        log,
        costs,
        outputs,
        attachments,
        needs_design,
        explained,
        api,
        subscription,
        started_ms: first.event.at_ms,
        last_ms: history.last().map_or(0, |record| record.event.at_ms),
        asking,
        last_seq: history.last().map_or(0, |record| record.seq),
    })
}

fn keep_output(outputs: &mut Vec<(Step, String)>, step: Step, value: &impl serde::Serialize) {
    outputs.retain(|(kept, _)| *kept != step);
    if let Ok(text) = serde_json::to_string_pretty(value) {
        outputs.push((step, text));
    }
}

fn spend(per_step: &mut Vec<(Step, Money, Money)>, step: Step, amount: Money, estimated: bool) {
    if !per_step.iter().any(|(s, _, _)| *s == step) {
        per_step.push((step, Money::default(), Money::default()));
    }
    if let Some((_, api, subscription)) = per_step.iter_mut().find(|(s, _, _)| *s == step) {
        let total = if estimated { subscription } else { api };
        *total = total.checked_add(amount).unwrap_or(*total);
    }
}

fn line(at_ms: u64, text: String) -> TimelineLine {
    TimelineLine { at_ms, text }
}

fn timeline_text(step: Option<Step>, event: &Event) -> Option<String> {
    let name = step.map(step_name);
    Some(match event {
        Event::TicketCreated { title } => format!("Ticket created: {title}"),
        Event::StepStarted { .. } => format!("{} started", name?),
        Event::StepFinished => format!("{} done", name?),
        Event::StepSkipped { reason } => format!("{} skipped · {reason}", name?),
        Event::PlanApproved { .. } => "Plan approved".to_owned(),
        Event::Asked { .. } => format!("{} is waiting for you", name?),
        Event::Answered { .. } => format!("You answered {}", name?),
        Event::StepReset { and_after } => {
            if *and_after {
                format!("{} and later steps reset to run again", name?)
            } else {
                format!("{} reset to run again", name?)
            }
        }
        Event::Paused => "Paused".to_owned(),
        Event::Resumed => "Resumed".to_owned(),
        Event::Interrupted { reason } => format!("Interrupted · {reason}"),
        Event::Failed { reason } => format!("{} failed · {reason}", name?),
        Event::Done => "Ticket done".to_owned(),
        _ => return None,
    })
}

fn badge(steps: &[StepView], done: bool) -> Badge {
    if done {
        return Badge::Done;
    }
    let states = || steps.iter().map(|view| &view.state);
    if states().any(|s| matches!(s, StepState::Asking(_))) {
        Badge::NeedsYou
    } else if states().any(|s| matches!(s, StepState::Paused)) {
        Badge::Paused
    } else if states().any(|s| matches!(s, StepState::Running)) {
        Badge::Running
    } else if states().any(|s| matches!(s, StepState::Failed(_))) {
        Badge::Failed
    } else if states().any(|s| matches!(s, StepState::Interrupted(_))) {
        Badge::Interrupted
    } else {
        Badge::Idle
    }
}

/// The ids of every ticket in a data directory, oldest first. An empty or
/// missing directory has none.
///
/// # Errors
/// Returns an error if the database exists but cannot be read.
pub fn tickets(directory: &Path) -> Result<Vec<TicketId>, Error> {
    let path = directory.join(DATABASE);
    if !path.exists() {
        return Ok(vec![]);
    }
    Ok(SqliteStore::open_read_only(path)?.tickets()?)
}

/// Read and build the view of one ticket.
///
/// # Errors
/// Returns an error for an unknown ticket or an unreadable history.
pub fn load_ticket(directory: &Path, ticket: &TicketId) -> Result<TicketView, Error> {
    ticket_view(&watch(directory, ticket, 0)?)
}

/// What every ticket has cost since `since_ms`, split into exact and estimated.
///
/// # Errors
/// Returns an error if a history cannot be read.
pub fn spent_since(directory: &Path, since_ms: u64) -> Result<(Money, Money), Error> {
    let mut api = Money::default();
    let mut subscription = Money::default();
    for ticket in tickets(directory)? {
        for record in watch(directory, &ticket, 0)? {
            if let Event::CostTick { amount, estimated } = &record.event.kind {
                if record.event.at_ms < since_ms {
                    continue;
                }
                let total = if *estimated {
                    &mut subscription
                } else {
                    &mut api
                };
                *total = total.checked_add(*amount).unwrap_or(*total);
            }
        }
    }
    Ok((api, subscription))
}

/// Dollars with two decimals, for a cost label.
#[must_use]
pub fn dollars(money: Money) -> String {
    let cents = money.as_microusd().div_ceil(10_000);
    format!("${}.{:02}", cents / 100, cents % 100)
}

#[cfg(test)]
mod tests {
    use autopilot_core::{AttemptId, NewEvent};

    use super::*;

    fn record(seq: u64, step: Option<Step>, kind: Event) -> RecordedEvent {
        RecordedEvent {
            ticket: TicketId::from("t"),
            seq,
            event: NewEvent {
                at_ms: seq * 1000,
                step,
                kind,
            },
        }
    }

    fn history(kinds: Vec<(Option<Step>, Event)>) -> Vec<RecordedEvent> {
        kinds
            .into_iter()
            .zip(1..)
            .map(|((step, kind), seq)| record(seq, step, kind))
            .collect()
    }

    fn started() -> Event {
        Event::StepStarted {
            attempt: AttemptId::from("a"),
            session: None,
        }
    }

    fn created() -> (Option<Step>, Event) {
        (
            None,
            Event::TicketCreated {
                title: "Timezone fix".into(),
            },
        )
    }

    // Invariant: the sidebar badge follows the most urgent step: asking beats
    // running, running beats a past failure, and a finished ticket is done.
    #[test]
    fn badge_follows_the_most_urgent_step() {
        let idle = ticket_view(&history(vec![created()])).unwrap();
        assert_eq!(idle.badge, Badge::Idle);

        let running = ticket_view(&history(vec![
            created(),
            (Some(Step::Understand), started()),
        ]))
        .unwrap();
        assert_eq!(running.badge, Badge::Running);
        assert_eq!(running.active(), Some(Step::Understand));

        let asking = ticket_view(&history(vec![
            created(),
            (Some(Step::Understand), started()),
            (
                Some(Step::Understand),
                Event::Asked {
                    request: RequestId::from("r1"),
                },
            ),
        ]))
        .unwrap();
        assert_eq!(asking.badge, Badge::NeedsYou);
        assert_eq!(
            asking.asking,
            Some((Step::Understand, RequestId::from("r1")))
        );

        let failed = ticket_view(&history(vec![
            created(),
            (Some(Step::Understand), started()),
            (
                Some(Step::Understand),
                Event::Failed {
                    reason: "no".into(),
                },
            ),
        ]))
        .unwrap();
        assert_eq!(failed.badge, Badge::Failed);
    }

    // Invariant: a skipped step keeps its reason, and the timeline says why.
    #[test]
    fn a_skipped_step_shows_its_reason() {
        let view = ticket_view(&history(vec![
            created(),
            (
                Some(Step::Clarify),
                Event::StepSkipped {
                    reason: "no open questions".into(),
                },
            ),
        ]))
        .unwrap();
        let clarify = view.step(Step::Clarify).unwrap();
        assert_eq!(
            clarify.state,
            StepState::Skipped("no open questions".into())
        );
        assert!(
            view.timeline
                .iter()
                .any(|line| line.text == "Clarify skipped · no open questions")
        );
    }

    // Invariant: exact API spend and estimated subscription spend are never
    // added together, in total or per step.
    #[test]
    fn api_and_subscription_cost_stay_apart() {
        let view = ticket_view(&history(vec![
            created(),
            (
                Some(Step::Plan),
                Event::CostTick {
                    amount: Money::from_microusd(40_000),
                    estimated: false,
                },
            ),
            (
                Some(Step::Plan),
                Event::CostTick {
                    amount: Money::from_microusd(620_000),
                    estimated: true,
                },
            ),
        ]))
        .unwrap();
        assert_eq!(view.api, Money::from_microusd(40_000));
        assert_eq!(view.subscription, Money::from_microusd(620_000));
        let plan = view.step(Step::Plan).unwrap();
        assert_eq!(plan.api, Money::from_microusd(40_000));
        assert_eq!(plan.subscription, Money::from_microusd(620_000));
        assert_eq!(dollars(view.subscription), "$0.62");
    }

    // Invariant: rounds collect their checks in order, and a failed check is
    // easy to find for the "Round 1 failed" note.
    #[test]
    fn rounds_hold_their_checks() {
        let check = |name: &str, passed: bool| Event::CheckResult {
            check: name.into(),
            passed,
            note: String::new(),
            artifacts: vec![],
            unknowns: vec![],
            reused: false,
        };
        let build = Some(Step::BuildCheck);
        let view = ticket_view(&history(vec![
            created(),
            (build, started()),
            (build, Event::RoundStarted { round: 1 }),
            (build, check("fmt", true)),
            (build, check("tests", false)),
            (build, Event::RoundStarted { round: 2 }),
            (build, check("fmt", true)),
        ]))
        .unwrap();
        assert_eq!(view.rounds.len(), 2);
        assert_eq!(view.rounds[0].passed(), 1);
        assert_eq!(view.rounds[0].failure().unwrap().name, "tests");
        assert_eq!(view.rounds[1].checks.len(), 1);
    }

    #[test]
    fn an_empty_history_is_an_unknown_ticket() {
        assert!(matches!(ticket_view(&[]), Err(Error::UnknownTicket)));
    }

    #[test]
    fn dollars_round_up_to_the_cent() {
        assert_eq!(dollars(Money::from_microusd(0)), "$0.00");
        assert_eq!(dollars(Money::from_microusd(1)), "$0.01");
        assert_eq!(dollars(Money::from_microusd(1_420_000)), "$1.42");
    }

    #[test]
    fn a_missing_data_directory_has_no_tickets() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(tickets(&directory.path().join("nothing")).unwrap(), vec![]);
    }
}
