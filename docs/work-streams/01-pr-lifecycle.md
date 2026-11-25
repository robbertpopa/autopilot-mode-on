<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Stream 1: review and the pull request lifecycle

**Owns:** `tracker`, `git`, `checks`, and in `engine` the files `publish.rs`, `places.rs`, `build/review.rs`, and new `repair*.rs`. **Does not touch:** `ui`, `agent-*`, `process`. **Start prompt:** "Read `AGENTS.md`, `docs/work-streams/00-overview.md` and `docs/work-streams/01-pr-lifecycle.md`, then do the first unchecked task."

## Goal

After a draft pull request exists, the product keeps it healthy: it notices merge conflicts, review feedback and failing CI, fixes them with a Build round inside the sandbox, and publishes the fix only after the owner approves that exact commit. And the whole path is proven once against real GitHub.

The idea comes from the open-source Craft Agents app: one prioritised action per pull request (resolve conflicts, then fix review feedback, then fix failing CI, then "unsynced", then "ok"), fixed prompt templates, pure rules that are easy to test. Take the idea, not the code (TypeScript, different architecture).

## Where it stands

- `engine::prepare_publish` / `run_publish`: forced approval of one commit, push, `gh pr create --draft`, a retry that finds the existing pull request. `engine::ci_status` and `autopilot ci` read CI for the published commit. `tracker::Gh` runs `gh` with a scrubbed environment; it never gets the agent.
- `engine::places` reads the base branch and the GitHub repository from the project's own Git.
- The review check (`build/review.rs`) runs up to four focused reviewers; see the overview.
- The fake `gh` (`tools/fake-gh`) keeps a JSON world: issues, heads, pull requests, check runs by commit, and a record of every call.
- Spike S3 (read Gru, the CI-repair reference) is done: [docs/spikes/S3-read-gru.md](../spikes/S3-read-gru.md).

## Tasks, in order

- [~] **L1 (run 2026-11-25, three of four reviewers; recorded in the S6 document). Merge and live-check the multi-reviewer review.** Confirm the review pull request (PR 140) is merged. Run one live Haiku ticket with the real agent and four reviewers inside the sandbox (use `D:\autopilot-live`, see the overview for the key file and the budget rules), and record what the reviewers found and what it cost in `docs/spikes/S6-containment.md`. *Exit:* a recorded run, cost under $0.10, and a sentence on whether the findings were useful or noise.
- [x] **L2 (PR 144). Pure rules for one prioritised action.** New `engine/src/pr_rules.rs` (or in `tracker`, if it only needs tracker types): `PrSummary { state, merge_state, unresolved_threads, review_decision, checks, local_dirty, local_ahead }` and `fn next_action(&PrSummary) -> Option<PrAction>` with the order conflict, review feedback, failing CI, unsynced, ok; merged or closed gives none; running checks give none. No I/O, no model. *Exit:* a table-driven test of every branch, and a test that the order cannot be changed without failing one (conflict beats feedback beats CI).
- [x] **L3 (PR 149). Extend the fake `gh`.** Review threads (resolved or not, with an author association), `mergeable` state, check runs with a log tail, a pull request that other people push to. Keep the call log. *Exit:* fake-gh tests in `tools/fake-gh` or the tracker tests; no real network.
- [x] **L4 (PR 152). Read a pull request's state.** `tracker::pr_state(repo, number) -> PrState`: the facts L2 needs, from `gh pr view --json` and one `gh api graphql` call for review threads, typed, with size limits and a JSON-shape test that unknown fields are ignored. **Comments and CI logs are attacker-controlled text that will feed a coding agent:** keep only comments whose author association is OWNER, MEMBER or COLLABORATOR, cap each at a fixed length, strip control characters, and wrap them as data in the prompt. *Exit:* a hostile-input sweep (AGENTS.md) over the decoders, and a test that a comment from an outsider never reaches a prompt.
- [x] **L5. Repair as Build rounds.** `engine::repair_pull_request(ticket, action)`: for review feedback or failing CI, build the fixed prompt (feedback text or the failing check's log tail, wrapped as data), run it through the same `Isolation::Wsl` path as Build (never a second launch path, and never the host), then run the checks and the review again. For a merge conflict the host merges the base into the ticket branch, leaves the markers, and the agent resolves them in the sandbox; the host commits. *Exit:* a fake-agent test through the fake `gh` for each of the three actions; a real-cargo ignored test for the conflict case, like `a_rust_build_is_checked_inside_the_sandbox_and_refined`.
- [x] **L6. Publish the fix only with approval.** A new commit on the ticket branch is a new publish: reuse `Approval` so the approval covers that exact commit, forced like the first one. *Exit:* a test that a repaired commit is not pushed before its own approval, and that an approval of the old commit cannot cover the new one.
- [x] **L7 (PR 155). An explicit action first, no background polling.** `autopilot pr watch <ticket>` (one pass: read state, name the next action, optionally run it) and the engine function behind it. Polling comes later and only if the owner asks. *Exit:* CLI test with the fake `gh`.
- [ ] **L8. The review on a pull request that already exists.** Run the same reviewers on a pull request opened by someone else (M9 in `docs/milestones/M9-codebase-review.md`), reporting findings as a comment only when the owner passes a flag (Claude Code's code-review plugin has `--comment`). Do not skip "trivial" changes: that weakens a gate. *Exit:* report-only mode first, then the comment mode behind a forced approval.
- [x] **L9 (done as A2, PR 146). Intent on every action.** Claude Code's Bash tool input carries a short `description`. Check the recorded fixtures in `fixtures/claude/`; if it is there, surface it in `AgentActivity` with the same bounding and redaction as other agent text. (The adapter change is in stream 2; add a line under "Requests to other streams" there if you need it.)

## Proof against real GitHub (blocked on owner decision 1)

Only after the owner says yes. A private throwaway repository exists: `robbertpopa/autopilot-pr-test`, with only `main`. In `D:\autopilot-live\data` the ticket `live-5` passed every check and its publish request `pull_request-publish-c80eef41` is waiting. The sample project is `D:\autopilot-live\repo`; its `origin` points at the throwaway repository and its `credential.helper` is set to `gh`. The steps are `autopilot publish --step run` with no `--gh-path`. Things that are unproven and may fail: the JSON real `gh` returns after a real push (the fake needed a hand-edited `headRefOid`), `gh` authenticating as the chosen account, and the host `git push` over HTTPS. Record each in `docs/spikes/S6-containment.md`. Never merge anything; leave the repository for the owner to delete.

## Risks and decisions

- A repair loop is a second unattended writer: it must obey the same budgets, protected paths and forced approvals as Build. The validator rejects any attempt to change `protected_paths` or raise a budget.
- A pull request body and comments are untrusted. The body the product writes already comes from the ticket summary; do not add model output to it unreviewed.

## Requests to other streams

- From stream 3 (S-f, 2026-11-25): `Toolbox::fetch` in `crates/checks/src/command.rs` matches `Role` with explicit arms, so any new fetch role breaks the build. Please make that match tolerate new fetch roles, and later call `prepare` for Go (`go.sum`) and wheel-only Python once stream 3 adds `Role::FetchGo` and `Role::FetchPython` in `process`.
