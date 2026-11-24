<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Next steps and how to resume the orchestration (2026-11-25)

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
| Window (stream 4) | Look at the installed release app again after the last fixes (black strip beside the sidebar, clipped Inbox card, status pill over the step label, "since start" clock) and fix what is still wrong; the stream file lists W3 to W10 still open. Light mode and a narrow window were never looked at. |
| Pipelines UI | Check drag to reorder, the reusable step library and the Edit with AI diff in the real window; rules.md Improve with AI has only run against fixtures. |
| macOS and sandbox (stream 3) | Per-call budget defaults (Design and Plan need far more than $0.05) with a "raise the cap and retry" action; git checks inside the check role (home configuration is unreadable by design); what the Design source options do; a test of three parallel tickets in one project. Intel Macs and other macOS versions are unvalidated; a descendant that calls `setsid` escapes cleanup on macOS. |
| Pull request lifecycle (stream 1) | L5 repair as Build rounds, L6 publish only with approval, L8 review of existing pull requests, L9 intent on every action; the live proof against real GitHub went once through the CLI (throwaway repository pull request 1) but not through the window. |
| Agents (stream 2) | A3 (decide Codex), A4 adapter, A5 capabilities, A6 provider drivers, A8 MCP server; each needs a live run within the budget. |
| Evaluation and review (M7 to M9) | Almost untouched: pipeline health on real data, codebase review, packaging for Codex and MCP. |

## What only the owner decides

2. Computer-use scope (disposable VM only, or the real desktop).
3. Which other open-source agents to adopt.
4. Subscription mode inside the sandbox (stays refused until the exfiltration path is closed).
5. Spend: about $2.1 has gone through the API key this session, above the $1 in the guide because the per-call limit was raised so Design and Plan could finish. Set a new total and per-call cap before the next live run, and say whether Opus may be used for Plan.

## Orchestration notes from this session

- Workers refuse to merge or push on a relayed message ("the owner allows it"), correctly. The orchestrator merges, or the owner tells that worker directly in its own chat.
- A stacked set of pull requests (each branch contains the one before) merges badly with squash: merge the top one and close the others as included, as was done for the pipelines work.
- Two pull requests that are green alone can break `develop` together: merge one, wait for `develop` CI, then rebase the next.
- The review bot on this public repository stops after its free limit; its comment says when the next review is available. It is not a required check. Rerun it by commenting `@coderabbitai review` on the pull request once the time has passed.
- BSD `sed` on macOS needs `sed -i ''`; prefer the edit tools for source files.
- CI uses the pinned toolchain; a newer local Rust reports lints CI does not (and the reverse), so trust CI.
- Keep orchestrator and worker context under 400k tokens: read files by path with offsets, tail command output.
- Live tests: one ticket at a time until parallel runs are proven; read events with `autopilot watch --data-dir <dir> <ticket>` and the process list instead of trusting a "running" label (a run with no child process and a frozen cost is stalled).
