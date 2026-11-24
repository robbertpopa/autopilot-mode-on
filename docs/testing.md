<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Testing standard

Few tests, each worth keeping. The suite must stay small enough to read.

## The keep rule

A test stays only if you can write all three lines:
1. **Invariant:** what it guards, in one line.
2. **Breaking change:** a plausible change someone could make while fixing, refactoring or extending this code (a reordered step, an off-by-one, a dropped case, a changed default). Deleting the function does not count.
3. **Only this test:** no other test would fail on that change. Search for the function before adding.

If you cannot write all three, do not add the test. The exception is a regression test for safety (secrets, protected paths, process control): keep it until someone has checked the three lines.

## Do not write tests for

- Code a reader can verify (a few lines, one branch, no state).
- Wiring: that an input reaches a function, a button runs its command.
- Defaults, constants, getters, table entries with no logic.
- What the compiler already guarantees; behaviour of a dependency.
- Duplicates of another test; several values through the same branch (keep one per distinct branch).
- Running code with no assertion a regression would change.

## Fix when you see

- Exact values we chose ourselves: assert the relation (inside, longer than, changed).
- Wall-clock time or sleeps: inject the clock and assert what happened, not how long.
- Platform-fragile paths and line endings.
- Unbounded loops over generated cases: use a small, shared case budget.
- A unit test over 0.1 s or an integration test over 1 s without a reason.

## Where tests go

- One integration binary per crate: `tests/it/main.rs` with files as modules.
- Test a shared mechanism once, where it is defined; test behaviour through the component the user meets.
- Names are sentences about the invariant: `a_secret_in_the_environment_never_reaches_a_subscription_child`.
- A bug fix comes with a test that fails without it.

## Specific to this project

- **No real agent in normal tests.** `tools/fake-agent` replays recorded transcripts from `fixtures/` and injects faults: truncated line, partial UTF-8, huge output, hang, crash, rate limit, `error_max_turns`.
- **Live tests** are `#[ignore]` and behind the `live` feature, capped at $0.05 each, run by hand, and re-run when Claude Code or Codex updates. Re-record fixtures if the contract changed.
- **Hostile input** for every parser (stream-json, profile, inbox files, structured output): truncation, bit flips, unknown types and fields. Never a panic.
- **Determinism:** `fold(events)` is the state; replay after a crash gives the same state; the clock is injected.
- **Policy tables:** every forced-confirmation rule, and that the checkbox can never loosen a rule.
- **Windows is a first-class CI job**, next to Linux and macOS (`ci.yml` runs all three). Process control, paths and file locks are where the risk is. Use `cargo nextest run --workspace`; plain `cargo test` shares one process and some window tests are flaky under it.
- **One conformance suite for every agent adapter.** `agent::conformance` holds the rules a run must obey (one result, ordered events, unknown fields ignored, prompt cancel, cost never backwards). `agent-claude` and `agent-fixture` run the same suite, and a deliberately broken adapter fails each rule.
- **The sandbox probe needs the real boundary.** Tests that start `srt` are `#[ignore]` (`the_real_boundary_passes_the_probe` in the WSL distro, `..._on_a_linux_host`, `..._on_a_mac`). Run them by hand with `cargo test -p autopilot-process -- --ignored <name>`; `sandbox-probe.yml` runs the Linux one in CI. A weakened boundary must fail the row it is meant to catch (the mutation tests).
- **The fake `gh`** (`tools/fake-gh`) keeps a JSON world of issues, pull requests, review threads and check runs and records every call, so tracker and `autopilot pr watch` tests never touch the network.
- **Cutting tests** happens in its own pull request per crate, listing every removed test with its reason.
