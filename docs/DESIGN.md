# autopilot-mode-on: design

Local Rust program (repo and crate name `autopilot-mode-on`). Input: a codebase plus stories, bugs and wishes. It clarifies the need in plain language (only when it
has something to ask), designs the UI when there is UI, plans, builds in isolated worktrees, checks, and opens PRs.
Tested first on Contest Keeper, built for any project. Not a repo yet: this is the scratch to create the open-source repo from.

Status: draft 2026-11-20. Survey numbers come from web snippets and blogs and are indicative, not verified.
Visual design: Claude Design canvas "Pipeline Canvas" (private artifact); style follows craft-anything's screens.

## 1. Decided and open

Decided by the user:
- Local program in Rust, egui window, CLI and engine first (the window is a thin client). Licence Apache-2.0.
- Workers are existing agents, Claude Code headless first. No own coding agent.
- Per-step auth: each step uses a subscription (default) or an API key, for Claude Code and for Codex. A `$20` budget for tests.
- Build and Check come from craft-anything's loop (build, check, refine, ready). **No Study step.**
- No gates. A step asks you whenever it needs a confirmation or more information, and every step has an "Ask me after this step" checkbox.
  The AI decides whether it has anything to clarify.
- A Design step uses Claude Design and needs approval.

Decided later the same day: the name is `autopilot-mode-on`; **subscription is the default auth**; Codex can be used too (section 7a).
Still open: where the repo lives (own account recommended) and the real-world behaviour of plan limits (spike S2).

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
| Design | agent using Claude Design + you | A design canvas link, revised until approved | yes: "Ask me after this step" is on by default |
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

### Design step (Claude Design)
- Runs only when `needs_design` is true (a user-facing UI change). The agent makes a design canvas with Claude Design
  (the Design artifact type) and returns its link. You open it, then approve, ask for changes or reject in the question inbox.
  Feedback goes back to the same canvas, so a revision keeps its link.
- Approval is required whenever the step runs. The approved design is attached to the Plan and becomes the reference for Build.
- Later idea (borrowed from craft-anything's website check): a visual check that compares the built UI with the approved design.
- Unverified: publishing artifacts from a headless `claude -p` session, and whether it needs a claude.ai login (subscription) rather than an API key.
  Test the remote path in S4. M6 can start with static HTML mockups from the worktree and a viewer with scripts and external requests disabled.
- Privacy: a design canvas lives on claude.ai under your account. Private repos may not want that; make the step switchable per project.

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

Use the process boundary and containment rules in the implementation plan. The agent adapters, check runner, Git and tracker each own their launch policy. The CLI, desktop and MCP share engine actions.

## 7. Auth and cost

- Each step has `auth = "api" | "subscription"`. The tool launches `claude` and sets the environment per child process. API: the key from a
  dedicated variable (for example `CK_PIPELINE_ANTHROPIC_KEY`) is passed as `ANTHROPIC_API_KEY`. Subscription: the tool strips
  `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `CLAUDE_CODE_USE_*` from the child, because a key outranks the login. A dedicated key may be passed to a verified provider-only environment. Keep credentials out of storage, logs and repository tool processes.
- API steps get dollar caps (`--max-budget-usd`, a Console workspace limit). Subscription steps get turn caps and an estimate.
- Caches are per account and model: keep a session's prefix on one auth; start each step as a fresh session and pass the previous output as a file.
- Cheap steps (intake, understand, PR text) on the API at cents per ticket; heavy steps (plan, build, review) on the subscription.
- Rough estimate: about $2 per standard ticket on all-API with mixed tiers (my assumptions, not measured). M1 prints real cost per ticket.
- Unverified: how a plan limit shows up in the output (needed for `fallback = "api"`); whether `--resume` works across auth modes.
- Model levers: model per step and per session, never per turn; stable prefix; deterministic work in Rust; compress tool output; loop detector.

## 7a. Agents and subscriptions (Claude Code and Codex)

- Every step chooses an **agent** (`claude` or `codex`) and an **auth**. **Subscription is the default** for both.
  - claude: `subscription` (default) or `api` key.
  - codex: `chatgpt` (sign in with a ChatGPT plan, the subscription route; default) or `api` key.
- The tool launches the user's own `claude` or `codex`. Use provider-managed sign-in by default and isolate any API key to the verified provider-only environment. Preflight checks sign-in
  (`claude auth status`; the Codex equivalent is to be confirmed in spike S8).
- Codex facts (search of OpenAI's auth pages, 2026-11-20): two sign-in methods exist, ChatGPT sign-in (plan credits) and an API key (standard API pricing);
  OpenAI's page recommends the API key for scripted and CI use; a key is passed through `codex login --with-api-key` on stdin, not as an argument; the two methods do not switch
  cleanly because the login is cached. A third-party guide says `CODEX_API_KEY` works only with `codex exec`; official pages did not mention it. **All of this is unverified until S8.**
- Consequences for the design:
  - Mixing subscription and API steps for Codex needs separate Codex config directories per auth mode, or a logout between steps (S8 decides).
  - `codex exec` never prompts: approval requests are rejected. Codex steps therefore run in a sandbox mode (read-only or workspace-write, network off) and
    "Ask me after this step" works at step boundaries, not in the middle of a tool call.
  - `codex exec --json` events (thread, turn, item, usage with cached tokens) map onto our normalised `AgentEvent`, so the pipeline never sees which agent ran.
  - A natural use: a different model family as the independent reviewer in the check list. This is an idea to test in the evaluation, not a known gain.
  - Whether a ChatGPT subscription may be driven by a local tool is a terms question; S8 reads OpenAI's terms. The same rule as for Claude applies: the user signs in, we never touch credentials.
- The profile schema has `agent` and `auth` per step from M0, so adding the Codex adapter later changes no file format.

## 8. Window (egui) and the Claude Design canvas

A node graph where each step shows its settings, with a profile file as the source of truth. Simple view: four steps (Plan, Build, Check, Ready);
Advanced view: the full graph. Every step has an "Ask me after this step" checkbox; a step that is asking is amber. Style follows craft-anything's screens: dark shell, ticket sidebar with
status chips, amber approval cards, progress stepper, cost bar. See [the UI reference](ui.md) for the local export.
Pending canvas changes: remove the three gate nodes, add the "Ask me after this step" checkbox to every step, Build and Check as one looping node, a Design node, Simple and Advanced views.

## 8a. Live status of every ticket

Requirement: for every ticket you can watch its pipeline run in real time.

- **One source of truth:** an append-only event log per ticket (`store`). Events: `StepStarted`, `AgentActivity` (a tool call or a line of output,
  truncated and secret-redacted), `RoundStarted`, `CheckResult`, `Asked`, `Answered`, `StepSkipped` (with the reason), `CostTick`,
  `Paused`, `Failed`, `Done`. The engine publishes each event on a broadcast channel; the window, the CLI and the MCP server all subscribe.
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
- **Headless:** `pipeline watch <ticket>` tails the same events; `pipeline status --json` prints the folded state.
- **Rendering:** repaint when an event arrives, not every frame. craft-anything's pitfalls note that egui's Spinner and ProgressBar request a
  repaint every frame, so we use paced widgets.
- **Estimates:** time left is the step's average from past runs, labelled as an estimate.

## 8b. Step details, actions and change requests

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

## 9. Evaluation

Replay tickets from a project's own history: take merged PRs with their issue text, run the pipeline from the base commit, compare by the tests the
PR added and a rubric. Record resolve rate, dollars, wall time, questions asked, questions skipped wrongly. First corpus: ck-* PRs.

## 10. Milestones

See the [implementation order](IMPLEMENTATION-PLAN.md#0a-implementation-order-and-deliverables) for the executable roadmap and milestone dependencies.

## 11. Before M0 (superseded: see IMPLEMENTATION-PLAN.md, section 3, spikes S1 to S8)

- Read Gru and oven-cli; decide contribute or build.
- Read Codex CLI's source for the sandbox and approval design.
- Test headless: `--json-schema`, `--max-budget-usd`, `--permission-prompt-tool`, `--disallowedTools`, resume across auth modes, a plan-limit error, and Artifact publishing.
