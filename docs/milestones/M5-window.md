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
- [x] Shell: titlebar, ticket sidebar with live chips and inline approve/reject, accounts (GitHub, Claude, Codex, Azure), dark and light theme, Ctrl+N. Open: desktop notifications (`notify-rust`).
- [x] Pipeline view (settings, ask-after, Simple and Advanced, validation from the profile; edits go to `profile.toml` through `engine::edit_step`). Open: forced-confirmation badges. Original item: nodes with settings and the "Ask me after this step" checkbox, Simple and Advanced views, forced-confirmation badges, validation warnings (for example a dollar budget on a subscription step). Edits go to `profile.toml` through `engine`.
- [x] Run view (strip, Build ⟲ Check panel, timeline, cost split). Open: turns/tokens/cache and the time estimate, which are not recorded yet. Original item: live strip with skip reasons, Build ⟲ Check panel, timeline, cost bars (API exact, subscription estimate shown apart).
- [~] Step detail (tabs built; Input, Prompt and Changes say they are not recorded yet; re-run, explain, open worktree and terminal are shown off with the reason). Original item: tabs Overview, Input, Prompt, Output, Log, Changes, Cost, Settings; the actions; "In short" and "Explain in plain words".
- [x] Inbox cards for questions and approvals (sidebar and run screen).
- [x] "Ask for a change": read by a fixed grammar in `engine::change` (no model), shown as a diff, applied to this ticket or all future ones; spending limits refused with the reason. Open: the "review" clause (no separate review step yet) and the target switch (This ticket's code / The pipeline / Just ask).
- [x] Graph widget: custom painter (arrows between step cards).
- [~] Repaint on events only (done: a watcher repaints when a ticket changes). Open: full keyboard navigation and accessible names.

## Tests that must exist
- View-model tests in `engine` (no egui): state to display model for each step state, including skipped with reason and asking.
- Every UI action maps to an engine action already covered by a CLI test.
- A few layout smoke tests (the window builds with each ticket state without panic). No pixel assertions.

## Exit criteria
Run a ticket from the window: watch it live, answer a question, approve a plan, see the checks tick, and open the draft PR.

## Try it
`cargo build --workspace`, then `cargo run -p autopilot-ui -- --data-dir <dir> --profile <file>`. Add `--demo` to fill an empty data directory with three sample tickets. `--view design|run|step|cost|accounts` opens on a screen.
