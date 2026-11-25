<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# autopilot-mode-on: design

Local Rust program (repo and crate name `autopilot-mode-on`). Input: a codebase plus stories, bugs and wishes. It clarifies the need in plain language (only when it
has something to ask), designs the UI when there is UI, plans, builds in isolated worktrees, checks, and opens PRs.
Tested first on Contest Keeper, built for any project. The repository is `robbertpopa/autopilot-mode-on` on GitHub; what is built and what is left is in the [implementation plan](IMPLEMENTATION-PLAN.md#0-status-and-next-step).

Status: written 2026-11-20 and kept as the statement of intent; where the code has moved on, a note says so (sections 6, 7, 8a, 8b). Survey numbers come from web snippets and blogs and are indicative, not verified.
Visual design: Claude Design canvas "Pipeline Canvas" (private artifact); style follows craft-anything's screens.

## 1. Decided and open

Decided by the user:
- Local program in Rust, egui window, CLI and engine first (the window is a thin client). Licence Apache-2.0.
- Workers are existing agents, Claude Code headless first. No own coding agent.
- Per-step auth: each step uses a subscription (default) or an API key, for Claude Code and for Codex. A `$20` budget for tests.
- Build and Check come from craft-anything's loop (build, check, refine, ready). **No Study step.**
- No gates. A step asks you whenever it needs a confirmation or more information, and every step has an "Ask me after this step" checkbox.
  The AI decides whether it has anything to clarify.
- A Design step produces a local HTML mockup and needs approval. The remote Claude Design option depends on S4.

Decided later the same day: the name is `autopilot-mode-on`; **subscription is the default auth**; Codex can be used too (section 7a).
Still open: the real-world behaviour of plan limits (spike S2).

## 2. What the survey says

| Source | Learned | We take |
|---|---|---|
| mini-SWE-agent | A ~100-line bash-only loop is competitive on SWE-bench Verified (project's own claim). | Do not write a coding agent; orchestrate existing ones. |
| OpenHands | Strongest full open-source platform in the results; heavy. | Reference for sandboxed runtimes. |
| Aider | Repo map and diff edits send fewer tokens. | Give agents a repo map and touched files, not the whole repo. |
| Codex CLI (Rust, Apache-2.0) | Sandbox mode and approval policy are separate settings; network off by default. | Same two axes in our policy; read its source before writing `policy`. |
| vibe-kanban, Parallel Code, AI Agent Board, Agetor, Kanban Code | Board, one worktree and branch per task, diff review before merge. | The board view and worktree handling. |
| Agent Orchestrator | CI failures and review comments go back to the agent that owns the PR. | The feedback loop after the PR opens. |
| Gru, oven-cli (Rust) | Issue to PR pipelines already exist. | Read both before M0; contribute instead of rebuilding if they cover the base. |
| GitHub spec-kit | Clarify (ambiguity scan), plan, tasks, read-only consistency check, implement. | Clarify, Plan and Analyze, with Clarify made conditional. |
| craft-anything (MIT OR Apache-2.0; LICENSE-MIT and LICENSE-APACHE present) | Build, Check, refine loop; checks are code; inbox of questions and approvals; pause and resume; Needs-you state. | Section 5. Patterns first; copying code needs their notices kept. |
| Cost research | Caching is the biggest saving; route per session, not per turn. | Stable prefix, routing per session, compress tool output. |

## 3. What is new in ours

1. The AI decides whether to ask, with its assumptions written down and shown with the plan.
2. A Design step for UI work, approved before any code, whose approved design can become a check.
3. A dependency-aware queue: tickets touching the same files wait.
4. Risk tier by rules (protected paths, diff size, labels), never by the model's opinion.
5. A review agent that never saw the author's context, run last, only when every cheaper check passed.
6. A cost ledger and caps, per ticket and per day.
7. A replay benchmark from the project's own history (section 9).

## 4. Pipeline

`Intake -> Understand -> [Clarify] -> [Design] -> Plan -> Build <-> Check -> Review -> PR`
Brackets: the step runs only when the AI (or a rule) says it is needed. Side states: `Needs you`, `Queued` (file conflict),
`Paused`, `Failed` (budget, loop, checks).

| Step | Done by | Output | Asks you |
|---|---|---|---|
| Intake | Rust | Normalised ticket; duplicates flagged | no |
| Understand | agent, cheap tier | Restated need, files touched, assumptions, `open_questions`, `needs_design` | no |
| Clarify | agent + you | Batched questions, each with a proposed default | only when it has open questions (the AI decides) |
| Design | agent + you | A local HTML mockup or an enabled remote canvas, revised until approved | yes: "Ask me after this step" is on by default |
| Plan | agent, top tier | Acceptance criteria (each with how it is verified), test plan, file list | on by default for standard and risky tickets |
| Build | worker agent in a worktree | Worktree changes, committed by the trusted Git path after Check | no |
| Check | Rust, then one agent | Ordered checks (section 5) | no |
| PR | Rust | Draft PR with a description based on the plan and check results | approve the exact checked head before publishing |

### Asking and confirming (there are no gates)
- There is no gate step and nothing to set up as a gate. A step asks you in the inbox whenever it needs a confirmation or more information,
  and that ticket waits as `Needs you`. Other tickets keep running.
- A step asks in two ways:
  1. **By itself**: it reports an open question, an assumption it cannot settle, a design to approve or a risky change.
  2. **By your checkbox**: every step has **"Ask me after this step"**. When ticked, the step shows its output and waits for
     Approve, Edit or Reject before the next step starts.
- **The AI decides whether to ask.** Understand returns structured output (`--json-schema`): `open_questions`, `assumptions`, `needs_design`, `ui_files`.
  No open questions means Clarify does not run and does not ask. A per-project setting picks what happens when a step is unsure:
  "ask me" or "decide and tell me" (like craft-anything's "Ask me / Decide yourself").
- **Nothing is hidden**: decisions the AI made without asking are listed as assumptions and shown with the plan.
- **Rules override the model and the checkbox**: a protected path, a risky ticket, a security label or an exceeded budget makes the step ask even
  when its box is off. The node shows why ("asks anyway: protected path"). The model cannot switch this off.
- **Defaults**: the box is on for Design, Plan and PR, and off for the rest. Trivial tickets may skip optional confirmations if the project allows it. Design approval and publishing approval are required when those actions run.
- **One inbox** for all of it, with craft-anything's two request kinds: a question (choices, optional free text) and an approval.

### Design step

- Runs only when `needs_design` is true. The first M6 path is a static HTML mockup in the worktree. Its viewer disables scripts and external requests.
- Review the design in the inbox. Approval is required whenever the step runs. The approved design is attached to the Plan and becomes the reference for Build.
- A remote Claude Design canvas is a per-project opt-in. Claude Code documents an eligible claude.ai login for publishing and revisions to the same URL. The [S4 live trial](spikes/S4-claude-design-headless.md#method) must verify headless publication, revisions and audience before M6 can enable it. The asset is hosted by Anthropic.
- A later visual check could compare the built UI with the approved design.

## 5. Build and Check (from craft-anything, without Study)

Their loop: preflight (agent tools and sign-in) -> Build turn -> Check (round += 1) -> Refine (next build turn with the failed note) -> Ready.
Checks are code, not an agent. A check has an id, `applies`, `can_refine` and `run`, and returns `passed`, `note`, `artifacts`, `unknowns`
(what it could not verify "can't pass yet"). The run stops at the first failed check. Limits: a round cap, a not-working counter that stops
before paying for another build turn, a result kept when the tree fingerprint is unchanged, turn retries with backoff, pause is not a failure.
Unverified (read only in part): whether their "good enough" threshold is used by the loop; how the plan list updates for CLI agents.

Our check order, cheap to expensive, stop at the first failure:
1. Build, fmt, clippy (from the project profile).
2. Tests.
3. Protected-path scan and diff size.
4. Acceptance criteria from the plan: each has a verification (test, command, or `unknown`); `unknown` cannot pass.
5. Agent review in a fresh context, last, only if everything above passed.

Refine: the failed note goes back to the same session. Default 2 to 3 rounds (theirs is 6; ours costs money). A stuck run becomes `Needs you`.
Questions and approvals are small request and answer files in an owner-only folder, so the CLI and the window behave the same.

## 6. Architecture (Rust workspace, layered, checked by an xtask)

```
L0  types        ids, ticket, event, money, error
L1  store        append-only event log + snapshots (SQLite); resume after a crash
L1  profile      project profile (steps, confirmations, checks, protected paths, budgets) + AGENTS.md/CLAUDE.md reader
L1  process      bounded subprocess lifecycle and platform supervision
L2  git          worktrees, branches, diff, conflict prediction (git CLI)
L2  tracker      GitHub issues and PRs (gh CLI or API)
L2  policy       sandbox x approval axes, risk tier, forced confirmations, budget, loop detector
L3  agent        provider-neutral Agent trait and events
L3  checks       Check trait, ordered runner, kept results
L4  adapters     Claude Code headless first, Codex CLI next; declared grants per adapter
L4  pipeline     step machine, scheduler (queue, parallelism), cost ledger, inbox
L5  engine       tool definitions once -> CLI command, MCP tool, UI action
L6  cli, mcp, ui-egui     thin clients; the UI never runs programs or writes files itself
```

This is the layering as designed. The crates that exist and their layers are in [the plan, section 5](IMPLEMENTATION-PLAN.md#5-repository-layout-apache-20-repo-autopilot-mode-on) and are enforced by `cargo xtask layers` (`tools/xtask/src/layers.rs`). Use the process boundary and containment rules in the implementation plan. The agent adapters, check runner, Git and tracker each own their launch policy. The CLI, desktop and MCP share engine actions.

## 7. Auth and cost

- Each step has `auth = "api" | "subscription"`. The tool launches `claude` and sets the environment per child process. API: the key from a
  dedicated variable (`AUTOPILOT_ANTHROPIC_API_KEY`; the ordinary `ANTHROPIC_API_KEY` is never read) is passed as `ANTHROPIC_API_KEY` to the child. Subscription: the tool strips
  `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `CLAUDE_CODE_USE_*` from the child, because a key outranks the login. A dedicated key may be passed to a verified provider-only environment. Keep credentials out of storage, logs and repository tool processes.
- Real Build and Check run only inside the validated sandbox and only in API mode (subscription mode there is refused; see [security](security.md)).
- API steps get dollar caps (`--max-budget-usd`, a Console workspace limit). Subscription steps get turn caps and an estimate.
- Caches are per account and model: keep a session's prefix on one auth; start each step as a fresh session and pass the previous output as a file.
- Cheap steps (intake, understand, PR text) on the API at cents per ticket; heavy steps (plan, build, review) on the subscription.
- Rough estimate: about $2 per standard ticket on all-API with mixed tiers (my assumptions, not measured). M1 prints real cost per ticket.
- Unverified: how a plan limit shows up in the output (needed for `fallback = "api"`); whether `--resume` works across auth modes.
- Model levers: model per step and per session, never per turn; stable prefix; deterministic work in Rust; compress tool output; loop detector.

## 7a. Agents and subscriptions (Claude Code and Codex)

- Every step chooses an **agent** (`claude` or `codex`) and an **auth**. The schema records the preferred mode for each provider.
  - claude: `subscription` (default) or `api` key.
  - codex: `chatgpt` (sign in with a ChatGPT plan, the subscription route) or `api` key. S8 gates the enabled modes and their defaults.
- The tool launches the user's own `claude` or `codex`. Use provider-managed sign-in and isolate any API key to the verified provider-only environment. The preflight commands are `claude auth status` and `codex login status`. Codex status reports credential presence.
- The [S8 preflight](spikes/S8-codex-auth.md) verified logged-out status in fresh isolated file stores on Windows. OpenAI documents `CODEX_HOME` for file-based credentials and `CODEX_API_KEY` for `codex exec`. API authentication is the candidate for unattended execution. S8 still needs to confirm when the ChatGPT route can be used and whether switching modes preserves isolation.
- **Auth rule per mode (S8, 2026-11-25).**
  - `codex login status` is the preflight in every mode: exit 0 means a login is stored (the text names the mode, for example `Logged in using ChatGPT`), exit 1 means not logged in. It does not prove a request will pass.
  - `api`: the only candidate for unattended steps. A dedicated OpenAI key, from a dedicated variable we read and never from `OPENAI_API_KEY`, goes to the child as `CODEX_API_KEY` only, with a Platform spend limit on it. The child gets its own empty `CODEX_HOME` with the file store, deleted afterwards, so no cached login can outrank the key.
  - `chatgpt`: sign-in is left to the owner in the Codex app; we never run `codex login`, never read or copy `auth.json`. It stays off for unattended steps until OpenAI's terms are confirmed for this use (S8 terms assessment). A launch must still give it its own home or the owner's, never one shared with an API run.
  - Observed, logged out: `exec --json` starts a thread, then retries five times against `api.openai.com/v1/responses` with 401 and ends in an error, not an `AgentEvent` result. The adapter must treat a non-JSON line as noise and check `login status` first.
- **Decision (A3): no-go for the adapter for now.** Everything that decides the event mapping, cost reporting, limit detection and sandbox behaviour needs an authenticated run, and none was possible. The owner must: (1) create a dedicated OpenAI API key with a Platform spend limit and say so (the live cap stays $0.10 for this story); (2) decide whether the ChatGPT route may ever run unattended, after reading the terms; (3) accept that Codex's own sandbox is not ours, so the Codex role in the S6 boundary (network to OpenAI only) comes from stream 3. With (1) the checks can be finished in one short session and this becomes go or no-go on evidence.
- OpenAI documents `codex exec --json` events for threads, turns, items and errors. M8 maps observed events to `AgentEvent` after S8 captures fixtures.
- Test approval, sandbox and network behavior with the intended launch settings before enabling real repository access. The pipeline asks for user input at step boundaries.

- M9 can measure whether a different model family improves independent review.
- The profile schema has `agent` and `auth` per step from M0, so adding the Codex adapter later changes no file format.

### Decision: the Claude Agent SDK (A12, 2026-11-25): defer, do not build

Question: would the Claude Agent SDK (TypeScript or Python) replace or sit beside `agent-claude` for sessions, hooks, permission callbacks and cost data?

**Defer.** Keep the CLI adapter. Reasons, each checked against the code except where marked:

- **Nothing we lack.** The adapter already gets sessions (`--session-id`, `--resume`), a deny hook (`--settings`), a permission mode, structured output and cost from the stream-JSON result (`crates/agent-claude/src/command.rs`). The SDK's callbacks would replace flags with in-process code, not add a capability. The one thing the CLI flags do not give us, answering a permission request from our inbox, is A11 (`--permission-prompt-tool` served by `autopilot-mcp`); decide again only if A11 shows the CLI cannot do it.
- **A second runtime.** The host is Rust. The SDK needs Node or Python on the owner's PC and a sidecar process we would have to launch, supervise and bridge to the engine, plus the same inside the sandbox for Build. Every extra runtime is more to install, pin and probe.
- **The boundary.** The sandbox launches one pinned program under `srt` or WSL with a validated environment, a hook the agent cannot overwrite and a placeholder key (S6). A sidecar that itself spawns the agent would put our launch policy behind someone else's code, and `cargo xtask layers` only lets `process` run subprocesses.
- **Subscription sign-in.** Subscription is our default auth (section 7). From memory and not re-checked today: Anthropic's SDK terms do not let a product offer claude.ai sign-in for its users, which would not fit that default. Re-read the SDK's current terms and licence before any reversal.
- **Cost.** More moving parts for no saving: the cost figures in the CLI result are what the SDK reports too (from memory).

Revisit when: A11 fails, the CLI drops a flag we rely on, or an SDK feature we need has no CLI equivalent.

## 8. Window (egui) and the Claude Design canvas

A node graph where each step shows its settings, with a profile file as the source of truth. Simple view: four steps (Plan, Build, Check, Ready);
Advanced view: the full graph. Every step has an "Ask me after this step" checkbox; a step that is asking is amber. Style follows craft-anything's screens: dark shell, ticket sidebar with
status chips, amber approval cards, progress stepper, cost bar. See [the window page](ui.md) for what the window shows today.

## 8a. Live status of every ticket

Requirement: for every ticket you can watch its pipeline run in real time.

- **One source of truth:** an append-only event log per ticket (`store`). Events: `StepStarted`, `AgentActivity` (a tool call or a line of output,
  truncated and secret-redacted), `RoundStarted`, `CheckResult`, `Asked`, `Answered`, `StepSkipped` (with the reason), `CostTick`,
  `Paused`, `Failed`, `Done`. `engine::follow` replays from a sequence number and then polls the store, so the window, the CLI (`autopilot watch --follow`) and a writer in another process all see the same events.
  A subscriber that connects late replays the log, so reopening the window shows the full history. The current state is a fold over the events,
  so after a crash or restart the view is still correct and a run that was cut off shows as `Interrupted`.
- **Where the events come from:** the agent's `stream-json` output (turns, tool calls, token usage), the check runner, the inbox and the scheduler.
- **Per ticket view:** every node shows its state: idle, queued (with the file conflict), running (elapsed time and what it is doing right now,
  for example "editing routes.rs" or "running cargo test"), waiting for you, passed, failed, skipped (with the reason, for example "no open
  questions"), paused. The Build and Check node shows "round 2 of 3" and its check list ticking through running, passed and failed.
  Live counters per node: time, turns, tokens, cache-hit rate, cost (exact for API steps, an estimate for subscription steps).
  A drawer shows the timeline and the raw agent console. A ticket-wide cost bar shows spend against the cap.
- **Sidebar:** every ticket with a status chip that updates live (running, needs you, queued, CI failing, interrupted, done).
- **Attention:** a desktop notification and a chip when a ticket needs you, fails or finishes; a "stalled" flag when a running step sends no event
  for a set time (configurable per step).
- **Headless:** `autopilot watch <ticket>` replays the same events (add `--follow` to keep tailing); `autopilot status <ticket> --json` prints the folded state.
- **Rendering:** repaint when an event arrives, not every frame. craft-anything's pitfalls note that egui's Spinner and ProgressBar request a
  repaint every frame, so we use paced widgets.
- **Estimates:** time left is the step's average from past runs, labelled as an estimate.

## 8b. Step details, actions and change requests

Built so far: the step page and its tabs, re-run for Understand, Design and Plan, and "Ask for a change" through a fixed grammar. Shown off with the reason: Open in a terminal and re-running a finished Build ([ui](ui.md), stream 4 task W4).

- **Click any step** (on the graph, the live strip or the sidebar's ticket) to open its detail. Tabs: Overview (why it ran or was skipped, who decided,
  rounds), Input (what it received), Prompt (the exact prompt and project rules, secrets removed), Output (its structured result), Log (agent console),
  Changes (the diff), Cost (per turn), Settings (the same fields as the node). Everything is read from the ticket's event log.
- **Actions on a step:** Re-run this step; Re-run from here (discards later results, asks first); Pause after this step; Stop this step;
  "Ask me after this step, next time"; Open the worktree; Open in a terminal (resumes the agent session); Copy the local session reference.
  A skipped step has "Run it anyway". A step that is asking has Approve, Edit and Reject. Destructive actions always confirm.
- **"Ask for a change" has three targets**, chosen automatically from the message and always shown so you can flip it:
  This ticket's code (feedback to the build agent), The pipeline, Just ask (a question, nothing changes).
- **Changing the pipeline in plain words** ("use sonnet for the review", "skip design for this ticket"):
  an agent turns the message into a typed patch of the profile (`--json-schema`), the engine validates it against the profile schema and shows it as a diff.
  Nothing applies until you press Apply. You choose the scope: this ticket only (stored as a per-ticket override event, so history stays auditable)
  or all future tickets (written to profile.toml). It takes effect when the next step starts; a running step is never touched.
  Rule-based confirmations and spending limits cannot be loosened this way; the patch is rejected with the reason.

## 8c. Summaries: free by default, AI only on request

- **Every step shows an "In short" line made by code** from its own results, for example "Found 2 files to change. No questions." or
  "Round 2 of 3. 1 of 5 checks passed." These are templates filled from the structured result and the check runner's counts, so they cost no tokens
  and cannot be wrong about the numbers. They appear on the node, in the live strip, at the top of the step detail and in the PR description.
- **A plain-words explanation by a model is opt-in.** The step detail has "Explain in plain words". It sends the step's output to a small model once
  (usually under a cent), labels the text "written by the agent" and keeps it in the event log, so it is never paid for twice. Never automatic,
  unless a project turns on "always explain" for chosen steps.
- No extra summary field is added to the agents' structured output by default, so the working steps spend nothing on summaries.
- A ticket-level summary follows the same rule: a code-made line (steps done, rounds, spend) and an on-request explanation.

## 8d. Codebase review and fix

A separate run reviews a scoped repository snapshot against sourced rules. It returns evidence-linked findings with independent verification. Report-only is the default. Approved fixes become small tickets and go through the existing checks and publication approval. [Review design](REVIEW-PIPELINE.md) · [Rule packs](rules/README.md)

## 9. Evaluation

Replay tickets from a project's own history: take merged PRs with their issue text, run the pipeline from the base commit, compare by the tests the
PR added and a rubric. Record resolve rate, dollars, wall time, questions asked, questions skipped wrongly. First corpus: ck-* PRs.

## 10. Milestones

See the [implementation order](IMPLEMENTATION-PLAN.md#0a-implementation-order-and-deliverables) for the executable roadmap and milestone dependencies.

## 11. Before M0 (superseded: see IMPLEMENTATION-PLAN.md, section 3, spikes S1 to S10)

- Read Gru and oven-cli; decide contribute or build.
- Read Codex CLI's source for the sandbox and approval design.
- Test headless: `--json-schema`, `--max-budget-usd`, `--permission-prompt-tool`, `--disallowedTools`, resume across auth modes, a plan-limit error, and Artifact publishing.
