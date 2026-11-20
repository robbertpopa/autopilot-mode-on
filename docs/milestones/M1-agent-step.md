<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M1: one agent step from the CLI

**Owner crates:** `process`, `agent`, `agent-claude`, `tools/fake-agent`, `engine`, `cli` (first commands). **Depends on:** M0. Live execution needs S1 and S5. Automatic limit classification needs S2. **Gate for:** M2 and M8.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M1-agent-step.md`."

## Goal
Demonstrate a replayable Understand run with fixtures, then run one synthetic ticket through the validated headless Claude Code setup on Windows.

## Tasks
- [ ] `agent` crate: `Agent` trait (`start(step_request) -> event stream + handle`), normalised `AgentEvent` (`Started`, `Message`, `Tool`, `FileChange`, `Usage`, `Retry`, `RateLimited`, `Result`, `Failed`) and provider-neutral result types. Use `crates/process` for bounded stdout and stderr, prompt on stdin, timeouts, cancellation and platform process-tree cleanup.
  - [x] Stream `Started`, `Message`, `Result` and `Failed` from fixture runs. Persist complete activity while the worker is alive and retain it if the run fails. The run handle requests cancellation if the consumer drops it.
- [ ] `agent-claude`: command builder (model, effort, max turns, budget, allowed and disallowed tools, `--settings` with deny hooks, `--session-id`, `--json-schema`, `--append-system-prompt`); stream-json parser from the S1 fixtures; `Result` subtypes; rate-limit and retry detection per S2; record `claude_code_version`.
- [ ] Per-step auth: build the child environment from scratch. Subscription: remove `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_*`. API: inject the dedicated key only into the provider environment verified for that step. Preflight with `claude auth status` (exit code and `authMethod`); a clear message if not signed in or if the method does not match the step's `auth`.
- [ ] Cost ledger: API steps use `total_cost_usd`; subscription steps estimate from token usage and a price table (from the pricing page, with cache read and write multipliers), always labelled "estimate".
- [x] `tools/fake-agent` replays the synthetic transcript and can emit `truncated`, `partial-utf8`, `oversized`, `hang` and `crash` variants via `replay <fault>`.
- [ ] Add replay cases for the observed Claude rate-limit and turn-cap outcomes after S1 and S2.
- [x] Save the fixture result through `engine`. Run `autopilot run-step --step understand --ticket <file> --agent fake` and `autopilot watch <ticket-id>` from separate processes. Add `--after <sequence>` to resume replay.
- [x] Reuse the S5 subprocess supervisor for the synthetic fixture adapter. The live agent contract and streaming integration remain above.
- [ ] A `live` feature test, `#[ignore]`, capped at $0.05.

## Tests that must exist
- Parser: every fixture parses; a hostile sweep (truncation, bit flips, unknown types) never panics and never loses a complete event.
- A secret placed in the parent environment never reaches a subscription child, and the API key never reaches a step that is not an API step.
- Cancel, timeout and abrupt supervisor exit leave no descendant process on each target enabled for unattended execution. Confirm no console window from a Windows GUI parent.
- Each `Result` subtype maps to the right normalised result; `error_max_turns` is not an error but a stop.
- Cost: API cost equals the reported `total_cost_usd`; the subscription estimate equals tokens times the table.

## Fixture verification

On 2026-11-20, the Windows and Linux CLI runs both recorded the `sample-export` Understand result. A second process replayed all six events. `watch --after 3` returned the three later events, including the structured result. The watcher reads committed history and exits. `cargo xtask ci` passed with 19 tests on each platform.

## Exit criteria
A fake Understand run replays from a separate CLI process. A manual synthetic Understand call with repository tools disabled returns structured output, tokens and reported or estimated cost on the validated Windows configuration. Keep sanitized fixtures and the versioned contract note.
