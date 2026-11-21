use autopilot_core::Money;
use autopilot_profile::{Profile, WhenUnsure};

use super::*;

fn policy(toml: &str) -> Policy {
    let profile = Profile::parse("profile.toml", toml).unwrap();
    Policy::new(&profile).unwrap()
}

fn project() -> Policy {
    policy(
        r#"
        protected_paths = ["secrets/**", "*.pem"]
        [budgets]
        per_ticket_microusd = 1000
        daily_microusd = 5000
        "#,
    )
}

fn facts<'a>(touched: &'a [String], labels: &'a [String]) -> Facts<'a> {
    Facts {
        touched,
        risk: Risk::Standard,
        labels,
        ticket_spent: Money::from_microusd(0),
        day_spent: Money::from_microusd(0),
        signals: DiffSignals::default(),
    }
}

fn paths(list: &[&str]) -> Vec<String> {
    list.iter().map(|p| (*p).to_owned()).collect()
}

/// One row per forced-confirmation rule: how to trigger it and the reason it
/// must report. Plain function pointers, since no row captures anything.
struct Rule {
    name: &'static str,
    set: fn(&mut Facts<'_>),
    reports: fn(&Reason) -> bool,
}

fn rules() -> Vec<Rule> {
    vec![
        Rule {
            name: "risky tier",
            set: |f| f.risk = Risk::Risky,
            reports: |r| *r == Reason::RiskyTier,
        },
        Rule {
            name: "ticket budget",
            set: |f| f.ticket_spent = Money::from_microusd(1001),
            reports: |r| *r == Reason::TicketBudgetExceeded,
        },
        Rule {
            name: "day budget",
            set: |f| f.day_spent = Money::from_microusd(5001),
            reports: |r| *r == Reason::DayBudgetExceeded,
        },
        Rule {
            name: "new network calls",
            set: |f| f.signals.new_network_calls = true,
            reports: |r| *r == Reason::NewNetworkCalls,
        },
        Rule {
            name: "new dependencies",
            set: |f| f.signals.new_dependencies = true,
            reports: |r| *r == Reason::NewDependencies,
        },
        Rule {
            name: "changed signing",
            set: |f| f.signals.changed_signing = true,
            reports: |r| *r == Reason::ChangedSigning,
        },
    ]
}

// Invariant: every rule forces a question with the checkbox off. If a rule
// were dropped or only applied when the box is on, a protected change could
// pass silently, and no other test would notice.
#[test]
fn every_rule_forces_a_question_even_with_the_checkbox_off() {
    let project = project();
    for rule in rules() {
        let mut f = facts(&[], &[]);
        (rule.set)(&mut f);
        let decision = project.evaluate(false, &f);
        assert!(decision.must_ask(), "{}", rule.name);
        assert!(decision.forced.iter().any(rule.reports), "{}", rule.name);
    }
}

#[test]
fn path_rules_force_a_question_with_the_checkbox_off() {
    let project = project();
    let cases = [
        ("secrets/key.txt", "protected path"),
        ("deploy/server.pem", "protected path"),
        (".github/workflows/ci.yml", "workflow file"),
        ("Cargo.lock", "lockfile"),
        ("web/package-lock.json", "lockfile"),
        ("scripts/deploy.sh", "script"),
        ("tools/run.ps1", "script"),
        ("Makefile", "script"),
        ("../outside.txt", "outside the repository"),
        ("C:\\Windows\\x", "outside the repository"),
        ("/etc/passwd", "outside the repository"),
    ];
    for (path, expected) in cases {
        let touched = paths(&[path]);
        let decision = project.evaluate(false, &facts(&touched, &[]));
        assert!(decision.must_ask(), "{path}");
        let text = decision.asks_anyway().unwrap();
        assert!(text.contains(expected), "{path}: {text}");
    }
}

#[test]
fn security_labels_force_a_question() {
    let project = project();
    for label in [
        "security",
        "Security",
        "security:auth",
        "cve-2026-1",
        "vulnerability",
    ] {
        let labels = paths(&[label]);
        assert!(
            project.evaluate(false, &facts(&[], &labels)).must_ask(),
            "{label}"
        );
    }
    for label in ["bug", "insecure-looking", "security-ish-no"] {
        let labels = paths(&[label]);
        let decision = project.evaluate(false, &facts(&[], &labels));
        // "security-ish-no" starts with the word and a dash, which counts: it
        // is better to ask once too often than to miss a label.
        assert_eq!(decision.must_ask(), label == "security-ish-no", "{label}");
    }
}

// Invariant: the checkbox only adds a question. For every scenario the forced
// reasons are identical with it on or off, and a forced step asks either way.
#[test]
fn the_checkbox_can_add_a_question_but_never_remove_a_forced_one() {
    let project = project();
    let touched = paths(&["secrets/a", "src/main.rs"]);
    let labels = paths(&["security"]);
    let mut scenarios = vec![facts(&[], &[]), facts(&touched, &labels)];
    for rule in rules() {
        let mut f = facts(&[], &[]);
        (rule.set)(&mut f);
        scenarios.push(f);
    }
    for f in &scenarios {
        let off = project.evaluate(false, f);
        let on = project.evaluate(true, f);
        assert_eq!(off.forced, on.forced);
        assert!(on.must_ask());
        assert_eq!(off.must_ask(), !off.forced.is_empty());
    }
}

#[test]
fn ordinary_work_does_not_force_a_question() {
    let project = project();
    let touched = paths(&["src/main.rs", "docs/readme.md", "crates/core/src/lib.rs"]);
    let labels = paths(&["bug", "frontend"]);
    let mut f = facts(&touched, &labels);
    // Spending exactly the limit is not exceeding it.
    f.ticket_spent = Money::from_microusd(1000);
    f.day_spent = Money::from_microusd(5000);
    let decision = project.evaluate(false, &f);
    assert!(decision.forced.is_empty() && !decision.must_ask());
    assert_eq!(decision.asks_anyway(), None);
    assert!(project.evaluate(true, &f).must_ask());
}

#[test]
fn a_project_without_budgets_never_reports_a_budget() {
    let open = policy("");
    let mut f = facts(&[], &[]);
    f.ticket_spent = Money::from_microusd(u64::MAX);
    f.day_spent = Money::from_microusd(u64::MAX);
    assert_eq!(open.evaluate(false, &f).forced.len(), 0);
}

// Invariant: Windows spellings of one path are one path. A protected file
// reached as `.GitHub\Workflows\ci.yml` must still be protected.
#[test]
fn spelling_tricks_do_not_hide_a_protected_path() {
    let project = project();
    for path in [
        ".GitHub\\Workflows\\ci.yml",
        "./Cargo.lock",
        "CARGO.LOCK",
        "src//..//x",
    ] {
        let touched = paths(&[path]);
        assert!(
            project.evaluate(false, &facts(&touched, &[])).must_ask(),
            "{path}"
        );
    }
}

#[test]
fn the_message_names_every_reason() {
    let project = project();
    let touched = paths(&["secrets/key.pem"]);
    let mut f = facts(&touched, &[]);
    f.risk = Risk::Risky;
    let text = project.evaluate(false, &f).asks_anyway().unwrap();
    assert_eq!(
        text,
        "asks anyway: protected path secrets/key.pem, risky ticket"
    );
}

#[test]
fn an_invalid_pattern_is_reported_with_its_text() {
    let profile = Profile::parse("profile.toml", "protected_paths = [\"[unclosed\"]").unwrap();
    let error = Policy::new(&profile).err().unwrap();
    assert!(error.to_string().contains("[unclosed"));
}

// Invariant: "decide and tell me" keeps the assumption so nothing is hidden,
// and it is a different question from the forced rules: evaluate() does not
// take the setting, so it cannot silence them.
#[test]
fn deciding_for_the_owner_still_records_the_assumption() {
    assert_eq!(when_unsure(WhenUnsure::Ask, "CSV"), Unsure::Ask);
    assert_eq!(
        when_unsure(WhenUnsure::DecideAndTell, "assume UTF-8"),
        Unsure::Decide {
            assumption: "assume UTF-8".into()
        }
    );
    let touched = paths(&["secrets/x"]);
    assert!(project().evaluate(false, &facts(&touched, &[])).must_ask());
}

// Invariant: the host pushes only a ticket's own branch. A protected branch
// is refused under any spelling, and so is anything that could be read as a
// force, a rename or a delete.
#[test]
fn only_plain_ticket_branches_may_be_pushed() {
    let policy = policy("branch_rules.protected = [\"main\", \"release/*\", \"ticket/frozen\"]");
    for ok in [
        "ticket/gh-12",
        "refs/heads/ticket/42",
        "ticket/a/b",
        "ticket/-x",
    ] {
        assert_eq!(policy.check_push(ok), Ok(()), "{ok}");
    }
    for protected in [
        "main",
        "MAIN",
        "refs/heads/main",
        "release/1.0",
        "ticket/frozen",
    ] {
        assert!(
            matches!(
                policy.check_push(protected),
                Err(PushRefusal::ProtectedBranch(_))
            ),
            "{protected}"
        );
    }
    for odd in [
        "",
        "develop",
        "ticket/",
        "ticket//x",
        "ticket/../main",
        "+ticket/x",
        "ticket/x:main",
        ":ticket/x",
        "ticket/x y",
        "HEAD",
        "refs/tags/ticket/x",
    ] {
        assert!(
            matches!(
                policy.check_push(odd),
                Err(PushRefusal::NotATicketBranch(_))
            ),
            "{odd:?}"
        );
    }
}

// Invariant: the default protected branches are refused with no profile at all.
#[test]
fn the_default_profile_protects_the_usual_branches() {
    let policy = policy("");
    for name in ["main", "master", "develop"] {
        assert!(policy.check_push(name).is_err(), "{name}");
    }
    assert!(policy.check_push("ticket/1").is_ok());
}
