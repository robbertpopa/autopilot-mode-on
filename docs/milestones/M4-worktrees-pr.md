<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M4: worktrees, parallelism, pull requests

**Owner crates:** `git` (part 1), `tracker`, `policy`, `pipeline` scheduler (part 2). **Depends on:** M0 for part 1. Part 2 needs M3 and S6. Read S3 before CI repair. **Gate for:** M7.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M4-worktrees-pr.md`."

## Goal
Two tickets run in parallel on a sample repository and open two draft pull requests, safely.

## Part 1: `git` crate (can start after M0)
- [x] Worktree manager over the `git` CLI: create from a base branch under a configurable data folder outside the repo, name branches `ticket/<id>`, list, remove with retry on Windows file locks, and recover clean owned checkouts after a crash.
  - [x] Resolve the exact local base branch and read Git's registered worktrees through the bounded process runner.
  - [x] Reserve a host-managed checkout outside the repository before creating its ticket branch from the captured base commit. Reopen a completed checkout only after matching its ownership, branch and path. An incomplete checkout is left available for recovery.
  - [x] Recover a pending checkout at the reserved commit after verifying its Git identity, unlocked index and clean worktree status. Preserve checkouts with changes, submodules or index flags that hide files for inspection.
  - [x] Remove clean owned checkouts and resume interrupted removals. Retry Windows sharing failures while keeping the Git registration. Delete an unchanged ticket branch and preserve any branch with a changed tip.
  - [x] Kill a synthetic owner after it creates its checkout. A fresh manager removes the checkout, unchanged ticket branch and ownership record while keeping the base checkout intact. The host must select the stopped ticket for cleanup.
- [x] Diff and stat helpers; a tree fingerprint for kept check results.
  - [x] Summarize two local commits with repository-relative changed paths and text line counts. Preserve binary entries and report both paths of a rename.
  - [x] Fingerprint a verified worktree after its workers stop. Include Git's commit and index, working files, untracked and ignored entries, supported permission bits and in-tree symlink destinations. An external or unresolved link prevents reuse. Keep the base, approved plan and check settings as separate evidence.
- [x] Conflict prediction: compare the files predicted by Understand and Plan against running tickets. Report equal paths and directory overlaps, including case variants on Windows. The scheduler consumes the comparison in part 2.

## Part 2
- [x] Scheduler (`pipeline::scheduler`, pure state): slot limit, queue with the reason ("waits for #128: both change routes.rs"), fairness (arrival order; a blocked ticket never holds up an independent one and is never overtaken by a later overlapping one), pause and resume of the whole queue. The loop that starts tickets from it comes with the runner in M5.
- [ ] Reuse the S6-tested isolation required by M3. Keep publisher credentials on the host side and outside the agent and check runtimes.
- [ ] `tracker` through the `gh` CLI: read issues as tickets; persist approval for the exact checked repository, base and head before creating the feature branch and draft PR. Reconcile uncertain results by branch and PR identity. Use actual check results and a code-made summary in the description.
- [ ] CI watch: show status for the current PR head. Later, after S3, allow up to two repair attempts using bounded, redacted failure data from trusted checks on that head. Updating the remote head requires rechecking and fresh approval.
- [ ] Diff scan: `.github/**`, lockfiles, scripts, new dependencies, new network calls force a confirmation.
- [ ] Never merge, never push to a protected branch (enforced in code and in the deny hook).

## Choices from S3

Adopt these three patterns from the [source review](../spikes/S3-read-gru.md#result).

- Keep the checkout separate from runtime metadata and reconcile Git's worktree list with the ownership records after a restart.
- Match failed check runs to the exact PR head and preserve both ends of a bounded, redacted log excerpt.
- Cap repair at two attempts and require a new commit, fresh checks and approval before publishing an update.

Guard against three failure modes.

- A stale base after a failed refresh. Resolve an explicit base commit and report when it cannot be verified.
- Forced cleanup of an unrelated checkout. Require recorded ownership and a canonical path within the managed root before removing anything.
- Agent-side publishing with broad permissions. Keep tools constrained in the worker and publish through the host-side approval path.

## Tests that must exist
- Two tickets touching one file queue and run in order; two independent tickets run in parallel up to the slot limit.
- A poisoned issue text, with a fake agent that obeys the injection, cannot cause a push, read a secret or edit outside its worktree (end-to-end).
- Worktree cleanup after a killed run leaves no orphan folder or branch.
- The PR description contains no secret and states only checks that ran.
- An uncertain publish followed by a restart reconciles the existing branch and draft PR. A changed head invalidates the approval.
- A failing CI log is fed back at most twice when CI repair is enabled.

## Exit criteria
First, one ticket opens a draft PR on an explicitly approved sample remote. Then two eligible tickets produce two draft PRs in parallel and the poisoned-ticket test passes.
