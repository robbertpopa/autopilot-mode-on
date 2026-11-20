<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# M0: foundation

**Owner crates:** `core`, `store`, `profile`, `tools/xtask`, repository config. **Depends on:** nothing. **Gate for:** everything.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M0-foundation.md`."

## Goal
A workspace that builds and passes CI on Windows and Linux, with the event log, the step and event types, and the profile format. No agent code yet.

## Tasks
- [x] Repository files: `LICENSE` (Apache-2.0), `.gitignore`, `rust-toolchain.toml` (stable, pinned), `rustfmt.toml`, `clippy.toml` (allow unwrap and expect in test functions), workspace `Cargo.toml` with `[workspace.lints]` (clippy pedantic, `unwrap_used`, `expect_used`, `print_stdout`, `dbg_macro`, `todo`; `unsafe_code = "deny"`), `deny.toml` (permissive licences only), `.coderabbit.yaml` (reviews PRs into `develop`, ignore Dependabot), release-please config.
- [x] CI: Windows and Linux jobs running fmt, clippy `-D warnings`, nextest, `cargo deny`, `cargo machete`, `cargo xtask layers`. A job that fails if `AGENTS.md` and `CLAUDE.md` drift.
- [x] `core`: ids (`TicketId`, `StepId`, `SessionId`), `Step` enum (understand, clarify, design, plan, build_check, pull_request), `Event` enum (see below), `Money` (integer micro-dollars), and typed errors for store and profile operations.
- [x] `Event` kinds: `TicketCreated`, `StepStarted`, `StepSkipped{reason}`, `AgentActivity`, `RoundStarted`, `CheckResult`, `Asked{request}`, `Answered{answer}`, `CostTick`, `SettingsOverridden{scope}`, `Paused`, `Resumed`, `Failed{reason}`, `Done`. Every event has `ticket`, `step?`, `at` (injected clock), `seq`.
- [x] `store`: `Store` trait; `SqliteStore` on `rusqlite` (bundled, WAL); `append`, `events(ticket, from_seq)`, `snapshot` and `load_snapshot`; a pure `fold(events) -> TicketState`. Migrations in code, versioned.
- [x] `profile`: structs with `serde` and `schemars`; per step `agent` (`claude`|`codex`), `auth` (`subscription`|`api`; default `subscription`), `model`, `effort`, limits (turns, budget, rounds), `ask_after` (bool); project-level `checks` (commands), `protected_paths`, `branch_rules`, `budgets`, `when_unsure` (`ask`|`decide_and_tell`). Defaults for every field; validation errors carry file, line and key.
- [x] `tools/xtask`: `layers` (production dependency direction, direct process spawning only in `process`, egui only in `ui`), `secrets` (scan the staged diff through Gitleaks), `ci` (run the clean-checkout checks). Add the `xtask` alias in `.cargo/config.toml`.
- [x] Seed `docs/pitfalls.md` entries as you learn them.

## Tests that must exist (each passes the keep rule)
- `fold` of the same events gives the same state in any chunking; a store reopened after a simulated crash mid-append gives the state before the last complete event.
- Invalid profiles report the file, source location and key for type and semantic errors.
- Omitted step settings resolve to defaults. A dollar budget on a subscription step is rejected by the parser and the generated schema.
- `xtask layers` fails on a deliberate violation (a test crate that depends upward).

## Exit criteria
`cargo xtask ci` passes against a clean source tree on Windows and Linux.

## Verification

On 2026-11-20, `cargo xtask ci` passed on a fresh Windows source copy and in a Linux Rust container. Nextest passed 11 tests on each platform. The abrupt-exit helper is launched by its parent test. `actionlint` also passed on the GitHub Actions workflow.
