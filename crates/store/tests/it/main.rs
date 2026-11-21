use std::{
    env,
    process::{self, Command},
};

use autopilot_core::{Event, Money, NewEvent, Step, TicketId};
use autopilot_store::{SqliteStore, Store, StoreError, TicketState};
use rusqlite::Connection;

const CRASH_EXIT: i32 = 17;

fn event(kind: Event) -> NewEvent {
    NewEvent {
        at_ms: 42,
        step: None,
        kind,
    }
}

fn create() -> NewEvent {
    event(Event::TicketCreated {
        title: "Example".into(),
    })
}

#[test]
fn a_stale_writer_cannot_append_another_event_at_the_same_sequence() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("events.sqlite3");
    let ticket = TicketId::from("CK-1");
    let mut first = SqliteStore::open(&database).unwrap();
    let mut second = SqliteStore::open(&database).unwrap();

    let written = first.append(&ticket, 0, &[create()]).unwrap();
    assert!(matches!(
        second.append(&ticket, 0, &[event(Event::Paused)]),
        Err(StoreError::SequenceConflict {
            expected: 0,
            actual: 1
        })
    ));
    drop((first, second));

    let reopened = SqliteStore::open(database).unwrap();
    assert_eq!(reopened.events(&ticket, 0).unwrap(), written);
}

#[test]
fn a_failed_batch_does_not_leave_a_partial_history() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("events.sqlite3");
    let ticket = TicketId::from("CK-2");
    let mut store = SqliteStore::open(&database).unwrap();
    let original = store.append(&ticket, 0, &[create()]).unwrap();
    Connection::open(&database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_batch BEFORE INSERT ON events WHEN NEW.seq = 3
             BEGIN SELECT RAISE(ABORT, 'simulated write failure'); END;",
        )
        .unwrap();

    assert!(
        store
            .append(&ticket, 1, &[event(Event::Paused), event(Event::Resumed)])
            .is_err()
    );
    drop(store);
    let reopened = SqliteStore::open(database).unwrap();
    assert_eq!(reopened.events(&ticket, 0).unwrap(), original);
}

#[test]
fn reopening_after_abrupt_exit_keeps_only_committed_events() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("events.sqlite3");
    let ticket = TicketId::from("crash-probe");
    let mut store = SqliteStore::open(&database).unwrap();
    let committed = store.append(&ticket, 0, &[create()]).unwrap();
    drop(store);

    let status = Command::new(env::current_exe().unwrap())
        .args(["--ignored", "--exact", "abrupt_writer"])
        .env("AUTOPILOT_STORE_CRASH_PROBE_DB", &database)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(CRASH_EXIT));
    let reopened = SqliteStore::open(database).unwrap();
    assert_eq!(reopened.events(&ticket, 0).unwrap(), committed);
}

#[test]
#[ignore = "run as a child of reopening_after_abrupt_exit_keeps_only_committed_events"]
fn abrupt_writer() {
    let Some(database) = env::var_os("AUTOPILOT_STORE_CRASH_PROBE_DB") else {
        return;
    };
    let mut connection = Connection::open(database).unwrap();
    connection.pragma_update(None, "cache_size", 1).unwrap();
    let tx = connection.transaction().unwrap();
    let before_crash = event(Event::AgentActivity {
        summary: "x".repeat(256 * 1024),
    });
    tx.execute(
        "INSERT INTO events (ticket_id, seq, body) VALUES ('crash-probe', 2, ?1)",
        [serde_json::to_string(&before_crash).unwrap()],
    )
    .unwrap();
    // Exit with a dirty transaction so SQLite's recovery runs on reopen.
    process::exit(CRASH_EXIT);
}

#[test]
fn a_step_event_missing_its_step_cannot_poison_the_history() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = TicketId::from("CK-4");
    let mut store = SqliteStore::open(temp.path().join("events.sqlite3")).unwrap();

    assert!(matches!(
        store.append(&ticket, 0, &[create(), event(Event::StepFinished)]),
        Err(StoreError::MissingStep("step_finished"))
    ));
    assert_eq!(store.events(&ticket, 0).unwrap(), []);
}

#[test]
fn a_cost_total_larger_than_one_event_amount_can_replay_and_reload() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = TicketId::from("CK-5");
    let mut store = SqliteStore::open(temp.path().join("events.sqlite3")).unwrap();
    let cost = |value| {
        event(Event::CostTick {
            amount: Money::from_microusd(value),
            estimated: false,
        })
    };
    let events = store
        .append(&ticket, 0, &[create(), cost(u64::MAX), cost(1)])
        .unwrap();

    let state = autopilot_store::fold(ticket.clone(), &events).unwrap();
    assert_eq!(state.reported_cost_microusd(), u128::from(u64::MAX) + 1);
    store.save_snapshot(&state).unwrap();
    assert_eq!(store.load_snapshot(&ticket).unwrap(), Some(state));
}

#[test]
fn a_snapshot_and_its_tail_match_full_replay_after_reopening() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("events.sqlite3");
    let ticket = TicketId::from("CK-3");
    let mut store = SqliteStore::open(&database).unwrap();

    let mut prefix = TicketState::new(ticket.clone());
    let skipped = NewEvent {
        at_ms: 43,
        step: Some(Step::Clarify),
        kind: Event::StepSkipped {
            reason: "No open questions".into(),
        },
    };
    let first = store.append(&ticket, 0, &[create(), skipped]).unwrap();
    prefix.replay(&first).unwrap();
    store.save_snapshot(&prefix).unwrap();
    store.append(&ticket, 2, &[event(Event::Done)]).unwrap();
    drop(store);

    let reopened = SqliteStore::open(database).unwrap();
    let mut cached = reopened.load_snapshot(&ticket).unwrap().unwrap();
    let mut complete = TicketState::new(ticket.clone());
    cached
        .replay(&reopened.events(&ticket, cached.last_seq()).unwrap())
        .unwrap();
    complete
        .replay(&reopened.events(&ticket, 0).unwrap())
        .unwrap();
    assert_eq!(cached, complete);
    assert_eq!(cached.last_seq(), 3);
}

// Invariant: after a reset the stored summary of the ticket forgets the step's
// old result and no longer reads as failed, while spend stays. If it kept the
// failure, a re-run ticket would show as failed until its next event.
#[test]
fn a_reset_clears_the_step_and_the_failed_status_but_not_the_cost() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = TicketId::from("CK-6");
    let mut store = SqliteStore::open(temp.path().join("events.sqlite3")).unwrap();
    let on = |step, kind| NewEvent {
        at_ms: 1,
        step: Some(step),
        kind,
    };
    let events = store
        .append(
            &ticket,
            0,
            &[
                create(),
                on(
                    Step::Understand,
                    Event::StepStarted {
                        attempt: "a".into(),
                        session: None,
                    },
                ),
                event(Event::CostTick {
                    amount: Money::from_microusd(7),
                    estimated: false,
                }),
                on(
                    Step::Understand,
                    Event::Failed {
                        reason: "boom".into(),
                    },
                ),
                on(Step::Understand, Event::StepReset { and_after: false }),
            ],
        )
        .unwrap();
    let state = autopilot_store::fold(ticket, &events).unwrap();
    assert_eq!(state.step(Step::Understand), None);
    assert_eq!(*state.status(), autopilot_store::TicketStatus::Idle);
    assert_eq!(state.reported_cost_microusd(), 7);
}
