# M1: one agent step from the CLI

**Owner crates:** `process`, `agent`, `agent-claude`, `tools/fake-agent`, `engine`, `cli` (first commands). **Depends on:** M0. Live execution needs S1 and S5. Automatic limit classification needs S2. **Gate for:** M2 and M8.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M1-agent-step.md`."

## Goal
Demonstrate a replayable Understand run with fixtures, then run one synthetic ticket through the validated headless Claude Code setup on Windows.

## Tasks
- [ ] `agent` crate: `Agent` trait (`start(step_request) -> event stream + handle`), normalised `AgentEvent` (`Started`, `Message`, `Tool`, `FileChange`, `Usage`, `Retry`, `RateLimited`, `Result`, `Failed`) and provider-neutral result types. Use `crates/process` for bounded stdout and stderr, prompt on stdin, timeouts, cancellation and platform process-tree cleanup.
- [ ] `agent-claude`: command builder (model, effort, max turns, budget, allowed and disallowed tools, `--settings` with deny hooks, `--session-id`, `--json-schema`, `--append-system-prompt`); stream-json parser from the S1 fixtures; `Result` subtypes; rate-limit and retry detection per S2; record `claude_code_version`.
- [ ] Per-step auth: build the child environment from scratch. Subscription: remove `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_*`. API: inject the dedicated key only into the provider environment verified for that step. Preflight with `claude auth status` (exit code and `authMethod`); a clear message if not signed in or if the method does not match the step's `auth`.
- [ ] Cost ledger: API steps use `total_cost_usd`; subscription steps estimate from token usage and a price table (from the pricing page, with cache read and write multipliers), always labelled "estimate".
- [ ] `tools/fake-agent`: a binary that replays a fixture transcript and injects faults on request (truncated line, partial UTF-8, huge output, hang, crash, rate limit, `error_max_turns`).
- [ ] `engine` composes the adapter and store. Use `autopilot run-step --step understand --ticket <file> --agent fake` and `autopilot watch <ticket-id>` from separate processes.
- [ ] A `live` feature test, `#[ignore]`, capped at $0.05.

## Tests that must exist
- Parser: every fixture parses; a hostile sweep (truncation, bit flips, unknown types) never panics and never loses a complete event.
- A secret placed in the parent environment never reaches a subscription child, and the API key never reaches a step that is not an API step.
- Cancel, timeout and abrupt supervisor exit leave no descendant process on each target enabled for unattended execution. Confirm no console window from a Windows GUI parent.
- Each `Result` subtype maps to the right normalised result; `error_max_turns` is not an error but a stop.
- Cost: API cost equals the reported `total_cost_usd`; the subscription estimate equals tokens times the table.

## Exit criteria
A fake Understand run replays from a separate CLI process. A manual synthetic Understand call with repository tools disabled returns structured output, tokens and reported or estimated cost on the validated Windows configuration. Keep sanitized fixtures and the versioned contract note.
