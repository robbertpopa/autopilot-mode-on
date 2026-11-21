use autopilot_core::{
    Ambiguity, Clarity, Event, Money, NewEvent, RecordedEvent, RequestId, Step, TicketId,
    Understanding,
};

use super::*;

const U: Step = Step::Understand;

fn ev(step: Option<Step>, kind: Event) -> NewEvent {
    NewEvent {
        at_ms: 0,
        step,
        kind,
    }
}

fn created() -> NewEvent {
    ev(None, Event::TicketCreated { title: "t".into() })
}

fn started(step: Step) -> NewEvent {
    ev(
        Some(step),
        Event::StepStarted {
            attempt: "a".into(),
            session: None,
        },
    )
}

fn state_after(events: &[NewEvent]) -> Result<TicketState, TransitionError> {
    let mut state = TicketState::default();
    for event in events {
        state.apply(event)?;
    }
    Ok(state)
}

fn running() -> Vec<NewEvent> {
    vec![created(), started(U)]
}

fn plus(mut events: Vec<NewEvent>, more: Vec<NewEvent>) -> Vec<NewEvent> {
    events.extend(more);
    events
}

fn result_event() -> NewEvent {
    ev(
        Some(U),
        Event::UnderstandResult {
            result: Understanding {
                summary: "s".into(),
                ambiguity: Ambiguity::all(Clarity::Clear),
                open_questions: vec![],
                assumptions: vec![],
                needs_design: false,
                ui_files: vec![],
                files_touched: vec![],
            },
        },
    )
}

#[test]
fn a_whole_run_ends_passed_and_done() {
    let events = plus(
        running(),
        vec![
            ev(
                Some(U),
                Event::AgentActivity {
                    summary: "x".into(),
                },
            ),
            result_event(),
            ev(Some(U), Event::StepFinished),
            ev(None, Event::Done),
        ],
    );
    let state = state_after(&events).unwrap();
    assert_eq!(state.step(U), &StepState::Passed);
    assert!(state.is_done() && state.active().is_none());
    assert_eq!(state.step(Step::Plan), &StepState::Idle);
}

#[test]
fn asking_waits_for_the_matching_answer() {
    let asked = |id: &str| {
        ev(
            Some(U),
            Event::Asked {
                request: RequestId::from(id),
            },
        )
    };
    let answered = |id: &str| {
        ev(
            Some(U),
            Event::Answered {
                request: RequestId::from(id),
                answer: "yes".into(),
            },
        )
    };
    let waiting = state_after(&plus(running(), vec![asked("r1")])).unwrap();
    assert_eq!(waiting.step(U), &StepState::Asking(RequestId::from("r1")));
    assert_eq!(waiting.active(), Some(U));

    let wrong = plus(running(), vec![asked("r1"), answered("r2")]);
    assert_eq!(state_after(&wrong), Err(TransitionError::WrongRequest));
    let resumed = state_after(&plus(running(), vec![asked("r1"), answered("r1")])).unwrap();
    assert_eq!(resumed.step(U), &StepState::Running);
    // An answer with no question is refused.
    assert!(state_after(&plus(running(), vec![answered("r1")])).is_err());
}

// Invariant: a rejected event changes nothing. If apply mutated before
// checking, a bad event in a replay would leave a half-built state.
#[test]
fn a_rejected_event_leaves_the_state_unchanged() {
    let mut state = state_after(&running()).unwrap();
    let before = state.clone();
    assert!(state.apply(&ev(Some(U), Event::StepFinished)).is_ok());
    let after_finish = state.clone();
    assert!(state.apply(&ev(Some(U), Event::StepFinished)).is_err());
    assert_eq!(state, after_finish);
    assert_ne!(before, after_finish);
}

// Invariant: one step holds the ticket at a time.
#[test]
fn only_one_step_may_be_active() {
    let busy = plus(running(), vec![started(Step::Plan)]);
    assert_eq!(
        state_after(&busy),
        Err(TransitionError::StepBusy { active: U })
    );
    let free = plus(
        running(),
        vec![ev(Some(U), Event::StepFinished), started(Step::Plan)],
    );
    assert_eq!(state_after(&free).unwrap().active(), Some(Step::Plan));
}

#[test]
fn failed_and_interrupted_steps_can_retry_but_passed_and_skipped_cannot() {
    for end in [
        ev(Some(U), Event::Failed { reason: "x".into() }),
        ev(Some(U), Event::Interrupted { reason: "y".into() }),
    ] {
        let retried = plus(running(), vec![end, started(U)]);
        assert_eq!(state_after(&retried).unwrap().step(U), &StepState::Running);
    }
    let passed = plus(
        running(),
        vec![ev(Some(U), Event::StepFinished), started(U)],
    );
    assert_eq!(
        state_after(&passed),
        Err(TransitionError::NotRestartable(U))
    );
    let skipped = vec![
        created(),
        ev(
            Some(Step::Clarify),
            Event::StepSkipped {
                reason: "no open questions".into(),
            },
        ),
        started(Step::Clarify),
    ];
    assert_eq!(
        state_after(&skipped),
        Err(TransitionError::NotRestartable(Step::Clarify))
    );
}

#[test]
fn skipping_keeps_its_reason_and_only_applies_to_an_idle_step() {
    let skip = |step| {
        ev(
            Some(step),
            Event::StepSkipped {
                reason: "no UI change".into(),
            },
        )
    };
    let state = state_after(&[created(), skip(Step::Design)]).unwrap();
    assert_eq!(
        state.step(Step::Design),
        &StepState::Skipped("no UI change".into())
    );
    assert_eq!(state.active(), None);
    assert_eq!(
        state_after(&plus(running(), vec![skip(U)])),
        Err(TransitionError::NotIdle(U))
    );
}

#[test]
fn pause_and_resume_only_apply_to_a_running_step() {
    let paused = state_after(&plus(running(), vec![ev(Some(U), Event::Paused)])).unwrap();
    assert_eq!(paused.step(U), &StepState::Paused);
    // A paused step still holds the ticket.
    assert_eq!(paused.active(), Some(U));
    let back = plus(
        running(),
        vec![ev(Some(U), Event::Paused), ev(Some(U), Event::Resumed)],
    );
    assert_eq!(state_after(&back).unwrap().step(U), &StepState::Running);
    assert!(state_after(&plus(running(), vec![ev(Some(U), Event::Resumed)])).is_err());
}

#[test]
fn activity_is_refused_outside_a_running_step() {
    let activity = ev(
        Some(U),
        Event::AgentActivity {
            summary: "late".into(),
        },
    );
    assert!(state_after(&[created(), activity.clone()]).is_err());
    let finished = plus(running(), vec![ev(Some(U), Event::StepFinished), activity]);
    assert!(state_after(&finished).is_err());
}

#[test]
fn cost_adds_up_and_remembers_whether_any_part_was_an_estimate() {
    let tick = |micro, estimated| {
        ev(
            None,
            Event::CostTick {
                amount: Money::from_microusd(micro),
                estimated,
            },
        )
    };
    let state = state_after(&[created(), tick(100, false), tick(50, true)]).unwrap();
    assert_eq!(state.cost(), (Money::from_microusd(150), true));
    let exact = state_after(&[created(), tick(7, false)]).unwrap();
    assert!(!exact.cost().1);
    let overflow = state_after(&[created(), tick(u64::MAX, false), tick(1, false)]);
    assert_eq!(overflow, Err(TransitionError::CostOverflow));
}

#[test]
fn ordering_rules_for_creation_and_done() {
    assert_eq!(
        state_after(&[started(U)]),
        Err(TransitionError::BeforeCreation)
    );
    assert_eq!(
        state_after(&[created(), created()]),
        Err(TransitionError::AlreadyCreated)
    );
    assert_eq!(
        state_after(&plus(running(), vec![ev(None, Event::Done)])),
        Err(TransitionError::StepBusy { active: U })
    );
    let done = [created(), ev(None, Event::Done), ev(None, Event::Paused)];
    assert_eq!(state_after(&done), Err(TransitionError::AfterDone));
    assert_eq!(
        state_after(&[created(), ev(None, Event::StepFinished)]),
        Err(TransitionError::MissingStep)
    );
}

// Invariant: every prefix of a valid history is itself valid and replaying is
// deterministic. A machine that only worked on whole histories could not
// resume a half-written run.
#[test]
fn every_prefix_of_a_valid_history_folds_and_replays_identically() {
    let events = plus(
        running(),
        vec![
            ev(
                Some(U),
                Event::AgentActivity {
                    summary: "a".into(),
                },
            ),
            ev(
                Some(U),
                Event::Asked {
                    request: RequestId::from("r"),
                },
            ),
            ev(
                Some(U),
                Event::Answered {
                    request: RequestId::from("r"),
                    answer: "ok".into(),
                },
            ),
            result_event(),
            ev(Some(U), Event::StepFinished),
            ev(None, Event::Done),
        ],
    );
    let records: Vec<RecordedEvent> = events
        .into_iter()
        .enumerate()
        .map(|(i, event)| RecordedEvent {
            ticket: TicketId::from("t"),
            seq: i as u64 + 1,
            event,
        })
        .collect();
    for end in 0..=records.len() {
        let first = fold(&records[..end]).unwrap();
        assert_eq!(first, fold(&records[..end]).unwrap());
    }
    let mut broken = records.clone();
    broken.swap(3, 4);
    let error = fold(&broken).unwrap_err();
    assert!(error.seq >= 4, "{error}");
}

fn reset(step: Step, and_after: bool) -> NewEvent {
    ev(Some(step), Event::StepReset { and_after })
}

fn finished(step: Step) -> NewEvent {
    ev(Some(step), Event::StepFinished)
}

// Invariant: a settled step can be run again only through an explicit reset,
// the reset reaches exactly the steps it names, and the ticket keeps what it
// already spent. A change that let a Passed step restart on its own, or that
// reset earlier steps, would break the history's meaning.
#[test]
fn a_reset_sends_a_settled_step_and_optionally_the_later_ones_back_to_idle() {
    let history = vec![
        created(),
        started(U),
        ev(
            Some(U),
            Event::CostTick {
                amount: Money::from_microusd(5),
                estimated: false,
            },
        ),
        finished(U),
        ev(
            Some(Step::Clarify),
            Event::StepSkipped {
                reason: "nothing to ask".into(),
            },
        ),
        started(Step::Plan),
        finished(Step::Plan),
    ];

    let only_plan = state_after(&plus(history.clone(), vec![reset(Step::Plan, false)])).unwrap();
    assert_eq!(only_plan.step(Step::Plan), &StepState::Idle);
    assert_eq!(only_plan.step(U), &StepState::Passed);

    let from_understand = state_after(&plus(history.clone(), vec![reset(U, true)])).unwrap();
    assert_eq!(from_understand.step(U), &StepState::Idle);
    assert_eq!(from_understand.step(Step::Clarify), &StepState::Idle);
    assert_eq!(from_understand.step(Step::Plan), &StepState::Idle);
    assert_eq!(from_understand.cost().0, Money::from_microusd(5));

    let again = plus(history, vec![reset(Step::Plan, false), started(Step::Plan)]);
    assert_eq!(
        state_after(&again).unwrap().step(Step::Plan),
        &StepState::Running
    );
}

// Invariant: nothing resets while a step holds the ticket, and a step that
// never ran has nothing to reset.
#[test]
fn a_reset_is_refused_while_a_step_is_active_or_when_nothing_ran() {
    assert_eq!(
        state_after(&plus(running(), vec![reset(U, false)])),
        Err(TransitionError::StepBusy { active: U })
    );
    let settled = plus(running(), vec![finished(U)]);
    assert_eq!(
        state_after(&plus(settled, vec![reset(Step::Plan, false)])),
        Err(TransitionError::NothingToReset(Step::Plan))
    );
    assert_eq!(
        state_after(&[created(), reset(U, false)]),
        Err(TransitionError::NothingToReset(U))
    );
}
