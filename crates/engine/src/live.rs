//! What a running step is doing right now, read from the log alone: the phase,
//! how long since anything happened, and a checklist of sub-steps that stays
//! after the step finishes. Nothing is stored; the clock comes from the caller.

use autopilot_core::{Money, Step};
use autopilot_pipeline::StepState;

use crate::view::TicketView;

/// No event for this long reads as "quiet": a model can think for a while.
pub const QUIET_MS: u64 = 30_000;
/// No event for this long reads as "may be stuck".
pub const STALLED_MS: u64 = 180_000;

/// How recently the run did anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Liveness {
    Active,
    Quiet,
    Stalled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveStatus {
    pub step: Step,
    /// What the step is doing, in plain words.
    pub phase: String,
    /// The agent's own last words, shortened, if it said any.
    pub detail: Option<String>,
    /// Milliseconds since the last event of the ticket.
    pub idle_ms: u64,
    pub liveness: Liveness,
    pub spent: Money,
}

#[must_use]
pub fn liveness(idle_ms: u64) -> Liveness {
    if idle_ms >= STALLED_MS {
        Liveness::Stalled
    } else if idle_ms >= QUIET_MS {
        Liveness::Quiet
    } else {
        Liveness::Active
    }
}

/// The phase a line of the agent's activity stands for.
fn phase_of(text: &str) -> &'static str {
    let Some(tool) = text.strip_prefix("tool requested: ") else {
        return "thinking";
    };
    let name = tool.split(':').next().unwrap_or_default().trim();
    match name {
        "Read" | "Grep" | "Glob" | "LS" | "WebFetch" | "WebSearch" => "reading the project",
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "editing files",
        "Bash" => "running commands",
        _ => "working with a tool",
    }
}

fn shorten(text: &str) -> String {
    const MAX: usize = 90;
    let flat = text.replace('\n', " ");
    if flat.chars().count() <= MAX {
        return flat;
    }
    let cut: String = flat.chars().take(MAX).collect();
    format!("{cut}…")
}

/// The status of `step` at time `now_ms`, or `None` when it is not running.
#[must_use]
pub fn live_status(view: &TicketView, step: Step, now_ms: u64) -> Option<LiveStatus> {
    let card = view.step(step)?;
    if card.state != StepState::Running {
        return None;
    }
    let last_words = view
        .log
        .iter()
        .rev()
        .find(|line| line.step == Some(step) && !line.text.starts_with("publish offer"));
    let words = last_words.map_or("starting", |line| phase_of(&line.text));
    let phase = if step == Step::BuildCheck {
        match view.rounds.last() {
            Some(round) if round.checks.is_empty() => {
                format!("round {}: build agent, {words}", round.round)
            }
            Some(round) => format!(
                "round {}: {} check(s) reported, the next one is running",
                round.round,
                round.checks.len()
            ),
            None => "starting the build".to_owned(),
        }
    } else {
        words.to_owned()
    };
    let idle_ms = now_ms.saturating_sub(view.last_ms);
    Some(LiveStatus {
        step,
        phase,
        detail: last_words.map(|line| shorten(&line.text)),
        idle_ms,
        liveness: liveness(idle_ms),
        spent: card.api.checked_add(card.subscription).unwrap_or(card.api),
    })
}

/// One sub-step of a step and whether it happened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubStep {
    pub text: String,
    pub done: bool,
    /// False for a check that ran and failed.
    pub ok: bool,
}

/// The sub-steps a step went through, in order. They are read from the log, so
/// they are the same before, during and after the step runs.
#[must_use]
pub fn sub_steps(view: &TicketView, step: Step) -> Vec<SubStep> {
    let Some(card) = view.step(step) else {
        return Vec::new();
    };
    let item = |text: &str, done: bool| SubStep {
        text: text.to_owned(),
        done,
        ok: true,
    };
    if step == Step::BuildCheck {
        let mut items = Vec::new();
        for round in &view.rounds {
            items.push(item(&format!("Round {}: build turn", round.round), true));
            for check in &round.checks {
                items.push(SubStep {
                    text: format!("Round {}: {}", round.round, check.name),
                    done: true,
                    ok: check.passed,
                });
            }
        }
        if card.state == StepState::Running {
            items.push(item("Next: the following check or turn", false));
        }
        return items;
    }
    let worked = view.log.iter().any(|line| line.step == Some(step));
    let has_result = view.outputs.iter().any(|(each, _)| *each == step);
    let started = !matches!(card.state, StepState::Idle | StepState::Skipped(_));
    vec![
        item("Started", started),
        item("The agent worked on it", worked || has_result),
        item("A result was recorded", has_result),
        item("Finished", card.state == StepState::Passed),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_quiet_then_stalled() {
        assert_eq!(liveness(0), Liveness::Active);
        assert_eq!(liveness(QUIET_MS - 1), Liveness::Active);
        assert_eq!(liveness(QUIET_MS), Liveness::Quiet);
        assert_eq!(liveness(STALLED_MS), Liveness::Stalled);
    }

    #[test]
    fn a_tool_line_names_its_phase() {
        assert_eq!(phase_of("tool requested: Edit"), "editing files");
        assert_eq!(
            phase_of("tool requested: Bash: run tests"),
            "running commands"
        );
        assert_eq!(phase_of("tool requested: Grep"), "reading the project");
        assert_eq!(phase_of("assistant: let me look"), "thinking");
    }

    fn history(
        kinds: Vec<(Option<Step>, autopilot_core::Event)>,
    ) -> Vec<autopilot_core::RecordedEvent> {
        kinds
            .into_iter()
            .zip(1_u64..)
            .map(|((step, kind), seq)| autopilot_core::RecordedEvent {
                ticket: autopilot_core::TicketId::from("t"),
                seq,
                event: autopilot_core::NewEvent {
                    at_ms: seq * 1000,
                    step,
                    kind,
                },
            })
            .collect()
    }

    fn running_build(extra: Vec<(Option<Step>, autopilot_core::Event)>) -> TicketView {
        use autopilot_core::{AttemptId, Event};
        let build = Some(Step::BuildCheck);
        let mut kinds = vec![
            (None, Event::TicketCreated { title: "t".into() }),
            (
                build,
                Event::StepStarted {
                    attempt: AttemptId::from("b"),
                    session: None,
                },
            ),
            (build, Event::RoundStarted { round: 1 }),
        ];
        kinds.extend(extra);
        crate::view::ticket_view(&history(kinds)).unwrap()
    }

    fn said(text: &str) -> (Option<Step>, autopilot_core::Event) {
        (
            Some(Step::BuildCheck),
            autopilot_core::Event::AgentActivity {
                summary: text.into(),
            },
        )
    }

    // Invariant: a running build says what it is doing from the agent's last
    // words, and how long since anything happened is told apart as active,
    // quiet and stalled. A page that showed only "Running" could not tell a
    // thinking agent from a dead one.
    #[test]
    fn a_running_build_reports_its_phase_and_how_long_it_was_quiet() {
        let view = running_build(vec![said("tool requested: Edit")]);
        let at = view.last_ms;
        let active = live_status(&view, Step::BuildCheck, at + 3_000).unwrap();
        assert_eq!(active.phase, "round 1: build agent, editing files");
        assert_eq!(active.liveness, Liveness::Active);
        assert_eq!(active.detail.as_deref(), Some("tool requested: Edit"));
        let quiet = live_status(&view, Step::BuildCheck, at + 40_000).unwrap();
        assert_eq!(quiet.liveness, Liveness::Quiet);
        assert_eq!(quiet.idle_ms, 40_000);
        let stalled = live_status(&view, Step::BuildCheck, at + 600_000).unwrap();
        assert_eq!(stalled.liveness, Liveness::Stalled);
        let thinking = running_build(vec![said("assistant: let me look")]);
        assert_eq!(
            live_status(&thinking, Step::BuildCheck, thinking.last_ms)
                .unwrap()
                .phase,
            "round 1: build agent, thinking"
        );
    }

    // Invariant: once a check has reported, the round is in its checks, and the
    // sub-steps list every check that ran, failed ones marked, after the step
    // ends too.
    #[test]
    fn sub_steps_list_each_check_and_stay_after_the_step_ends() {
        use autopilot_core::Event;
        let build = Some(Step::BuildCheck);
        let result = |name: &str, passed| {
            (
                build,
                Event::CheckResult {
                    check: name.into(),
                    passed,
                    note: String::new(),
                    artifacts: vec![],
                    unknowns: vec![],
                    reused: false,
                },
            )
        };
        let view = running_build(vec![result("build", true), result("tests", false)]);
        let live = live_status(&view, Step::BuildCheck, view.last_ms).unwrap();
        assert_eq!(
            live.phase,
            "round 1: 2 check(s) reported, the next one is running"
        );
        let items = sub_steps(&view, Step::BuildCheck);
        let texts: Vec<&str> = items.iter().map(|item| item.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Round 1: build turn",
                "Round 1: build",
                "Round 1: tests",
                "Next: the following check or turn"
            ]
        );
        assert!(items[1].ok && !items[2].ok && !items[3].done);
        let mut finished = vec![result("build", true)];
        finished.push((build, Event::StepFinished));
        let done = running_build(finished);
        assert!(live_status(&done, Step::BuildCheck, done.last_ms).is_none());
        assert_eq!(sub_steps(&done, Step::BuildCheck).len(), 2);
    }
}
