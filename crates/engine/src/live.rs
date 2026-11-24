//! What a running step is doing right now, read from the log alone: the phase,
//! how long since anything happened, and a checklist of sub-steps that stays
//! after the step finishes. Nothing is stored; the clock comes from the caller.

use autopilot_core::{Money, Step};
use autopilot_pipeline::StepState;

use crate::view::{Fact, TicketView};

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

/// Where one item of a step's checklist stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubState {
    Pending,
    /// The item the step is on now.
    Active,
    Done,
    /// The step stopped here, with the reason in its own words.
    Stopped(String),
    /// The step was skipped, with its reason.
    Skipped(String),
    /// A check or turn that ran and did not pass.
    Failed,
}

/// One item of a step's checklist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubStep {
    pub text: String,
    pub state: SubState,
}

/// What an item waits for in the log.
#[derive(Clone, Copy)]
enum Need {
    Started,
    Worked,
    Result,
    Asked,
    Answered,
    Approved,
    Finished,
}

const fn plan_of(step: Step) -> &'static [(&'static str, Need)] {
    match step {
        Step::Understand => &[
            ("Read your request and attachments", Need::Started),
            ("Look through the project", Need::Worked),
            ("Write down what it understood", Need::Result),
            ("Keep the result", Need::Finished),
        ],
        Step::Clarify => &[
            ("Check whether anything is unclear", Need::Started),
            ("Ask you the open questions", Need::Asked),
            ("Take your answers", Need::Answered),
            ("Settle Clarify", Need::Finished),
        ],
        Step::Design => &[
            ("Start the design", Need::Started),
            ("Draw the mockups", Need::Result),
            ("Wait for your approval", Need::Answered),
            ("Settle the design", Need::Finished),
        ],
        Step::Plan => &[
            ("Start the plan", Need::Started),
            ("Write the criteria and the test plan", Need::Result),
            ("Wait for your approval of the plan", Need::Approved),
            ("Settle the plan", Need::Finished),
        ],
        Step::BuildCheck => &[
            ("Build turn", Need::Started),
            ("Checks, cheapest first", Need::Worked),
            ("Review of the change", Need::Result),
        ],
        Step::PullRequest => &[
            ("Prepare the commit", Need::Started),
            ("Ask you to approve the push", Need::Asked),
            ("Take your approval", Need::Answered),
            ("Push and open the draft pull request", Need::Finished),
        ],
        Step::Custom(_) => &[
            ("Start your step", Need::Started),
            ("Do the work", Need::Result),
            ("Finish", Need::Finished),
        ],
    }
}

fn happened(view: &TicketView, step: Step, need: Need) -> bool {
    let has = |wanted: &dyn Fn(&Fact) -> bool| {
        view.facts
            .iter()
            .any(|(each, fact)| *each == step && wanted(fact))
    };
    match need {
        Need::Started => has(&|fact| matches!(fact, Fact::Started)),
        Need::Worked => view.log.iter().any(|line| line.step == Some(step)),
        Need::Result => has(&|fact| matches!(fact, Fact::Result)),
        Need::Asked => has(&|fact| matches!(fact, Fact::Asked)),
        Need::Answered => has(&|fact| matches!(fact, Fact::Answered)),
        Need::Approved => has(&|fact| matches!(fact, Fact::Approved)),
        Need::Finished => has(&|fact| matches!(fact, Fact::Finished)),
    }
}

/// The checklist of a step: what it will do, in order, with what has happened
/// ticked. It reads only the log, so it is the same before the step starts,
/// while it runs, after it ends and after a restart.
#[must_use]
pub fn sub_steps(view: &TicketView, step: Step) -> Vec<SubStep> {
    let Some(card) = view.step(step) else {
        return Vec::new();
    };
    if let StepState::Skipped(reason) = &card.state {
        return plan_of(step)
            .iter()
            .map(|(text, _)| SubStep {
                text: (*text).to_owned(),
                state: SubState::Skipped(reason.clone()),
            })
            .collect();
    }
    if step == Step::BuildCheck && !view.rounds.is_empty() {
        return build_rounds(view, card);
    }
    let settled = card.state == StepState::Passed;
    let mut items = Vec::new();
    let mut marked = false;
    for (text, need) in plan_of(step) {
        let state = if settled || happened(view, step, *need) {
            SubState::Done
        } else if marked {
            SubState::Pending
        } else {
            marked = true;
            match &card.state {
                StepState::Running | StepState::Asking(_) | StepState::Paused => SubState::Active,
                StepState::Failed(reason) | StepState::Interrupted(reason) => {
                    SubState::Stopped(reason.clone())
                }
                _ => SubState::Pending,
            }
        };
        items.push(SubStep {
            text: (*text).to_owned(),
            state,
        });
    }
    items
}

/// A build that has begun lists what actually ran, round by round.
fn build_rounds(view: &TicketView, card: &crate::view::StepView) -> Vec<SubStep> {
    let mut items = Vec::new();
    for round in &view.rounds {
        items.push(SubStep {
            text: format!("Round {}: build turn", round.round),
            state: SubState::Done,
        });
        for check in &round.checks {
            items.push(SubStep {
                text: format!("Round {}: {}", round.round, check.name),
                state: if check.passed {
                    SubState::Done
                } else {
                    SubState::Failed
                },
            });
        }
    }
    let state = match &card.state {
        StepState::Running | StepState::Asking(_) | StepState::Paused => Some(SubState::Active),
        StepState::Failed(reason) | StepState::Interrupted(reason) => {
            Some(SubState::Stopped(reason.clone()))
        }
        _ => None,
    };
    if let Some(state) = state {
        let text = if matches!(state, SubState::Active) {
            "Next: the following check or turn"
        } else {
            "Stopped here"
        };
        items.push(SubStep {
            text: text.to_owned(),
            state,
        });
    }
    items
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
        assert_eq!(items[1].state, SubState::Done);
        assert_eq!(items[2].state, SubState::Failed);
        assert_eq!(items[3].state, SubState::Active);
        let mut finished = vec![result("build", true)];
        finished.push((build, Event::StepFinished));
        let done = running_build(finished);
        assert!(live_status(&done, Step::BuildCheck, done.last_ms).is_none());
        assert_eq!(sub_steps(&done, Step::BuildCheck).len(), 2);
    }

    fn plan_view(extra: Vec<(Option<Step>, autopilot_core::Event)>) -> TicketView {
        use autopilot_core::Event;
        let mut kinds = vec![(None, Event::TicketCreated { title: "t".into() })];
        kinds.extend(extra);
        crate::view::ticket_view(&history(kinds)).unwrap()
    }

    fn states(view: &TicketView, step: Step) -> Vec<SubState> {
        sub_steps(view, step)
            .into_iter()
            .map(|item| item.state)
            .collect()
    }

    // Invariant: a step's checklist is the same list before it starts, while
    // it runs, and after it ends, and every tick comes from a recorded event: a
    // step that has not begun shows every item pending, the item it is on is
    // marked, a step that stopped marks where and why, a skipped one says why.
    // A list that only appeared once the step ran, or ticked by guesses, would
    // show the owner something the log does not say.
    #[test]
    fn a_checklist_is_the_same_list_before_during_and_after_a_step() {
        use autopilot_core::{AttemptId, Event, Understanding};
        let step = Some(Step::Understand);
        let started = (
            step,
            Event::StepStarted {
                attempt: AttemptId::from("a"),
                session: None,
            },
        );
        let before = plan_view(vec![]);
        assert_eq!(
            states(&before, Step::Understand),
            vec![SubState::Pending; 4]
        );
        let texts_before: Vec<String> = sub_steps(&before, Step::Understand)
            .into_iter()
            .map(|item| item.text)
            .collect();

        let running = plan_view(vec![
            started.clone(),
            said_for(step, "tool requested: Read"),
        ]);
        assert_eq!(
            states(&running, Step::Understand),
            [
                SubState::Done,
                SubState::Done,
                SubState::Active,
                SubState::Pending
            ]
        );

        let result = Understanding {
            summary: "s".into(),
            ambiguity: autopilot_core::Ambiguity::all(autopilot_core::Clarity::Clear),
            open_questions: vec![],
            assumptions: vec![],
            needs_design: false,
            ui_files: vec![],
            files_touched: vec![],
        };
        let finished = plan_view(vec![
            started.clone(),
            (step, Event::UnderstandResult { result }),
            (step, Event::StepFinished),
        ]);
        assert_eq!(states(&finished, Step::Understand), vec![SubState::Done; 4]);
        let texts_after: Vec<String> = sub_steps(&finished, Step::Understand)
            .into_iter()
            .map(|item| item.text)
            .collect();
        assert_eq!(texts_before, texts_after);

        let stopped = plan_view(vec![
            started,
            (
                step,
                Event::Failed {
                    reason: "the agent stopped after 20 turns".into(),
                },
            ),
        ]);
        assert_eq!(states(&stopped, Step::Understand)[0], SubState::Done);
        assert_eq!(
            states(&stopped, Step::Understand)[1],
            SubState::Stopped("the agent stopped after 20 turns".into())
        );

        let skipped = plan_view(vec![(
            Some(Step::Clarify),
            Event::StepSkipped {
                reason: "no open questions".into(),
            },
        )]);
        assert!(
            states(&skipped, Step::Clarify)
                .iter()
                .all(|state| *state == SubState::Skipped("no open questions".into()))
        );
    }

    fn said_for(step: Option<Step>, text: &str) -> (Option<Step>, autopilot_core::Event) {
        (
            step,
            autopilot_core::Event::AgentActivity {
                summary: text.into(),
            },
        )
    }
}
