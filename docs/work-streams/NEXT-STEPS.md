<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Next steps and how to resume the orchestration (2026-11-25, updated at the end of the second wave)

Read this first on a new machine, then [ORCHESTRATOR.md](ORCHESTRATOR.md), [owner-prompts.md](owner-prompts.md) (the three prompts and the run recipe) and [owner-scenarios.md](owner-scenarios.md) (what each prompt must show). `develop` carries everything merged so far; there is no unmerged work worth keeping outside it.

## Resume in five steps

1. Clone `https://github.com/robbertpopa/autopilot-mode-on`, check out `develop`, install Rust from `rust-toolchain.toml` (the pinned 1.99.1 or newer, through rustup), run `cargo xtask ci` once.
2. `gh auth login` as `robbertpopa`; check `gh auth status` before each push. Never use `--dangerously-skip-permissions`. No `Co-Authored-By` or "Generated with" lines.
3. Mac: run `tools/macos/setup.sh`, sign in with `claude auth login`, create a new spend-limited API key (the old one was printed once in a worker's local transcript: revoke it) and keep it outside the repository.
4. Build and install the app (recipe in `owner-prompts.md`), start it with the key in the environment, and run the three prompts one ticket at a time.
5. Start the orchestrator session in the repository, tell it "read docs/work-streams/NEXT-STEPS.md and ORCHESTRATOR.md and continue", and let it open one worker per lane below. Workers push their branches and open pull requests; the orchestrator checks CI and the file list of each pull request and squash-merges into `develop` (never `master`).

## Lanes and what is left in each

| Lane | Next |
|---|---|
| Pipelines UI | Check drag to reorder, the reusable step library and the Edit with AI diff in the real window; rules.md Improve with AI has only run against fixtures. |
| macOS and sandbox (stream 3) | Done 2026-11-25: probe rows as data, terminal from `process`, macOS `setsid` cleanup, S11 spike. Open: a "raise the cap and retry" action in the window; git checks inside the check role (a Mac probe row needs a Mac session; a Linux row would prove nothing); Go and wheel-only Python fetch roles (the hook exists, PR 209); the Codex network role (after Codex); git inside the sandbox (S-g); Intel Macs and other macOS versions; computer use C2 to C6 (owner decision 2). |
| Pull request lifecycle (stream 1) | Done 2026-11-25: L1 live review run, L5 repair as Build rounds, L6 publish a repair only with its own approval, L8 review of an existing pull request (report only, then comment after a forced approval), `sync_inbox` keeps `perm-` requests. Open: the real-GitHub proof through the window (owner decision 1); a review run with a planted defect and a project that has rules. |
| Agents (stream 2) | Done 2026-11-25: A8 MCP read views and answers, the A11 permission prompt tool (not launched yet), A12 SDK decision (defer), A13 limit stop with the money spent and the Plan cost table, A3 documentation checks (Codex no-go for now). Open: A11 launch wiring (`--mcp-config`, `--permission-prompt-tool`) and the window listing and answering `perm-` requests; A13 retry display; A4 Codex adapter and A5 `capabilities()` after the owner provides an OpenAI key; A6, A7, A9, A14. |
| Window (stream 4) | Done 2026-11-25: spend limits and daily limit, notices per ticket, wrapped plan rows, error log, terminal and Build reset, shared probe, slots and queue, running turn and cost, `run_plan` guard. Open: look at every new screen by eye (light and dark, narrow); W8 health on real data; W7 triggers; W9 accessibility; W17 owner scenarios; W6 graphs document; W10 release checklist; deleting a ticket does not remove its worktree; a Build waiting for the owner cannot be reset. |
| Evaluation and review (M7 to M9) | Almost untouched: pipeline health on real data, codebase review (`autopilot review` for a whole repository), packaging for Codex and MCP. |

## What only the owner decides

2. Computer-use scope (disposable VM only, or the real desktop).
3. Which other open-source agents to adopt.
4. Subscription mode inside the sandbox (stays refused until the exfiltration path is closed).
5. Spend: the $1 cap restarted on 2026-11-25 and about $0.09 has gone through the key (review run, Plan measurement, permission-prompt trial). The cap and the per-call cap now live in Settings (PR 188), so raise them there. Say whether Opus may be used for Plan.
6. Codex: create a dedicated OpenAI API key with a Platform spend limit and say so, and decide whether the ChatGPT route may ever run unattended. Then one short session finishes S8 and the adapter becomes go or no-go on evidence.
7. Subscription mode: read [S11](../spikes/S11-subscription-filter.md) and the spike in `tools/s11-launcher/`; the product still refuses subscription mode.
8. The real-GitHub proof (decision 1) and the release steps (`develop` to `master`, repository secrets, Pages) are unchanged and still need the owner in the session.

## Orchestration notes from this session

- Workers refuse to merge or push on a relayed message ("the owner allows it"), correctly. The orchestrator merges, or the owner tells that worker directly in its own chat.
- A stacked set of pull requests (each branch contains the one before) merges badly with squash: merge the top one and close the others as included, as was done for the pipelines work.
- Two pull requests that are green alone can break `develop` together: merge one, wait for `develop` CI, then rebase the next.
- The review bot on this public repository stops after its free limit; its comment says when the next review is available. It is not a required check. Rerun it by commenting `@coderabbitai review` on the pull request once the time has passed.
- BSD `sed` on macOS needs `sed -i ''`; prefer the edit tools for source files.
- CI uses the pinned toolchain; a newer local Rust reports lints CI does not (and the reverse), so trust CI.
- Keep orchestrator and worker context under 400k tokens: read files by path with offsets, tail command output.
- Live tests: one ticket at a time until parallel runs are proven; read events with `autopilot watch --data-dir <dir> <ticket>` and the process list instead of trusting a "running" label (a run with no child process and a frozen cost is stalled).
