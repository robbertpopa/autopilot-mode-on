<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# autopilot-mode-on: implementation plan

Companion to `DESIGN.md` (what we build and why). This file says how, in what order, with which parts, and how we know each step works.
Reviewed 2026-11-20. The research notes below are dated references. Provider behavior, dependency compatibility, and containment need the evidence described in the spikes before they become release requirements.

## 0. Status and next step

**Next session starts here.** Work in `D:\autopilot-mode-on`. M0 and the [S5 synthetic process checks](spikes/S5-windows-process-control.md) passed. The [fixture-backed M1 CLI](milestones/M1-agent-step.md#fixture-verification) runs and replays a ticket and saves streamed activity before exit. The offline S1 preflight is ready. Capture the versioned S1 provider contract once the dedicated workspace and budget are confirmed. The [S3 source review](spikes/S3-read-gru.md#result) and [M4 local worktree operations](milestones/M4-worktrees-pr.md#part-1-git-crate-can-start-after-m0) are verified. The [S7 graph comparison](spikes/S7-graph-widget.md#result) selected a fixed egui canvas. The [S8 documentation and logged-out preflight](spikes/S8-codex-auth.md#result) are recorded. Live auth switching, event capture and the unattended-use decision still gate M8. S9 and M9 add the later codebase-review workflow.

The standalone repository holds the current implementation and milestone status. Record spike results and tick milestone items after running their exit checks.

| Item | File | State |
|---|---|---|
| S1 headless contract | `docs/spikes/S1-headless-contract.md` | [ ] |
| S2 plan-limit detection | `docs/spikes/S2-plan-limits.md` | [ ] |
| S3 read Gru | `docs/spikes/S3-read-gru.md` | [x] |
| S4 Claude Design headless | `docs/spikes/S4-claude-design-headless.md` | [ ] |
| S5 synthetic process control | `docs/spikes/S5-windows-process-control.md` | [x] |
| S6 containment | `docs/spikes/S6-containment.md` | [ ] |
| S7 graph widget | `docs/spikes/S7-graph-widget.md` | [x] |
| S8 Codex auth and terms | `docs/spikes/S8-codex-auth.md` | [ ] |
| S9 review quality and cost | `docs/spikes/S9-review-quality.md` | [ ] |
| M0 foundation | `docs/milestones/M0-foundation.md` | [x] |
| M1 one agent step | `docs/milestones/M1-agent-step.md` | [ ] |
| M2 understand, ask, inbox | `docs/milestones/M2-understand-ask.md` | [ ] |
| M3 plan, build, check | `docs/milestones/M3-plan-build-check.md` | [ ] |
| M4 worktrees, parallel, PR | `docs/milestones/M4-worktrees-pr.md` | [ ] |
| M5 window | `docs/milestones/M5-window.md` | [ ] |
| M6 design step | `docs/milestones/M6-design-step.md` | [ ] |
| M7 evaluation | `docs/milestones/M7-evaluation.md` | [ ] |
| M8 Codex adapter, MCP, packaging | `docs/milestones/M8-codex-mcp-packaging.md` | [ ] |
| M9 codebase review and fix | `docs/milestones/M9-codebase-review.md` | [ ] |

## 0a. Implementation order and deliverables

The first demo takes a local ticket through Understand and an optional inbox question, then shows a replayable result in the CLI. The first useful release takes one non-UI repository ticket through Plan, Build and Check and produces a local commit and a check summary. The draft-PR release adds explicit publishing approval. Parallel tickets, the desktop window, Design, evaluation, Codex and MCP follow in their existing milestones.

| Slice | Files and work | Acceptance |
|---|---|---|
| **M0.1 Workspace** | Create `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, `LICENSE`, `deny.toml`, `.github/workflows/ci.yml` and `tools/xtask/src/main.rs`. Verify the pinned compiler and compatible crate versions on both targets. Wire the CI checks from M0. | Workspace checks run on Windows and Linux. The layer check rejects a deliberately inverted dependency. |
| **M0.2 Event state and profiles** | Define IDs, money, event envelopes and stored step outcomes in `crates/core/src/lib.rs`. Add SQLite migrations, sequence-checked append, replay and snapshots in `crates/store/src/`. Add TOML parsing, diagnostics and resolved settings in `crates/profile/src/`. | Reopen the store after an interrupted transaction. Replay from the beginning and from a snapshot plus its tail, and compare the state and sequence. An invalid profile reports its file, key and location. |
| **S1, S2, S5. Agent contract** | Start S5 with `tools/fake-agent` and record process-tree behavior on both targets. For S1, capture and sanitize a minimal structured result, resume and failure transcript before adding optional flags. Use S2's local server to exercise retries and keep an explicit unknown-limit case. | Versioned sanitized fixtures and a written provider contract. Process cleanup evidence covers normal completion, cancel, timeout, and abrupt supervisor exit per target. A target without proven tree cleanup stays fixture-only for unattended execution. |
| **M1. Fixture-backed CLI** | Put supervision in `crates/process/src/`, the provider-neutral `Agent` contract in `crates/agent/src/`, parsing and provider flags in `crates/agent-claude/src/`, and actions in `crates/engine/src/`. Add the CLI and a fixture runner in `crates/cli/src/` and `tools/fake-agent/src/`. | Build with `cargo build --locked -p autopilot-process -p autopilot-fake-agent -p autopilot-cli`. Run `cargo run -p autopilot-cli -- run-step --step understand --ticket fixtures/tickets/understand.json --agent fake`. It prints a ticket ID and structured result. In a second process, `autopilot watch <ticket-id>` replays the same run and resumes from its event sequence. One manual live smoke test uses a synthetic ticket and no repository tools. |
| **M2. Inbox and resume** | Add the Understand schema, step machine, policy, durable requests/answers, settings snapshots and CLI status in `crates/pipeline/src/`, `crates/policy/src/`, and the shared engine. Keep the Plan stub local. | Zero questions records a skip reason. A question can be answered after a restart and consumes the answer once. Killing a worker leaves an interrupted attempt that the user can resume. |
| **M4 part 1 + S6. Worktrees and isolation** | After M0, implement tree ownership, clean base selection, fingerprints and recoverable cleanup in `crates/git/src/`. After S1 and S5, prove the Build and Check isolation contract in S6. Keep a synthetic repository for repeatable tests. | A cancelled run leaves no active worker. The isolated agent and check processes cannot access adjacent fixtures or credentials, mutate the base checkout, or publish. A platform or auth mode without evidence stays fixture-only for repository Build and Check. |
| **M3. Local Build and Check** | Add Plan analysis and approval, immutable criteria, a sequential check runner and bounded refinement in `crates/pipeline/src/` and `crates/checks/src/`. Land fixture-driven tests while S6 runs. Unlock real Build and Check only after S6 passes for that combination of platform, launcher and auth mode. | A sample ticket goes through one failed check and one successful refine round. Green evidence records actual checks against the final worktree fingerprint. The result is a local commit and summary. |
| **M4 part 2. Draft pull request** | Implement issue import and draft PR operations in `crates/tracker/src/` and scheduling and reconciliation in `crates/pipeline/src/`. Prove single-ticket publishing first. Add file-conflict scheduling and CI intake afterward. | Repeating the same approved publish after a simulated timeout or restart creates at most one draft PR. An altered head or rejected request cannot publish. The real smoke test uses an explicitly approved disposable remote. |

Use the milestone checklists above for detailed tasks. The CLI selects the sibling fake-agent binary only when `--agent fake` is supplied. Both CLI processes use the same local data directory. Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace`, and `cargo xtask layers` before marking a slice complete. The release checks run on both platforms. Normal tests use fixtures and local processes.

### First implementation session

- [x] Copy the project documents into the standalone repository and start [M0](milestones/M0-foundation.md). Add an `autopilot-core` library and the `xtask` member. Get the workspace checks running before adding dependencies for later milestones.
- [x] Add the store tests in `crates/store/tests/it/main.rs`. Have two append attempts use the same expected sequence. Assert that only one succeeds and the event appears once after reopen. Persist a snapshot at sequence 2, append an event at sequence 3, and assert that snapshot-plus-tail and full replay produce the same state.
- [x] Add the append and replay implementation in `crates/store/src/`. Use `(ticket_id, seq)` as the event key and store the snapshot's last sequence per ticket. The targeted store tests were exercised before and after implementation. The final workspace check also runs them through Nextest.
- [x] Add `crates/profile` and its invalid-profile cases, then verify `cargo xtask ci` on Windows and Linux. The results are recorded in [M0](milestones/M0-foundation.md#verification).
- [x] Use [S5](spikes/S5-windows-process-control.md) to verify the local process launcher on Windows and Linux with synthetic processes.

## 0b. Contracts to settle in the first slices

- **Durable state.** SQLite is authoritative. Identify an event by ticket and sequence, and an execution by a separate attempt ID. Append with an expected last sequence in one transaction. Snapshots carry their last sequence and schema version. Persist terminal step outcomes and pending requests before broadcasting. Readers replay by sequence and catch up from storage after a gap. M1 has one run owner per data directory. CLI watchers are read-only.
- **Inbox and interruption.** Request files mirror durable requests. Write answer files atomically, validate them against the pending request ID, and append `Answered` under a uniqueness constraint before acknowledging or cleaning up the files. Reopen with in-flight attempts marked `Interrupted`. Resume is explicit and uses the captured step settings. Apply profile edits at the next step boundary. Store an opaque local session reference in the event log and keep any provider resume identifier in restricted local state. Exclude it from exports and fixtures.
- **Launcher and isolation.** A worktree identifies the ticket's files. Keep the common Git metadata outside the agent's write scope. The trusted Git path creates the local commit after the checks pass. The sandbox enforces the permitted reads, writes, process launches and network access. Constrain agent descendants and check processes, including tools launched by build and test scripts. Use synthetic out-of-scope files and credentials for tests. Permit the provider connection through the validated runtime and deny agent tool egress. If provider auth exposes credentials to repository commands, keep that configuration disabled. Hook rules and environment scrubbing add protection inside this boundary.
- **Agent output.** Stream stdout and stderr with explicit line and total-byte limits. Keep unknown fields forward compatible. Persist bounded, redacted activity and a validated structured result. Keep fixture captures synthetic and sanitize them before writing them under `fixtures/`. A process exit, missing terminal result, invalid schema or idle timeout has its own outcome. Any suspected subscription limit pauses for user input. API fallback stays off until supported by evidence and explicitly enabled.
- **Plan and checks.** The approved Plan is immutable engine data. The agent can report progress separately. The engine derives `passes` from executed verification evidence and blocks `unknown` criteria. Resolve each criterion's verification ID to a trusted, locally configured check with an executable and argument list. Unknown IDs stop Plan approval until resolved. Reuse green results only when the base, full worktree fingerprint, approved plan and effective check configuration match. Rerun the scan before publish.
- **Approval and publishing.** Keep confirmation in the inbox at the step that needs it. Before push or PR creation, bind approval to the repository, base, head commit and a fresh check/diff result. Publish via a separate host-side `tracker` path with GitHub credentials. Persist the intended operation first. On an uncertain result, look up the exact branch and open draft PR before retrying. Stop on a mismatch or a protected target. The first release displays CI failures for review. Any later CI repair requires an eligible check from the expected PR head and another approval before updating that head.

## 1. Decisions the research settled

### Build our own; do not fork
| Project | Facts | Verdict |
|---|---|---|
| Vibe Kanban (BloopAI, Apache-2.0) | Rust + React, ~30 crates, SQLite, nightly toolchain, 10+ agent executors, worktrees, diff review, PR creation. **Sunsetting**: the company shut down in April 2026, the project continues community-maintained; last release 2026-11-15. No clarify, design, check loop, per-step auth or cost caps. | Too big and in transition to adopt. Borrow ideas: worktree manager, stored execution logs replayed on reload, executor list. |
| Gru (Apache-2.0, Rust) | Active (last commit 2026-11-19, ~660 commits, 11 stars). Claim an issue, worktree, agent, tests, review subagent, PR, watch CI and auto-fix (max 2 attempts). State in GitHub labels and a JSON registry. Calls `claude --print --verbose --session-id <UUID> --output-format stream-json --dangerously-skip-permissions --include-partial-messages`. One GitHub token, no per-step auth, no clarify stage. | Contributing makes sense only for the CI-fix loop. Our core differs (event log, per-step auth, asking). Read its source (spike S3). Do **not** copy its `--dangerously-skip-permissions`. |
| oven-cli (MIT, Rust) | Claude-only, SQLite, planner DAG, reviewer-fixer loop (max 2), `cost_budget`. Last commit March 2026, 0 stars. Tests never call real CLIs (mocks). | Not a base. Adopt its test rule. |
| craft-anything (MIT OR Apache-2.0) | Build, Check, refine loop; inbox of requests and answers; pause and resume; layer rules checked by xtask. | Pattern source (design section 5). Licence files confirmed, so code could be reused with notices; we still write our own. |
| spec-kit (GitHub) | Clarify: 10 categories each marked Clear, Partial or Missing; at most 5 questions, one at a time, a recommended answer on each, "yes" accepts it; answers recorded as `Q → A`; says "No critical ambiguities detected" when nothing is worth asking. Analyze: read-only, severity levels, coverage table. | Our Understand and Clarify output format (section 6, M2). |
| Codex CLI (Apache-2.0, Rust) | `codex exec --json` events: `thread.started`, `turn.*`, `item.*` with `usage` including cached tokens. Sandbox modes read-only, workspace-write, full access, network off by default; a Windows sandbox exists. Tests use insta, wiremock, assert_cmd; clippy denies unwrap and expect. | Second adapter (M8). Our event model must fit both. Policy design reference. |

### Claude Code facts we build on (official docs, fetched 2026-11-20)
- `claude -p --output-format stream-json --verbose` emits SDK-style messages: system init (`session_id`, `cwd`, `tools`, `model`, `permissionMode`, `claude_code_version`),
  assistant and user messages, `system/api_retry` (attempt, delay, error such as `rate_limit` or `overloaded`), and a final `result`
  (`subtype`: `success`, `error_max_turns`, `error_max_budget_usd`, `error_during_execution`, `error_max_structured_output_retries`; `total_cost_usd`,
  `usage` with cache read and creation tokens, `num_turns`, `duration_ms`, `session_id`, `permission_denials`).
- `--json-schema` returns validated output in `structured_output`. `--max-turns` has no default. `--max-budget-usd` is a client-side estimate and resumed totals do not count.
- `-p` with no mode set starts in Manual mode from v2.1.302: calls that need approval are denied. `--permission-prompt-tool mcp__<server>__<tool>` exists; its response contract was
  **not retrieved**. `--resume` needs `--mcp-config`, `--add-dir`, `--settings` and permission flags repeated.
- `claude auth status` prints JSON (`authMethod`) and exits 0 or 1. Limit messages read "You've hit your session limit" or "weekly limit"; retries show as `system/api_retry`.
  There is no per-error exit code. A set `ANTHROPIC_API_KEY` outranks the login in `-p`. `--bare` is key-only and skips CLAUDE.md, hooks and MCP, so API steps should **not** use it.
- Hooks (33 events) can enforce hard rules (`PreToolUse` deny); an http hook can post events to a URL. Transcripts live in `~/.claude/projects/…jsonl`, format internal: we never parse them.
- Windows: native `claude.exe` in `%USERPROFILE%\.local\bin`; Git Bash recommended, not required. **No documented way to start it without a console window.**
- Headless use of the Artifact tool or Claude Design: **not documented**.

## 2. Principles for building it (from Anthropic's agent articles and the survey)

1. **A prompt chain with checks, not an autonomous agent.** Fixed steps, code decides the next step. Use orchestrator-workers only inside a step whose file set is unknown.
   The Build ⟲ Check loop is an evaluator-optimizer and is only used because the criteria are clear and checkable.
2. **Hard rules live in code and hooks, never in prompts.** Protected paths, branch rules and secret access are enforced by `policy` and by `PreToolUse` deny rules per step.
3. **Pass validated structured results between steps.** Store the approved criteria in engine-owned state and export a copy for context. The engine sets `passes` from check evidence.
4. **Context is a budget.** Repo map and touched files, not the whole repo; a fresh session after two failed refine rounds with a summary of what failed.
5. **Cap everything.** Turns, dollars, rounds, wall time; a loop detector; a stuck stream (Gru uses 15 minutes) becomes `Needs you`.
6. **Parallelise only independent tickets.** Coding agents coordinate poorly; isolation by worktree and a file-conflict queue.
7. **Measure.** Start an eval from ~20 real tickets; judge by end state and tests, plus one rubric call returning a 0 to 1 score; track runtime, tool calls, tokens and errors.
8. **Untrusted text.** Issue text and comments are data. Attacks reach 85% or more against adaptive defences, so assume injection reaches the model and limit the impact: no secrets in the agent's environment, PRs opened by our code (the agent never holds the GitHub token), network off where possible, and a diff scan for `.github/**`, lockfiles and scripts.

## 3. Spikes (run first; each has a question, a method and a stop condition)

Budget for all live spikes: **at most $1**, on a dedicated API key with a Console workspace spend limit, Haiku, tiny prompts, `--max-budget-usd 0.05`.

| # | Question | Method | Done when |
|---|---|---|---|
| S1 | What exactly does the headless interface emit and accept? | Run real `claude -p` calls and record every event: stream-json, `--json-schema` (does `structured_output` appear in stream-json mode?), `--resume`, `--max-turns`, `--max-budget-usd`, `--permission-prompt-tool` (find the response shape by trial), `--allowedTools`, `--disallowedTools`, `--session-id`, `--settings` with a hook. Save sanitized synthetic transcripts as fixtures. | Versioned fixtures, a contract note and parser replay tests. |
| S2 | How does a plan or rate limit look, and how do we detect it? | Capture `system/api_retry` and limit text on a throttled key if possible; otherwise read the docs' error page and plan the detector as "api_retry with `rate_limit` for N seconds, or result text matching a limit phrase". | Detector rule written; marked **unverified on a real subscription** until seen. |
| S3 | What can we learn from Gru? | Read its worktree, stream-stuck and CI-fix code. | A list of pitfalls and 3 ideas to copy (for example pre-assigned `--session-id`). |
| S4 | Can a headless session publish a Claude Design canvas, and under which login? | Manual test on the subscription: `claude -p` asking for a Design artifact; check the link and permissions. | Yes (and how) or no; if no, the HTML-mockup fallback is the default Design step. |
| S5 | How do we run `claude.exe` with no window and kill its whole tree on Windows? | Small Rust program: `creation_flags(CREATE_NO_WINDOW)`, a Job Object with kill-on-close, `kill_on_drop`, stdout pipe reading. Test cancel, timeout, parent crash. | No flashing window; no orphan processes after cancel or crash. |
| S6 | How far can we contain an agent on Windows? | Test agent and check runtimes with synthetic fixtures. Verify filesystem and process-tree boundaries, tool egress restrictions and credential isolation. | A platform and auth capability table that determines where real Build and Check can run. |
| S7 | egui-snarl or our own graph? | One-day prototype of the 6-node graph with arrows, selection and per-node widgets. | Choose by whether snarl's node widgets can hold our settings forms without fights. |
| S8 | Can Codex run on a ChatGPT subscription from `codex exec`, how do we switch between subscription and API without the cached login interfering, and what do OpenAI's terms say about driving it from a local tool? | Read OpenAI's auth and terms pages; try `codex login status` and `codex exec --json` on a tiny task with each method; test a separate config directory per auth mode (find the environment variable). | A written rule per auth mode, or a decision that Codex runs on the API key only. |
| S9 | How well does the review pipeline find real defects and filter noise? | Compare the three review variants on a seeded synthetic sample. Measure precision, recall and cost per verified finding. | A measured baseline and chosen review-unit size, caps and verifier configuration. |

S1 and S5 gate M1's live execution. S2 gates automatic limit classification and any later API fallback. S6 gates the first real repository Build or Check in M3 and publishing in M4. S7 gates M5's graph. S4 gates the remote Design path in M6. S8 gates the Codex adapter only. [S9](spikes/S9-review-quality.md) measures review quality and cost before M9 model-assisted review.

## 4. Technology choices

Rust stable, edition 2024, toolchain pinned in `rust-toolchain.toml` (same pin as the ck-* repos at the time). Windows is the primary platform, Linux second; CI runs both.

| Need | Choice | Notes (crates.io, 2026-11-20) |
|---|---|---|
| Async and processes | `tokio` 1.54 (`process`, `sync`, `time`) | `kill_on_drop`; Windows Job Object through the `windows` crate (S5). `command-group` is stale (2023): not used. |
| CLI | `clap` 4.6 | |
| Serialisation | `serde`, `serde_json` 1.0, `toml` 1.2 | |
| JSON Schema | `schemars` 1.2 | One source for `--json-schema` shapes and profile validation. |
| Errors, logs | `thiserror` 2.0, `tracing` | No `unwrap` or `expect` outside tests; no `println!` (lint). |
| Event store | `rusqlite` 0.43 (bundled SQLite) behind a `Store` trait | Sync and simple; `sqlx` 0.9 would pull async and macros we do not need. |
| Git | the `git` CLI for every mutating step; `gix` 0.90 optional for read-only | Same approach as Claude Code, Gru and oven. `git2` 0.21 is an alternative if the CLI proves slow. |
| GitHub | `gh` CLI first (reuses your login; Gru and oven do the same); `octocrab` 0.54 later | The agent never gets the token. |
| Paths | `camino` 1.2 | UTF-8 paths on Windows. |
| Finding `claude` | `which` 8.0 | Also `%USERPROFILE%\.local\bin`. |
| Notifications | `notify-rust` 4.19 | |
| GUI | `eframe` 0.37 (egui) | |
| Graph widget | Fixed egui canvas | The fixed six-node layout and edge labels passed the [S7 probe](spikes/S7-graph-widget.md#result). |
| Tests | `cargo-nextest` 0.9, `insta` 1.49, `assert_cmd` 2.2, `proptest` 1.12, `tempfile` | `wiremock` only if we mock HTTP; with the `gh` CLI we mock the process instead. |
| Quality gates | `cargo clippy -D warnings` (pedantic, as in the ck-* repos), `cargo deny`, `cargo machete`, `cargo xtask layers` | |

## 5. Repository layout (Apache-2.0; repo `autopilot-mode-on`)

```
crates/
  core        ids, ticket, step, event, money, errors                       (L0)
  store       Store trait + SQLite event log, snapshots, fold               (L1)
  profile     profile.toml schema, defaults, validation, AGENTS.md reader   (L1)
  process     bounded subprocess lifecycle and platform supervision        (L1)
  git         worktrees, branches, diff, conflict prediction (git CLI)      (L2)
  tracker     GitHub issues and PRs through gh                              (L2)
  policy      sandbox and approval axes, risk tier, forced confirmations,
              budgets, loop detector, protected paths                       (L2)
  agent       Agent trait, normalised AgentEvent                            (L3)
  checks      Check trait, ordered runner, kept results                     (L3)
  agent-claude  Claude Code headless adapter, stream-json parser            (L4)
  agent-fixture  synthetic Understand adapter for the CLI demo             (L4)
  agent-codex   Codex CLI adapter, JSONL parser (M8)                         (L4)
  pipeline    step machine, scheduler, inbox, ledger, summaries             (L4)
  engine      one definition per action -> CLI, MCP, UI                     (L5)
  cli  mcp  ui                                                              (L6)
tools/
  fake-agent  test binary that replays recorded transcripts and simulates faults
  xtask       layers, secrets, ci
fixtures/     recorded stream-json transcripts, sample repos, profiles
docs/         headless contract note, pitfalls (past mistakes)
```
Rules enforced by `xtask layers`: production dependencies point down. Direct OS subprocess calls live in `process`. Its production consumers are Git, tracker, checks and the concrete agent adapters. Each consumer owns its own executable, arguments and environment. Tooling such as `xtask` sits outside the product layers. Only `ui` uses egui. Library crates produce structured events and errors. The CLI owns terminal output.
`AGENTS.md` (with `CLAUDE.md` containing `@AGENTS.md`) in the repo root holds the working rules, and a `.coderabbit.yaml` reviews PRs into `develop`.

**Normalised agent event** (so Claude and Codex fit): `Started{session}`, `Message{text}`, `Tool{kind, summary, path?}`, `FileChange{path, kind}`,
`Usage{input, output, cache_read, cache_write}`, `Retry{reason, delay}`, `RateLimited{kind}`, `Result{ok, subtype, cost_usd?, turns}`, `Failed{reason}`.
Ignore unknown message types and fields in the state machine. Persist only bounded, redacted diagnostics. Record `claude_code_version` and replay sanitized fixtures in contract tests.

## 6. Milestones

Each milestone ends with a working vertical slice and its tests. The window comes late on purpose: everything works from the CLI first, and the window is a thin client of the same events.

### M0: foundation
- Repo, toolchain, workspace, lint config (pedantic, no unwrap, no print), `cargo deny`, nextest, CI on Windows and Linux, `AGENTS.md`, CodeRabbit, release-please.
- `core`, `store` (append-only events, snapshot, `fold`), `profile` (schema with defaults, validation errors with line numbers), `xtask layers`.
- Tests: `fold(events)` is deterministic; a store reopened after a crash gives the same state; invalid profiles give precise errors (insta snapshots).
- Exit: `cargo xtask ci` green on both platforms.

### M1: one agent step from the CLI (live needs S1 and S5)
- `agent` trait and `process` supervisor (no window, job object, timeout, cancel, stdout and stderr capture). Introduce `engine` as the composition point for CLI actions.
- `agent-claude`: build the command (model, effort, turns, budget, allowed and disallowed tools, `--settings` hooks, `--session-id`), parse the stream into `AgentEvent`, detect `Result` subtypes, rate limits and retries.
- Per-step auth: strip or set `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `CLAUDE_CODE_USE_*` on the child only; preflight `claude auth status`.
- Cost ledger: `total_cost_usd` for API steps; for subscription steps tokens times a price table (from the pricing page) labelled as an estimate.
- CLI: `autopilot run-step --step understand --ticket <file>` and `autopilot watch <ticket-id>`.
- Tests: `fake-agent` replays fixtures and injects faults (truncated line, partial UTF-8, huge output, hang, crash, rate limit, `error_max_turns`); proptest sweep on the parser; env-scrubbing test proves a secret never reaches a subscription child; Windows test proves no orphan process after cancel.
- Live smoke test behind a `live` feature and `#[ignore]`, capped at $0.05.
- Exit: one real Understand call returns structured output and a cost, from the CLI, on Windows.

### M2: understand, ask, inbox, step settings
- Understand with `--json-schema`: the 10-category ambiguity map (Clear, Partial, Missing), at most 5 `open_questions` (each with options and a recommended answer), `assumptions`, `needs_design`, `ui_files`.
- Clarify: asks through the inbox (question and approval kinds, request and answer files in an owner-only folder), records `Q → A`; "No critical ambiguities" skips Clarify.
- The "Ask me after this step" checkbox and forced confirmations in `policy` (protected path, risky tier, budget). "When unsure: ask me | decide and tell me".
- Pipeline step machine with skip reasons, `Needs you`, pause and resume (saved session id, flags repeated on resume).
- Code-made "In short" summaries (templates); "Explain in plain words" on request, cached in the log.
- Tests: table-driven policy tests (every forced-confirmation rule, and that the checkbox cannot loosen a rule); inbox file protocol including a crash between request and answer; skip-reason snapshots; resume after kill.
- Exit: a real ticket goes Understand, optional Clarify, Plan stub, and stops where asked.

### M3: plan, build and check loop (real execution needs M4 part 1 and S6)
- Plan: acceptance criteria as JSON (`id`, text, `verify: test|command|unknown`, `passes`), test plan, file list; a read-only Analyze pass (spec-kit style: coverage of criteria by tasks, ambiguity words) before approval.
- Build in a worktree; Check runner in the agreed order (build, fmt, clippy; tests; protected paths and diff size; criteria; review last as an agent in a fresh context); stop at the first failure; refine rounds with a cap; fresh session after two failed rounds; not-working counter; kept results when the tree fingerprint is unchanged.
- Tests: checks against throwaway git repos (assert_cmd); "unknown cannot pass"; loop and budget caps; kept-result invalidation on any file change.
- Exit: a real small ticket on a sample repo goes from plan to green checks.

### M4: worktrees, parallelism, PR (worktrees after M0, publishing after M3 and S6)
- Worktree manager (Windows file-lock retries on delete), file-conflict prediction and queue, parallel runs with a slot limit, containment from S6 (deny hooks, no secrets, `--add-dir` scope).
- Draft PR creation through `gh` in the host-side tracker, with approval tied to the checked head. Start with one ticket and CI status. Use S3 before implementing CI repair with a 2-attempt cap.
- Diff scan for `.github/**`, lockfiles and scripts forces a confirmation.
- Tests: two tickets touching one file queue; a poisoned issue text cannot cause a push or read a secret (end-to-end test with `fake-agent` that obeys the injection); worktree cleanup after crash.
- Exit: two tickets run in parallel on a sample repo and open two draft PRs.

### M5: the window (needs S7)
- Event subscription with replay; sidebar of tickets with live chips; graph with node settings and the checkbox; Build ⟲ Check panel; step detail tabs and actions; inbox cards; cost bars; notifications; "Ask for a change" with the three targets and the profile-patch diff.
- Repaint on events, not per frame. All logic stays in `engine`; the UI never spawns processes or writes files.
- Tests: view-model tests on the engine; a handful of layout smoke tests; every UI action maps to an engine action that the CLI tests already cover.
- Exit: a ticket can be run, watched, answered and merged-ready entirely from the window.

### M6: the design step (remote path needs S4)
- Start with local HTML mockups and an inbox approval. S4 can unlock the remote Claude Design path, including revisions to the same canvas.
- Approved design attached to the plan. Optional later: a visual check against it.
- Exit: a UI ticket produces an approved design before any code.

### M7: evaluation
- `eval` crate: take merged PRs with their issue text from a project's history, run the pipeline from the base commit, judge by the tests the PR added plus one rubric call; record resolve rate, dollars, time, questions asked, wrongly skipped questions.
- First corpus: Contest Keeper PRs, about 20. Used to tune models, effort, round caps and the Understand prompt. Not in the CI gate (it costs money).

### M8: second adapter and integrations
- Codex adapter on `codex exec --json`; MCP server over `engine`; GitHub App authentication option; release packaging (Windows installer, `cargo install`).

### M9. Codebase review and fix

- Start with a report-only review using scoped source inventory, mechanical checks, evidence-linked findings, and independent verification. Report-only needs M3 and the S9 baseline. Running repository code or agents on real projects uses the validated S6 runtime.
- Add approved fix batches after M4. Each batch follows the same ticket checks, budgets and publishing approval.
- The [milestone](milestones/M9-codebase-review.md), [review design](REVIEW-PIPELINE.md) and [rule packs](rules/README.md) define the scope and acceptance tests.

## 7. Test strategy (applies our testing standard to the new repo)

- **Keep rule:** a test stays only if it states the invariant, a plausible breaking change and that no other test catches it. Cut categories: readable code, wiring, defaults, compiler-checked, library behaviour, duplicates, combinations.
- **No real agent in CI** (oven's rule). `fake-agent` replays fixtures; real calls are `#[ignore]` behind the `live` feature, run by hand under a dollar cap.
- **Hostile input** for every parser (stream-json, profile, inbox files, `structured_output`): truncation, bit flips, unknown types; never a panic.
- **One integration binary per crate** (`tests/it/main.rs`); shared helpers in a `testkit` dev crate.
- **Determinism:** event-fold and replay tests; the clock is injected.
- **Windows is a first-class CI job**, because process control and paths are where the risk is.
- **Contract test against Claude Code versions:** CI records `claude --version`; a nightly job (optional, costs nothing) runs fixture replay; a manual `live` run is repeated when Claude Code updates and the fixtures are re-recorded if the contract changed.

## 8. Risks and what we do about them

| Risk | Mitigation |
|---|---|
| Headless Claude Design does not work | S4 first; HTML-mockup fallback is a full alternative, not a stub. |
| Plan-limit detection is wrong | S2; the fallback to the API key is off by default until verified; show "limit suspected" and ask. |
| `--permission-prompt-tool` contract unknown | S1 finds it by trial; until then run steps with an explicit allowed-tools list and deny hooks, so no prompt is needed. |
| Stream format changes between Claude Code versions | Tolerant parser, recorded `claude_code_version`, fixture contract tests, pinned minimum version in the preflight. |
| Orphan processes or a flashing window on Windows | S5 before M1 is accepted; job objects; test on cancel and crash. |
| Prompt injection through issues | Section 2 rule 8; end-to-end poisoned-ticket test in M4; never `--dangerously-skip-permissions`. |
| Cost overrun while developing and testing | Dedicated key with a workspace limit; Haiku for spikes; `live` tests capped; ledger from M1. |
| Subscription limits stall parallel runs | Slot limit; per-step auth lets cheap steps use the key; queue shows "waiting for plan limit". |
| Too much scope | The CLI slice (M1 to M4) is useful alone; the window and design step come after it works. |

## 8a. Working in several tabs at once

Each tab starts with: "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of <file>." Each tab owns its crates and does not edit another tab's crates.
If it needs a change there, it adds one line under "Requests to other tabs" below and carries on with something else.

| Tab | Work | Owns | Can start when |
|---|---|---|---|
| 1 | M0, S1 and S2, then M1 | `core`, `store`, `profile`, `agent`, `agent-claude`, `tools/fake-agent`, `cli` | M0 now; M1 live after S1 and S5 |
| 2 | S5, then shared process supervision | `process` | S5 now |
| 3 | S7, then M5 (window) | `ui` | S7 now; M5 after M2 has events |
| 4 | S3, S4, S6, S8 (reading and trials, no product code) | `docs/spikes/*` | S6 after S1 and S5; others as needed |
| 5 | M2 (understand, ask, inbox) | `pipeline`, `policy` | M1 done |
| 6 | M4 part 1 (worktrees, conflict queue) | `git` | M0 done |
| 7 | M8 part 1 (Codex adapter) | `agent-codex` | M1 done and S8 done |
| 8 | M3, then M4 part 2, M6, M7 | `checks`, `tracker`, `eval` | M3 fixtures after M2; real Build and Check after M4 part 1 and S6 |
| 9 | S9, then M9 | `rules`, `review` | S9 offline setup now. M9 report-only after M3 and S9. Fixing after M4. |

The engine starts in M1 and follows the CLI action work. Sequence M2, M3 and M4 edits to `pipeline` so there is one owner at a time. Sequence S5 and M1 edits to the launch test fixtures the same way.

Requests to other tabs: (none yet)

## 9. How we work on it

- A new repo with its own `AGENTS.md`; branches from `develop`, squash PRs, titles `type: lowercase imperative`; one milestone per branch family; moves and behaviour changes in separate PRs.
- The ck-* rules about asking the GitHub account apply to work under `D:\contest-keeper`; the new repo lives outside it (proposed `D:\autopilot-mode-on`), so its own `AGENTS.md` sets the account rule.
- Each session starts by reading this plan and `DESIGN.md`, does the next unticked item, and ticks it. Spikes write their results to `docs/` so the answers outlive the session.
- Definition of done for an item: code, tests that pass the keep rule, `cargo xtask ci` green on Windows and Linux, and the plan ticked.

## 10. Decisions made

1. **Name:** `autopilot-mode-on` (crates.io name free on 2026-11-20; plain `autopilot` is taken by another crate). The local repository is initialized.
2. **Licence:** Apache-2.0.
3. **Default auth: subscription**, for Claude Code and for Codex; API key per step as an option (see `docs/DESIGN.md`, section 7a).
4. **Work method:** this plan is split into files so separate tabs can continue; see section 8a.

Remote repository ownership and the S2 and S8 findings are still open.
