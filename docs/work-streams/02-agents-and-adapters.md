<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Stream 2: agents and adapters

**Owns:** `agent`, `agent-claude`, `agent-fixture`, new `agent-codex`, `mcp`, `tools/fake-agent`, `fixtures/`, and the spike files S2 and S8. **Does not touch:** `engine`, `ui`, `process` (ask in the other streams' files). **Start prompt:** "Read `AGENTS.md`, `docs/work-streams/00-overview.md` and `docs/work-streams/02-agents-and-adapters.md`, then do the first unchecked task."

## Goal

More than one real agent behind one interface, with the same tests holding every adapter to the same behaviour, so adding an agent is a matter of writing an adapter and passing the suite, not rewriting the engine.

The idea comes from the open-source Craft Agents app: one `AgentBackend` interface with a provider-neutral event stream, per-provider drivers for credentials and model lists, and `capabilities()` so the UI asks what an agent can do instead of assuming. Take the idea, not the code.

## Where it stands

- `agent::Agent` has `start` (Understand), `start_build`, `start_design`, `start_plan`, `start_custom`, `start_draft`, and `start_review` (added by PR 140; check it is merged); every method but `start` has a default that refuses, so an adapter implements only what it supports. Events: `AgentEvent` (activity, session, usage, cost, results, stop, failure).
- Two adapters: `agent-claude` (Claude Code CLI, headless, stream-JSON, structured output, per-step tool lists, API-key and subscription modes) and `agent-fixture` (canned answers for the practice agent). `tools/fake-agent` replays recorded Claude streams for tests (`fixtures/claude/`; the synthetic review fixtures arrive with PR 140).
- A real Claude Build runs only in the sandbox (stream 3 owns the boundary). The adapter builds the sandboxed turn's environment from nothing (`sandbox_env`) and passes the deny hook through `--settings`.
- `crates/mcp` exists with a binary `autopilot-mcp`; its status is not recorded here: read it before you plan around it.
- S1 (headless contract) is done; S2 (plan-limit detection) and S8 (Codex auth and terms) are not: [S2](../spikes/S2-plan-limits.md), [S8](../spikes/S8-codex-auth.md). S8 has a documentation and logged-out preflight but no live evidence.

## Tasks, in order

- [ ] **A1. A conformance suite.** Behaviour every adapter must show, written once as a reusable test helper in the `agent` crate (for example `agent::conformance::run(&dyn Agent, &Fixtures)`): a run ends in exactly one result or one stop or one failure; events arrive in order and a stream that ends without a result is an error; an unsupported method returns the "unsupported" error rather than panicking; unknown events and fields are kept and ignored, never an error; a cancelled run ends promptly; cost and usage are reported once and never go backwards. Run it against `agent-claude` (with the fake program) and `agent-fixture`. *Exit:* both pass the same suite, and a deliberately broken test adapter fails each rule (mutation check).
- [ ] **A2. Surface the intent of each action.** Check whether Claude Code's Bash tool input carries a `description` in the recorded fixtures (`fixtures/claude/live-2.1.330-*.jsonl`). If so, carry it into `AgentActivity` with the same bounding and secret redaction as other agent text, so the history says why each command ran. If not, record that and stop; do not patch the model's requests. *Exit:* a fixture test and a hostile-input case (very long, control characters).
- [ ] **A3. Finish S8, decide Codex.** Run the live checks the S8 file lists (logged-in and logged-out `codex` status, what events `codex exec --json` emits, what a subscription run reports for cost and limits), within the spike budget, and write the result and the auth rule per mode into `docs/DESIGN.md` section 7a. Codex runs in the same sandbox as Claude, so check that it works under the boundary (network to its provider only; stream 3 adds the role). *Exit:* S8 ticked with evidence, and a go or no-go for the adapter.
- [ ] **A4. The Codex adapter (`agent-codex`).** Only if A3 says go. Implement Understand, Plan and Build; map Codex events onto `AgentEvent`; no credentials in our storage; pass the A1 suite; add `fixtures/codex/` recordings and a fake program for tests. Do not add `capabilities()` before this adapter exists: an interface with one implementation is an abstraction AGENTS.md forbids. *Exit:* the suite passes for two real adapters; one live Build with Codex inside the sandbox within the spend cap.
- [ ] **A5. `capabilities()`.** Now that there are two adapters, add what the window and the engine actually branch on (which steps, whether it reports cost, whether it can resume a session, which auth modes). Only add a capability something reads. *Exit:* the engine and window read it instead of matching on the adapter's name.
- [ ] **A6. Provider drivers.** Credential routing and model discovery per provider, in the adapter, never in our storage: the key comes from the user environment or the provider's own sign-in, and the sandboxed turn gets only the masked placeholder (see stream 3). *Exit:* a test that no adapter reads an ambient key by accident.
- [ ] **A7. Other agents, one at a time.** For each candidate the owner names (OpenCode and others; see owner decision 3): read its repository and licence first, write a short spike file like the S-files (headless contract, auth, events, cost reporting, terms of use for unattended use), and only then an adapter. Never copy code with an incompatible licence, and do not use leaked sources. *Exit per agent:* a spike file with a decision, then A1 passing.
- [ ] **A8. MCP server.** Read `crates/mcp` and `docs/milestones/M8-codex-mcp-packaging.md`. Expose the engine's read-only views first (tickets, status, history), then actions that go through the same approvals as the window; never an action that skips a forced approval. *Exit:* a protocol test over stdio and a test that an MCP client cannot approve a forced request on the owner's behalf.
- [ ] **A9. S2: plan limits.** Detect a subscription limit from a real run and classify it. This needs real subscription evidence and about $0 of API spend; the sandbox does not run subscription mode, so use the host steps (Understand, Plan). *Exit:* S2 ticked with a recorded limit event and the classifier test over it.
- [ ] **A10. Live tests.** Keep each `#[ignore]` behind the `live` feature, at most $0.05 each, never in a normal run.

## Risks and decisions

- Subscription mode inside the sandbox is refused on purpose (S6): do not add it to an adapter.
- Every adapter that can build must accept only a validated boundary from the engine (`Isolation`), exactly like Claude. A real agent with shell tools outside the boundary is a defect.
- Unknown fields and message types from an agent are kept and ignored, never an error. A parser never panics on any input: add the input that breaks one to its hostile-input sweep.

## Requests to other streams

- (none yet)
