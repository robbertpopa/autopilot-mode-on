<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M6: the design step

**Owner crates:** `pipeline` (design step), `ui` (viewer for the fallback). **Depends on:** M3. The local HTML path can ship independently. The remote canvas path needs S4.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M6-design-step.md`."

## Goal
A ticket that changes a user-facing screen produces an approved design before any code is written.

## Tasks
- [x] Start with local HTML mockups (`engine::design`). The remote Claude Design path is the separate item below and waits for S4.
- [x] Run only when Understand reports `needs_design`; skip with the reason otherwise. A ticket that needs a design holds Build until the design is settled.
- [ ] Claude Design path (waits for S4's live trial and a per-project opt-in): the agent makes the canvas, returns its link; revisions go to the same canvas (same URL); the link is stored in the ticket. Requires a claude.ai login, so the step's auth is subscription; the step is switchable off per project (the canvas lives on claude.ai under the user's account, so private repos may not want it).
- [x] Local path: the agent returns static HTML mockups as its structured answer; the host refuses any that hold script, a frame, an event handler or an address, saves the rest under `designs/<ticket>/` in the data directory (not in the worktree, so no commit can contain them) and the window opens them in the browser from the approval.
- [x] Approval: always asked when the step runs (checkbox on by default, labelled "the design must be approved"); feedback in the inbox goes back to the same design; reject stops the ticket.
- [x] The approved design is attached to the plan and becomes context for the build (`engine::design::context` feeds the Plan prompt).
- [ ] Optional later: a visual check that compares the built UI with the approved design.

## Tests that must exist
- Skipped without `needs_design`; asked when it runs; a revision keeps the same link; a reject stops the ticket (fake agent).
- Switching the step off per project skips it with the reason.

## Exit criteria
A UI ticket on a sample project yields an approved design (either path) before the build starts.
