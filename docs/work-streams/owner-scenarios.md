<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Owner scenarios: end-to-end acceptance

Three requests from the owner, written as steps a person can follow in the window, each with the result to expect. They are acceptance checks: a scenario passes only when someone ran every step in the window and saw the result. Owned by stream 4 (task W17); engine or agent work they need goes to the stream that owns the crate. Start the window with a profile that has one project, and `autopilot-ui --demo` where a step needs a ticket in a given state.

Tick a scenario with the date and commit once it passed. Never tick one from a test alone.

## Scenario A: the window and the pipelines

- [ ] **A. Pass date and commit:** _not run_

1. **Look at the top.** Open the window. *Expect:* no separate app title bar of our own above the content; the top of the window looks like Claude's top bar (window controls and one slim row). Nothing is cut off.
2. **Centering.** Resize from wide to narrow. *Expect:* the content column stays centered with equal margins left and right on every page; no page sits off-center.
3. **Create a pipeline.** Open Pipelines, choose New pipeline, name it, save. *Expect:* it appears in the list and can be chosen on New ticket.
4. **Remove a pipeline.** Delete the one just made, confirm. *Expect:* it leaves the list; tickets that used it keep running with their stored steps; the built-in pipelines cannot be deleted.
5. **Steps of one pipeline.** Open a pipeline, add a step that belongs only to it, give it a prompt. *Expect:* the step shows in that pipeline only.
6. **Reusable steps.** Save a step as reusable, then add it to a second pipeline. *Expect:* both pipelines list it; editing the reusable step changes both, and the page says so before saving.
7. **Reorder pipelines.** Drag one pipeline above another. *Expect:* the order changes at once and is the same after a restart.
8. **Explain Design.** Open the Design step. *Expect:* a plain-words explanation of what the step does, when it runs (only when Understand reports `needs_design`), what it produces, and that it needs approval.
9. **Show and edit a step by hand.** Open the step's prompt and agent details (model, effort, tools, limits). Change the model and one line of the prompt, save. *Expect:* the new values show after reopening; a value that loosens a safety rule is refused with the reason.
10. **Edit a step with AI.** Type "make this step ask fewer questions" in the step's change box. *Expect:* a proposal shown as a before/after of the prompt and settings; nothing changes until the owner accepts; declining leaves the step as it was.
11. **AI check of `rules.md`.** On the project's rules page choose Check with AI, once with Haiku and once with Sonnet. *Expect:* a list of findings (contradictions, vague rules, rules the agent cannot follow), each pointing at a line, with the model and the cost shown; the file is not changed.

## Scenario B: a detailed plan on every step

**Built (macOS, not yet run by hand):** every step has a fixed ordered plan (`engine::live::sub_steps`), ticked only from recorded events (`TicketView.facts`), shown as a collapsible Plan under each step on the ticket page (open for the step that is on and the one that is next), in the step's own page, and under the live status line. Item states: pending, in progress, done, failed, stopped here (with the reason), skipped (with the reason). Elapsed time counts from the step's latest start. **Not built:** step 4 (an agent adding or dropping items): plans are fixed per step kind, the Build list grows by round and check as they report.

- [ ] **B. Pass date and commit:** _not run_

1. **Start a ticket.** Start a ticket in a project with the default pipeline. *Expect:* before any step runs, each step already shows its planned checklist (what it will do, in order), not just a name.
2. **Before a step runs.** Open a step that has not started. *Expect:* its checklist with every item unticked, and the step marked as waiting.
3. **While it runs.** Open the running step. *Expect:* items tick off as the step does them; the item in progress is marked; the ticks come from recorded events, not guesses; elapsed time counts from when this step started.
4. **A step changes its plan.** When the agent adds or drops an item, *expect* the checklist shows it, and items already done stay ticked.
5. **After a step.** Open a finished step. *Expect:* the same checklist with what was done ticked, anything skipped marked skipped with a reason, and the step's cost and result.
6. **A stopped or failed step.** Open a step that stopped for the owner or failed. *Expect:* ticked items so far, the item it stopped at marked, and the reason in plain words.
7. **Restart.** Close and reopen the window. *Expect:* every checklist is the same as before closing.

## Scenario C: archive, delete, restore, stop, resume

- [ ] **C. Pass date and commit:** _not run_

1. **Archive from the ticket page.** Open a ticket, choose Archive. *Expect:* the ticket leaves the active list and sidebar, shows in Archived, and nothing running is lost.
2. **Archive from the sidebar.** Right-click (or use the menu on) a ticket in the sidebar, choose Archive. *Expect:* the same result as step 1.
3. **Restore.** Open Archived, choose Restore on a ticket, from the page and from the sidebar. *Expect:* it returns to its old place with its steps, history and cost unchanged.
4. **Delete asks first.** Choose Delete on a ticket, from the page and from the sidebar. *Expect:* a confirmation that names the ticket, says what is removed (history, worktree and branch only if the owner picks that) and that it cannot be undone; Cancel changes nothing.
5. **Delete.** Confirm. *Expect:* the ticket is gone from every list; a running step is stopped first; no worktree or branch is left behind unless the owner chose to keep it.
6. **Stop a conversation.** While a step runs, choose Stop. *Expect:* the agent stops within a few seconds, the step shows "stopped by you" with the cost so far, and no process is left running.
7. **Resume.** Choose Resume on the stopped ticket. *Expect:* the conversation continues from where it stopped (same provider session when one exists, otherwise a new one that is told what was done), and the history shows both parts.
8. **Resume after restart.** Stop a ticket, close the window, reopen, resume. *Expect:* the same as step 7.
9. **Both places agree.** Do steps 1 to 8 using the ticket page for one ticket and the sidebar for another. *Expect:* the same states and the same wording in both places.

## First run on macOS (2026-11-24, develop at f6aae06)

Observed by driving the real window with computer use (Haiku, API mode, cost $0.21). Plan and Build were not reached. These are observations, not fixes.

**Ticket for scenario A (top bar and pipelines)**
- Understand and Clarify passed. Design ran although the pipeline chosen was "Normal". Design hit `error_max_budget_usd` at the $0.05 per-call cap and was interrupted with no result.

**Ticket for scenario B (per-step checklist)**
- Understand, Clarify and Design passed (2 mockups; the forced Design approval was asked).
- The step list on the ticket page shows only step names and status. There is no per-step checklist plan: scenario B is not implemented.

**Ticket for scenario C (archive, delete, stop, resume)**
- Understand, Clarify and Design passed. Plan hit `error_max_budget_usd` at $0.05 (interrupted, "Try again" shown).
- The ticket page "More" menu has only "Open the worktree" and "Open in a terminal". There is no archive, delete, restore, stop or resume: scenario C is not implemented.

**Bugs and risks observed**
- A ticket shows under "no project" right after creation although a project is set (W14).
- The Inbox detail pane is clipped on the right edge in a 1280 px window; text and "Open ->" are cut off (W13).
- The "Reading your request…" and "Making the design…" spinner shows in the page header and inside the card (W15).
- Built-in pipelines are read-only with only Duplicate; no create or delete pipeline is visible, and the detail view shows all six steps for every pipeline (scenario A, steps 3 to 6).
- The pipeline's Plan step defaults to auth=subscription, model=opus (cost risk, A13). The orchestrator suggests "Plan: opus · high effort" for tickets.
- A bare binary shows "autopilot-process-guardian is missing next to the window" unless the sibling binaries (`autopilot-process-guardian`, `autopilot`, `autopilot-fake-agent`) sit beside it.
- The first-run folder picker is a native NSOpenPanel that cannot be driven in background mode.
- The per-call $0.05 cap is too small for Haiku Design and Plan on a real repo: Plan fails with no result (A13).
