<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M5: the window (egui)

**Owner crates:** `ui`, `engine` (actions it needs). **Depends on:** M2 (events and inbox). The graph needs S7. The full demo needs M3 and M4.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M5-window.md`."

## Goal
A ticket can be run, watched, answered and made merge-ready entirely from the window. The window holds no logic: everything it does is an `engine` action that the CLI also has.

## Reference
`docs/ui.md` and the three local artboards described there.

## Tasks
- [ ] `engine` crate (done: run steps, answer, approve, pause, resume, stop through flags a run checks between turns, publish, CI; open: re-run, re-run from here, apply pipeline change, open worktree, open terminal, explain): one definition per action (run, pause, stop, re-run, re-run from here, answer, approve, apply pipeline change, open worktree, open terminal, explain) exposed to CLI, MCP and UI.
- [x] Event subscription with replay: `engine::follow` replays from a sequence number, then delivers new events by polling the store (so a writer in another process is seen); `autopilot watch --follow`. Tested late-open plus live. A broadcast channel was not needed.
- [ ] Shell: titlebar, ticket sidebar with live chips, notifications (`notify-rust`).
- [ ] Pipeline view: nodes with settings and the "Ask me after this step" checkbox, Simple and Advanced views, forced-confirmation badges, validation warnings (for example a dollar budget on a subscription step). Edits go to `profile.toml` through `engine`.
- [ ] Run view: live strip with skip reasons, Build ⟲ Check panel, timeline, cost bars (API exact, subscription estimate shown apart).
- [ ] Step detail: tabs Overview, Input, Prompt, Output, Log, Changes, Cost, Settings; the actions; "In short" and "Explain in plain words".
- [ ] Inbox cards for questions and approvals.
- [ ] "Ask for a change": three targets, a typed pipeline patch shown as a diff, apply to this ticket or all future ones; rule changes rejected with the reason.
- [ ] Graph widget as decided in S7 (egui-snarl or custom).
- [ ] Repaint on events only; keyboard navigation and accessible names on controls.

## Tests that must exist
- View-model tests in `engine` (no egui): state to display model for each step state, including skipped with reason and asking.
- Every UI action maps to an engine action already covered by a CLI test.
- A few layout smoke tests (the window builds with each ticket state without panic). No pixel assertions.

## Exit criteria
Run a ticket from the window: watch it live, answer a question, approve a plan, see the checks tick, and open the draft PR.
