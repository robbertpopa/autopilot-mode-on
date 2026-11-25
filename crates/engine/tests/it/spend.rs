#![allow(clippy::unwrap_used)]
//! The total spending cap: a paid call is refused once the exact spend in the
//! data directory reaches it, and is never allowed more than what is left.

use std::path::Path;

use autopilot_core::{Event, Money, NewEvent, Step, TicketId};
use autopilot_engine::{
    SpendError, SpendField, edit_spend, is_cap_refusal, load_profile, micro_from_dollars,
    paid_call_cap,
};
use autopilot_profile::SpendLimits;
use autopilot_store::{SqliteStore, Store};

fn limits(total: u64, call: u64) -> SpendLimits {
    let text = format!("[spend]\ntotal_microusd = {total}\ncall_microusd = {call}\n");
    autopilot_profile::Profile::parse("profile.toml", &text)
        .unwrap()
        .spend
}

/// One ticket per cost, so the cap counts across tickets.
fn spend(directory: &Path, ticket: &str, micro: u64, estimated: bool) {
    let mut store = SqliteStore::open(directory.join("events.sqlite3")).unwrap();
    let id = TicketId::from(ticket);
    let events = [
        NewEvent {
            at_ms: 1,
            step: None,
            kind: Event::TicketCreated {
                title: ticket.into(),
            },
        },
        NewEvent {
            at_ms: 2,
            step: Some(Step::Understand),
            kind: Event::CostTick {
                amount: Money::from_microusd(micro),
                estimated,
            },
        },
    ];
    store.append(&id, 0, &events).unwrap();
}

// Invariant: with nothing spent, a call gets its own cap, and the profile's
// default when its step sets none. Breaks if the default is dropped or the step
// budget is ignored.
#[test]
fn a_call_gets_its_step_budget_or_the_profile_default() {
    let directory = tempfile::tempdir().unwrap();
    let limits = limits(1_000_000, 50_000);
    let cap = paid_call_cap(directory.path(), &limits, None).unwrap();
    assert_eq!(cap.as_microusd(), 50_000);
    let own = paid_call_cap(
        directory.path(),
        &limits,
        Some(Money::from_microusd(200_000)),
    )
    .unwrap();
    assert_eq!(own.as_microusd(), 200_000);
}

// Invariant: the call that would pass the cap is cut down to what is left, and
// once it is all spent (on any ticket) the call is refused with the words the
// window recognises. Breaks if the check counts one ticket only, uses `>`
// instead of `>=`, or does not clamp.
#[test]
fn the_cap_counts_every_ticket_and_stops_a_call_at_zero_left() {
    let directory = tempfile::tempdir().unwrap();
    let limits = limits(100_000, 50_000);
    spend(directory.path(), "a", 40_000, false);
    spend(directory.path(), "b", 30_000, false);
    let cap = paid_call_cap(directory.path(), &limits, None).unwrap();
    assert_eq!(cap.as_microusd(), 30_000, "70k of 100k spent, 30k left");
    spend(directory.path(), "c", 30_000, false);
    let refused = paid_call_cap(directory.path(), &limits, None).unwrap_err();
    assert!(matches!(refused, SpendError::CapReached { .. }));
    assert!(is_cap_refusal(&refused.to_string()));
}

// Invariant: a subscription estimate is not money spent, so it never uses up
// the cap. Breaks if the estimated flag is ignored.
#[test]
fn an_estimated_subscription_cost_does_not_count() {
    let directory = tempfile::tempdir().unwrap();
    spend(directory.path(), "a", 5_000_000, true);
    assert!(paid_call_cap(directory.path(), &limits(100_000, 50_000), None).is_ok());
}

// Invariant: raising the cap in Settings lets the stopped call go again, and
// the file keeps the owner's other text. Breaks if edit_spend writes nothing
// or rewrites the file.
#[test]
fn raising_the_cap_in_the_profile_lets_a_stopped_call_through() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.toml");
    std::fs::write(&path, "# mine\nprotected_paths = [\"infra/**\"]\n").unwrap();
    spend(directory.path(), "a", 1_000_000, false);
    let profile = load_profile(&path).unwrap();
    assert!(paid_call_cap(directory.path(), &profile.spend, None).is_err());

    let profile = edit_spend(&path, SpendField::Total(2_000_000)).unwrap();
    assert!(paid_call_cap(directory.path(), &profile.spend, None).is_ok());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# mine") && text.contains("infra/**"));
}

// Invariant: a zero or unreadable limit is refused and the file is untouched.
#[test]
fn a_zero_limit_is_refused_and_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.toml");
    assert!(edit_spend(&path, SpendField::Total(0)).is_err());
    assert!(!path.exists());
    assert_eq!(micro_from_dollars("0.05"), Some(50_000));
    assert_eq!(micro_from_dollars("1"), Some(1_000_000));
    assert_eq!(micro_from_dollars(".5"), Some(500_000));
    for bad in ["", ".", "-1", "1e3", "0.0000001", "$1", "1,5"] {
        assert_eq!(micro_from_dollars(bad), None, "{bad}");
    }
}
