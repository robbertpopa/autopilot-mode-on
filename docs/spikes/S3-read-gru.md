# S3: what we can learn from Gru

**Question:** which parts of Gru's design and code do we copy, and which pitfalls did it already hit? **Gates:** M4 design. No spending, no product code.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S3-read-gru.md`."

Repository: github.com/fotoetienne/gru (Apache-2.0, Rust, active; read `CLAUDE.md`, `docs/DESIGN.md`, then the source).

## Read and note
- [ ] How the agent is launched (`--session-id <UUID>` pre-assigned, stream-json, partial messages) and how output is read; where it kills a stuck stream (15 minutes) and how.
- [ ] The worktree layout (bare mirror plus checkout) and cleanup; what happens on crash.
- [ ] The CI-fix loop: what is fed back to the agent, the cap of 2, how a failing log is trimmed.
- [ ] Registry and state handling (JSON registry, GitHub labels as source of truth); what we do differently with the event log.
- [ ] Per-session token and cost tracking.
- [ ] Their tests: how they fake the agent and `gh`.
- [ ] Their pitfalls recorded in `CLAUDE.md`, and the mismatches between old design text and current docs.

## Done when
`docs/pitfalls.md` has the new entries, and a list of at most 3 ideas to copy and at most 3 things to avoid is added to `docs/milestones/M4-worktrees-pr.md`. Do not copy code without keeping their notices.

## Result
(fill in)
