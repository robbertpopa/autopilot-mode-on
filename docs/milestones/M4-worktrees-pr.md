# M4: worktrees, parallelism, pull requests

**Owner crates:** `git` (part 1), `tracker`, `policy`, `pipeline` scheduler (part 2). **Depends on:** M0 for part 1. Part 2 needs M3 and S6. Read S3 before CI repair. **Gate for:** M7.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M4-worktrees-pr.md`."

## Goal
Two tickets run in parallel on a sample repository and open two draft pull requests, safely.

## Part 1: `git` crate (can start after M0)
- [ ] Worktree manager over the `git` CLI: create from a base branch under a configurable data folder (not inside the repo), name branches `ticket/<id>`, list, remove with retry on Windows file locks, clean orphans after a crash.
- [ ] Diff and stat helpers; a tree fingerprint for kept check results.
- [ ] Conflict prediction: which files a ticket will touch (from Understand and Plan) versus the files other running tickets touch.

## Part 2
- [ ] Scheduler: slot limit, queue with the reason ("waits for #128: both change routes.rs"), fairness, pause and resume of the whole queue.
- [ ] Reuse the S6-tested isolation required by M3. Keep publisher credentials on the host side and outside the agent and check runtimes.
- [ ] `tracker` through the `gh` CLI: read issues as tickets; persist approval for the exact checked repository, base and head before creating the feature branch and draft PR. Reconcile uncertain results by branch and PR identity. Use actual check results and a code-made summary in the description.
- [ ] CI watch: show status for the current PR head. Later, after S3, allow up to two repair attempts using bounded, redacted failure data from trusted checks on that head. Updating the remote head requires rechecking and fresh approval.
- [ ] Diff scan: `.github/**`, lockfiles, scripts, new dependencies, new network calls force a confirmation.
- [ ] Never merge, never push to a protected branch (enforced in code and in the deny hook).

## Tests that must exist
- Two tickets touching one file queue and run in order; two independent tickets run in parallel up to the slot limit.
- A poisoned issue text, with a fake agent that obeys the injection, cannot cause a push, read a secret or edit outside its worktree (end-to-end).
- Worktree cleanup after a killed run leaves no orphan folder or branch.
- The PR description contains no secret and states only checks that ran.
- An uncertain publish followed by a restart reconciles the existing branch and draft PR. A changed head invalidates the approval.
- A failing CI log is fed back at most twice when CI repair is enabled.

## Exit criteria
First, one ticket opens a draft PR on an explicitly approved sample remote. Then two eligible tickets produce two draft PRs in parallel and the poisoned-ticket test passes.
