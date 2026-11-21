//! The machine against history written by the real engine.
#![allow(clippy::unwrap_used)] // helpers outside #[test] functions

use autopilot_agent::{Agent, AgentError, AgentRun, StepRequest, Stopped};
use autopilot_core::{Ambiguity, Clarity, Money, Step, Ticket, TicketId, Understanding};
use autopilot_pipeline::{StepState, fold};

struct Scripted(&'static str);

impl Agent for Scripted {
    fn start(&self, _: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        let how = self.0;
        Ok(AgentRun::spawn(move |context| {
            context.message("working".into());
            context.cost(Money::from_microusd(900), true);
            match how {
                "ok" => Ok(Understanding {
                    summary: "s".into(),
                    ambiguity: Ambiguity::all(Clarity::Clear),
                    open_questions: vec![],
                    assumptions: vec![],
                    needs_design: false,
                    ui_files: vec![],
                    files_touched: vec![],
                }),
                "stop" => Err(Stopped("error_max_turns".into()).into()),
                _ => Err(std::io::Error::other("boom").into()),
            }
        })?)
    }
}

fn history(how: &'static str) -> autopilot_pipeline::TicketState {
    let data = tempfile::tempdir().unwrap();
    let ticket = Ticket {
        id: TicketId::from("pipe-1"),
        title: "t".into(),
        description: "d".into(),
    };
    let _ = autopilot_engine::run_understand(data.path(), &ticket, &Scripted(how));
    let events = autopilot_engine::watch(data.path(), &ticket.id, 0).unwrap();
    fold(&events).unwrap()
}

// Invariant: whatever the engine records for a good run, a limit stop and a
// failure folds without error into the matching state, with the cost kept.
#[test]
fn engine_histories_fold_to_passed_interrupted_and_failed() {
    let passed = history("ok");
    assert_eq!(passed.step(Step::Understand), &StepState::Passed);
    assert!(passed.is_done());
    assert_eq!(passed.cost(), (Money::from_microusd(900), true));

    let stopped = history("stop");
    assert_eq!(
        stopped.step(Step::Understand),
        &StepState::Interrupted("error_max_turns".into())
    );
    assert!(!stopped.is_done());

    assert!(matches!(
        history("fail").step(Step::Understand),
        StepState::Failed(_)
    ));
}
