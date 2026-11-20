# M6: the design step

**Owner crates:** `pipeline` (design step), `ui` (viewer for the fallback). **Depends on:** M3. The local HTML path can ship independently. The remote canvas path needs S4.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M6-design-step.md`."

## Goal
A ticket that changes a user-facing screen produces an approved design before any code is written.

## Tasks
- [ ] Start with local HTML mockups. Enable the remote Claude Design path only after S4 verifies publishing, revisions and privacy.
- [ ] Run only when Understand reports `needs_design`; skip with the reason otherwise.
- [ ] Claude Design path: the agent makes the canvas, returns its link; revisions go to the same canvas (same URL); the link is stored in the ticket. Requires a claude.ai login, so the step's auth is subscription; the step is switchable off per project (the canvas lives on claude.ai under the user's account, so private repos may not want it).
- [ ] Local path: the agent writes static HTML mockups into `design/` in the worktree. Display them with active content and external resource requests disabled.
- [ ] Approval: always asked when the step runs (checkbox on by default, labelled "the design must be approved"); feedback in the inbox goes back to the same design; reject stops the ticket.
- [ ] The approved design is attached to the plan and becomes context for the build.
- [ ] Optional later: a visual check that compares the built UI with the approved design.

## Tests that must exist
- Skipped without `needs_design`; asked when it runs; a revision keeps the same link; a reject stops the ticket (fake agent).
- Switching the step off per project skips it with the reason.

## Exit criteria
A UI ticket on a sample project yields an approved design (either path) before the build starts.
