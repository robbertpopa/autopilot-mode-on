# M8: Codex adapter, MCP server, packaging

**Owner crates:** `agent-codex`, `mcp`, packaging. **Depends on:** M1 (adapter trait stable) and S8 for Codex; the MCP server needs `engine` from M5. The Codex adapter can start in its own tab as soon as M1 and S8 are done.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M8-codex-mcp-packaging.md`."

## Goal
Use Codex as an agent next to Claude Code, on a ChatGPT subscription by default or an API key, expose the engine over MCP, and ship installable builds.

## Tasks
- [ ] Read the S8 result and write the auth rule per mode (subscription, API key) into `docs/DESIGN.md` section 7a.
- [ ] `agent-codex`: command builder for `codex exec --json` (sandbox mode, network off by default, working directory, output schema if available), parse `thread.started`, `turn.*`, `item.*` (agent message, command execution, file change, MCP tool call, todo list) and `usage` (input, cached, cache write, output, reasoning) into the normalised `AgentEvent`.
- [ ] Per-step auth for Codex: environment and config-directory scrubbing per S8 so a subscription step never sees an API key and the cached login never interferes; preflight sign-in check.
- [ ] Codex steps run in a sandbox mode (read-only or workspace-write); `codex exec` rejects approval requests, so "Ask me after this step" works at step boundaries for Codex.
- [ ] Cost: token counts and a Codex price table for API mode; subscription steps show tokens and an estimate labelled as such.
- [ ] `agent` field in the profile selects `claude` or `codex` per step; the window shows it on each node. A different-family reviewer is available as a check option.
- [ ] `mcp` crate: expose the `engine` actions as MCP tools; refuse to start without explicit read and write roots; refuse file access outside them.
- [ ] GitHub App authentication option for `tracker`.
- [ ] Packaging: Windows installer, `cargo install`, release workflow, update notes.

## Tests that must exist
- Fixtures for `codex exec --json` parse; hostile sweep; unknown item types ignored.
- A secret in the parent environment never reaches a Codex subscription child; an API key never reaches a step not set to API.
- The same pipeline test passes with the fake agent configured as Claude and as Codex (agent-independent pipeline).
- The MCP server refuses paths outside its roots.

## Exit criteria
One ticket runs with Claude for Build and Codex for review, each on its subscription, and the MCP server answers a status call.
