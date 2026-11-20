<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# S3: what we can learn from Gru

**Question:** which parts of Gru's design and code do we copy, and which pitfalls did it already hit? **Gates:** M4 design. No spending, no product code.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S3-read-gru.md`."

Repository: github.com/fotoetienne/gru (Apache-2.0, Rust, active; read `CLAUDE.md`, `docs/DESIGN.md`, then the source).

## Read and note
- [x] Agent launch, stream framing and stuck-process handling.
- [x] Worktree layout and crash cleanup.
- [x] CI repair input, retry cap and log trimming.
- [x] Registry and GitHub state handling.
- [x] Per-session token and cost tracking.
- [x] Agent and GitHub test coverage.
- [x] Pitfalls and discrepancies between design notes and source.

## Done when
`docs/pitfalls.md` has the new entries, and a list of at most 3 ideas to copy and at most 3 things to avoid is added to `docs/milestones/M4-worktrees-pr.md`. Do not copy code without keeping their notices.

## Result
Reviewed Gru at commit [`3e6823b`](https://github.com/fotoetienne/gru/commit/3e6823b1035853c570e3e45c7f1f0a51e9de1546) on 2026-11-21. This was a source review.

- **Launch and timeouts.** Gru creates the worktree and allocates a UUID before the Claude launch. The launcher requests stream JSON and partial messages. Its runner reads stdout line by line and kills and waits for the child on a stream error. The 15-minute inactivity check and five-minute timeout on each read are separate. A fully silent child can hit the five-minute read timeout first. [Worktree setup](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/commands/fix/worktree.rs#L54-L92) [Claude launcher](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/claude_runner.rs#L11-L47) [Read deadline](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/agent_runner.rs#L270-L334) [Process exit](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/agent_runner.rs#L375-L395)
- **Worktrees and crash recovery.** Gru keeps a bare repository under `repos/<owner>/<repo>.git`, a checkout under the minion directory, and logs alongside the checkout. It registers the minion after creating the worktree, which leaves a crash window between those operations. Cleanup inventories Git worktrees and stopped registry entries. It canonicalizes paths before deleting directories and can remove a stale registry entry while leaving an external directory alone. [Worktree setup](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/commands/fix/worktree.rs#L34-L110) [Cleanup scan](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/commands/clean.rs#L361-L489) [Orphan cleanup](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/commands/clean.rs#L954-L993)
- **CI repair.** The loop polls check runs by commit and allows up to two repair attempts. The prompt includes failed check details and diagnostic output. It keeps the beginning and end of long logs with an omitted-byte marker, using a 10,000-byte input budget per failing check before that marker. It compares old and new heads to detect repairs that produced no commit. The design document's older 10–15 lifecycle attempts are separate from the implemented CI repair cap. [CI constants](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/ci.rs#L13-L32) [Log trimming](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/ci.rs#L316-L334) [Repair prompt](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/ci.rs#L512-L560) [CI loop](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/ci.rs#L1166-L1257) [Old design](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/docs/DESIGN.md#L239-L249)
- **State and usage.** GitHub labels and PRs coordinate remote work. The local JSON registry uses locking and a synced temp file followed by a rename. It records the minion's process start time for PID reuse detection and stores optional token totals. The runner accumulates input, output, optional cache tokens and backend-reported cost, with an EOF fallback when a backend omits its terminal usage. Our SQLite history owns approvals, attempts and results. GitHub is an external source of issue and check data. [Registry](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/minion_registry.rs#L612-L643) [Atomic save](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/minion_registry.rs#L1023-L1060) [Usage handling](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/agent_runner.rs#L354-L433) [Usage persistence](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/commands/fix/agent.rs#L313-L327)
- **Tests.** The streaming runner exercises a stub backend with a shell that prints synthetic usage. GitHub tests cover response parsing and command construction. The `gh` integration examples examined here require authentication and are ignored by default. The source review found no full fake-`gh` flow in those tests. [Agent stub](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/agent_runner.rs#L778-L919) [GitHub command and live checks](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/github.rs#L1876-L1942) [GitHub parsing](https://github.com/fotoetienne/gru/blob/3e6823b1035853c570e3e45c7f1f0a51e9de1546/src/github.rs#L2477-L2537)

The M4 choices and cleanup rules are recorded in [the worktree milestone](../milestones/M4-worktrees-pr.md#choices-from-s3) and [pitfalls](../pitfalls.md#other-tools-traps).
