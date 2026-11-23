# Window redesign: egui plan

Draft 2026-11-22. Source design: the private Claude Design canvas "Autopilot Redesign" (10 screens: Inbox/Today, Ticket, Plan review, New ticket, Settings, Board, First run, Projects, Pipelines, Pipeline health; each dark and light; the Pipeline health numbers there are sample data). This file says how to build it in `crates/ui`, in what order, and what each slice needs from other tabs. Read [AGENTS.md](../AGENTS.md) and [ui.md](ui.md) first; the rules in `ui.md` still hold (thin client, repaint on events, amber means asking, dashed means skipped with its reason).

## What changes for the user

| Today | After |
|---|---|
| Home opens on a start box; approvals live on each ticket page | Home is **Today**: every decision across tickets, one detail pane, keys J/K/A/E/R |
| Ticket page stacks status card, chips, history, files, settings, change box | **Ticket**: one story (newest last) plus a step drawer |
| Step detail is its own screen with 8 tabs | Drawer with 4 tabs: Summary, Output, Log, Changes |
| Plan approval is Approve or Reject | **Plan review**: criteria with their checks, a note per criterion, assumptions to correct, "Send back with notes" |
| Pipeline graph with node settings | **When it asks you**: one switch per step, Design and PR locked, forced-ask rules spelled out |
| Cost is a separate tab | Spend meter in the nav; limits in Settings |
| No overview of parallel work | **Board**: Needs you, Working, Waiting in line, PR open, Done |
| Accounts page only | **First run**: agent, project folder, GitHub, with the fix command |
| Accounts are a settings page | Nav foot: coloured icons for Claude, GitHub, Azure and Codex with a status dot and the real account in the tooltip; Settings is a gear icon |
| Tickets are one flat list | Tickets grouped by project (Claude Code style), with a group-by-status toggle |
| One fixed pipeline | **Projects** and **Pipelines**: presets, a pipeline per ticket, agent-built drafts, per-project `rules.md` |
| No view of cost or quality over time | **Pipeline health**: grade, tokens and context per step, findings, automatic tuning |
| Long explanatory copy | Terse, technical copy (`ask_after`, `when_unsure`, worktree), keyboard hints, the app logo |

## Code shape

- Replace `Mode` and `Screen` in `app.rs` with one enum:
  `Page::{Today, Board, Ticket { id, drawer: Option<Step> }, PlanReview(TicketId), NewTicket, Projects(Option<ProjectId>), Pipelines(Option<PipelineId>), Health(Option<PipelineId>), Settings(SettingsPage), FirstRun}`.
  `SettingsPage::{Asking, Limits, Checks, Agents, Accounts, Review, Look}`. `--view` keeps its old names as aliases.
- One file per page: `today.rs`, `ticket.rs` (from `run.rs`), `drawer.rs` (from `step.rs`), `plan_review.rs`, `new_ticket.rs` (from `composer.rs`), `settings.rs` (from `pipeline.rs`, `accounts.rs`, the cost screen), `board.rs`, `first_run.rs`, `nav.rs`, `projects.rs`, `pipelines.rs`, `health.rs`, `account_icons.rs` (the nav foot).
- `Tab` shrinks to `Summary, Output, Log, Changes`. Log shows input, prompt and console in one stream; cost and settings move into Summary.
- Layout: a `SidePanel::left("nav")` (resizable, collapses to icons under 900 px), a `CentralPanel`, and for Ticket a `SidePanel::right("drawer")`. Drop the fixed `COLUMN` width; use `ui.available_width()` with a max of 1120.
- Keyboard: one `shortcuts(ctx)` per frame, read with `ctx.input_mut(|i| i.consume_key(..))`, skipped when `ctx.wants_keyboard_input()`. Ctrl+N new ticket, Ctrl+K search and commands (U13), J/K/A/E/R on Today and Plan review, P pause, Esc closes the drawer or the new-ticket page.
- Every control maps to an existing `backend::Action` or to a new one listed under "Requests to other tabs". The window starts nothing itself.

## Slices (one pull request each, in order)

Each slice keeps `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` and `cargo xtask layers` green, and extends `every_screen_builds_for_every_ticket_state` in `crates/ui/src/lib.rs` to the new page.

**Status checked 2026-11-23** by running `cargo test -p autopilot-ui --lib` (38 passed, 2 live tests ignored). U1, U3 to U10 are ticked because their named exit test exists and passes: U3 `the_a_key_approves_the_waiting_ticket_on_today`, U4 `the_timezone_ticket_shows_round_one_failed_and_round_two_live`, U5 `a_plan_can_be_made_and_approved_from_the_window`, U6 `starting_a_ticket_runs_the_real_engine_and_fixture_agent`, U7 `toggling_plans_switch_and_saving_writes_profile_toml`, U8 `badges_land_in_the_right_columns`, U9 `an_empty_data_directory_opens_first_run`, U10 `projects_list_register_and_keep_rules_outside_the_folder`. Second check the same day, also running the engine tests for draft, tuning and review (17 passed): U11 is ticked (`a_draft_cannot_weaken_design_or_the_pull_request` rejects a draft that unlocks PR, and `a_pipeline_draft_is_shown_then_saved_only_on_request` saves one), U12 is ticked (`a_finding_is_written_once_and_announced_once` for one finding and one notification, `only_a_custom_pipeline_is_tuned_and_undo_restores_it` for the tuning and Undo), U13 is ticked (`search_finds_tickets_runs_commands_and_never_reads_rules` covers all three exit cases). U2 is ticked after PR #111: `design` was missing from `apply_start_view`, and `every_view_name_opens_its_screen` now opens every name the usage text lists. The detected checks and stack from U10 now come from U10b (#112 to #115); the stack is a line on the project card, not a chip.

### U1. Theme tokens and System / Light / Dark
- [x] Move `theme.rs` colours to the canvas tokens (bg, rail, surface, raised, line, text, text-2, muted, accent, on-accent, blue/amber/green/red fill, line, text). Keep the existing names where the meaning is the same.
- [x] `AMBER`, `GREEN` and `RED` are text shades, darker in light mode (4.5:1 on white). The brighter fill shades for bars and dots arrive with U3, where they are first used.
- [x] `ui-theme.txt` stores `system`, `light` or `dark`; `system` follows `ctx.system_theme()`. Keep reading the old `light`/`dark` values.
- [x] Inter is already bundled (`assets/fonts`). JetBrains Mono for code text is a separate decision.
- Exit: `the_theme_choice_is_remembered` covers all three values; the old file still loads.

### U2. Shell and navigation (move only, no behaviour change)
- [x] Introduce `Page` and `nav.rs`: Autopilot mark, New ticket button, Today with the needs-you count, Board, tickets grouped by badge, spend meter (`spent_since`, API and subscription kept apart), Settings with an accounts dot.
- [x] Nav links: Inbox (Today) with the count, Board, Projects, Pipelines, Health. Links to pages that do not exist yet are hidden, not dead.
- [x] Tickets grouped by project, collapsible, with a group-by-project/status toggle and an add-project button; "no project" last.
- [x] Nav foot: one icon per account in its brand colour (Claude #D97757, GitHub, Azure, Codex #10A37F, grey when off) with a status dot; the tooltip holds the real detail from `autopilot_accounts` (Claude plan and email or organization, GitHub login, Azure user and subscription, Codex method). Settings is a gear icon, not the word. Azure comes from `AzureAccount` in `crates/accounts` (the Azure CLI check can take several seconds, so it loads in the background and the icon shows a pending state). Codex has only a method line today.
- [x] The app logo (`assets/icon.svg`) in the nav head; copy across the shell is terse and technical.
- [x] Existing screens render under the new pages unchanged.
- Exit: every old `--view` value still opens the right screen.

### U3. Today
- [x] Queue = tickets with `asking.is_some()`, then `Badge::Failed` and `Badge::Interrupted`, oldest first. Selection kept in `App`.
- [x] Detail by request kind, reusing `waiting_text`: approval (In short, files, what happens next from the step states, assumptions), question (choices, recommended answer, `AcceptDefaults`), stopped (Try again = `RerunUnderstand`, Close).
- [x] Approve and Reject call `Decide`; "Don't ask after this step" sets `always`. Hidden for forced confirmations, with the reason shown.
- [x] "Working on its own" list for `Badge::Running`, with the round strip from `rounds`.
- Exit: a test approves the demo Heartbeat ticket from Today using the A key.

### U4. Ticket story and step drawer
- [x] Story built from `timeline`, `steps` (skipped steps show their reason, dashed), `rounds` (one entry per round, check chips) and the next pending step.
- [x] Layout is chat first (Craft style): a top bar with a `worktree · isolated` chip; the project nav on the left; the conversation of cards in the centre; the composer at the bottom with modes `code | ask | pipeline`, `/btw`, `when_unsure ask|decide` and an approvals link; a right panel with the pipeline select, a vertical step stepper, cost, context and actions.
- [x] Step progress: the vertical stepper in the right panel (one row per step state; skipped steps dashed with their reason). The pipeline select changes only steps not started (R7).
- [x] Drawer opens on a story entry or step click; 4 tabs; actions Re-run from here, Open the worktree, Open in a terminal, Explain in plain words (greyed with the existing "coming soon" reasons until S6).
- [x] "Ask or change something" keeps `change::propose`, with three targets: Just ask, This ticket's code, How it runs.
- Exit: the demo Timezone ticket shows round 1 failed on protected paths and round 2 live.

### U5. Plan review
- [x] Full page from the Plan output: criteria with their verification id, files, test plan, assumptions.
- [x] A criterion whose verification is `unknown` shows why approval is blocked.
- [x] Approve calls `Decide`. Notes and assumption corrections wait for request R1 and R2; until then the note fields stay hidden.
- Exit: `a_plan_can_be_made_and_approved_from_the_window` runs through the new page.

### U6. New ticket
- [x] Kind choice (Something is broken, Change or add, Just ask), description, attachments, project folder preselected from `saved_workspace`.
- [x] "What will happen" panel from the profile: which steps ask (`ask_after`, locked Design and PR); Clarify and Design shown as "known after Understand".
- [x] "When it is unsure" for this ticket waits for R3; until then it shows the project setting with a link to Settings.
- [x] The duplicate warning waits for R5.
- Exit: `starting_a_ticket_runs_the_real_engine_and_fixture_agent` uses the new page.

### U7. Settings: when it asks you, limits, accounts
- [x] One switch per step bound to `ask_after`; Design and PR drawn locked. `when_unsure` as two cards. Forced-ask rules read from `protected_paths` and `budgets`.
- [x] Edits collect in `App` and are written on Save through `profile_edit`, with the diff shown first. Limits can be raised only here.
- [x] Accounts and the agent choice move here from `accounts.rs`; Look holds the theme and notifications.
- Exit: a test toggles Plan's switch, saves, and reads the value back from `profile.toml`.

### U8. Board
- [x] Columns from `Badge`: Needs you, Working, Done. "Waiting in line" needs R4. "Pull request open" stays empty until the tracker path runs for real (M4 part 2, gated by S6).
- Exit: demo tickets land in the right columns.

### U9. First run
- [x] Shown when there are no tickets and no saved workspace. Steps from `autopilot_accounts`: agent, project folder, GitHub (optional). "Copy" uses `ctx.copy_text`; "Check again" calls `fetch_accounts`.
- Exit: a test with an empty data directory opens First run.

### U10. Projects
- [x] Projects page: list, detail (workspace, default pipeline, agent, GitHub repo, detected checks, `protected_paths`, budget), "new project" from a workspace folder. Nav and the ticket list group by project; "no project" holds ask-only tickets.
- [x] New ticket gets a project select. Needs R7 (projects) before anything is stored; until then it shows the single saved workspace.
- [x] Project rules: a `rules.md` editor on the project page (text box, Save, Revert, token estimate, "always included · not committed"). New ticket shows a "rules.md · always on" row. Needs R13. See "Project rules file" below.
- Exit: a test creates a project, starts a ticket in it and reads the project back from the profile. A second test saves rules and finds them in the next step's prompt, and not in the worktree, the diff or the pull request.

### U10b. Get to know a project
- [x] Scan, card and detected checks (#112, #113, #114): adding a project (Projects button or the welcome Environment step) scans its folder once; the project page shows the card with its token estimate and "Scan again"; each detected command has "Add as check", which writes `[checks.<kind>]` through `engine::add_detected_check`. Still open: "Summarize with agent", the re-scan diff and the prompt pointer.
- [x] Project page lists suggested skills and MCP servers (#115, `engine::suggest_tools`, `crates/engine/src/catalog.rs`): Context7 for any stack, Playwright MCP for Node, GitHub MCP server for GitHub workflows, Azure MCP Server for an Azure pipeline file. Each shows maintainer, licence, source, the release looked at and what it can do. All are marked unreviewed. Advice only: nothing installs or turns on.
- Exit (met by #112 to #115): `commands_come_from_a_fixed_table_not_from_project_text`, `a_hostile_workspace_gives_a_bounded_card_without_a_panic`, `the_card_stays_under_its_cap_when_every_list_is_full`, `the_card_is_saved_outside_the_workspace_and_replaced_on_a_new_scan` and `a_detected_command_becomes_a_check_and_never_replaces_one` in the engine; `adding_a_project_scans_it_and_shows_a_card_outside_the_folder` in the window.

### U11. Pipelines: presets, per-ticket choice, agent-built
- [x] Pipelines page: presets (`full`, `bug-fix`, `quick-change`, `ui-change`, `ask-only`), node graph, step table. Built-ins are read-only; duplicate to edit; set as a project's default. `review` stays "soon" until M9.
- [x] New ticket suggests a pipeline from the kind (bug, change, ask) and lets the owner switch; Ticket shows a pipeline select that applies only to steps not started. Needs R7.
- [x] "Build with agent": the owner describes a workflow, an agent drafts a pipeline, the page shows it as a dashed draft with the agent's reasons, and nothing is stored until the owner saves. The agent cannot unlock Design or PR, drop a forced ask or raise a budget; the validator rejects such a draft before it is shown. Needs R9.
- Exit: a test drafts a pipeline from the fixture agent, rejects one that unlocks PR, saves a valid one and reads it back from `profile.toml`.

### U12. Pipeline health: review, tokens, tuning
- [x] Health tab per pipeline: last N runs, a quality grade and a token meter per step, findings with their evidence (run ids), and a "Tuned" badge with the diff and Undo.
- [x] Settings > Pipeline review: on/off, thresholds, what may change automatically, the reviewer's own spend limit.
- [x] Today shows a health item when a finding opens or a tuning is applied; a finished Ticket shows its review card (grade, tokens, context peak per step).
- Exit: a test feeds recorded runs with a high context peak, sees one finding, one notification and one tuning in the overlay, and sees Undo restore the original.
- Needs R10 to R12. See "Pipeline review and automatic tuning" below.

### U13. Global search and commands
- [x] Ctrl+K (and a search field at the top of the nav) opens one overlay: a text box, results grouped by kind, J/K or arrows to move, Enter opens, Esc closes. Opens on any page, skipped while a text field has focus unless the shortcut is Ctrl+K itself.
- [x] Searches, locally and live as you type: tickets (title, description, step summaries, timeline text, file names), projects, pipelines, settings pages and their labels, and commands. Each result shows its kind, its project and a status dot; the match is highlighted.
- [x] Commands: New ticket, Go to Board / Projects / Pipelines / Health / Settings, Switch theme, Pause or Resume the open ticket, Open the worktree (greyed until S6). Commands call existing `backend::Action`s; the palette starts nothing itself.
- [x] Filters by prefix: `t:` tickets, `p:` projects, `pl:` pipelines, `>` commands; a project chip narrows tickets to one project. Empty query shows the recent tickets and the commands.
- [x] The index is built in memory from `TicketView` and the profile on a background thread and rebuilt on a changed snapshot. It never leaves the machine, is never written to disk, and never indexes `rules.md` text, secrets, agent output marked as such, or attachments' contents (names only).
- Exit: a test finds the demo Timezone ticket by a word in its description, opens it with Enter; another runs the "Switch theme" command; a third shows that a query that matches only `rules.md` text returns nothing.
- Needs R14 only for full-text over step output; ticket, project, pipeline and settings search work without it.

## Feature inventory (every screen, every UI feature)

This is the full list from the canvas and the design conversation. Each line names the slice that builds it and the request that blocks it, if any. A line is built only after its slice ticks; until a blocking request lands, the control is shown greyed with the reason. "Canvas" = the Claude Design canvas "Autopilot Redesign".

### Shell and navigation (U2)
- Left nav, resizable, icons only under 900 px; app logo (`assets/icon.svg`) at the head; **New ticket** button with `^N`.
- Links: Inbox (Today) with an amber needs-you count, Board, Projects, Pipelines, Health. A link to a page that does not exist yet is hidden.
- Tickets grouped by project (folder icon, collapse arrow, count, amber dot when a ticket needs you); toggle group by project / by status; add-project button; "no project" group last; the open ticket highlighted; each ticket row has a status dot (amber asking, blue running, green passed, red stopped) and its round when in Build.
- Spend meter: spent this period, API and subscription apart (`spent_since`).
- Nav foot: account icons in brand colour (Claude #D97757, GitHub, Azure, Codex #10A37F), status dot (green ok, grey off), tooltip with the real account (Claude plan, email or organization; GitHub login; Azure user and subscription; Codex method); click opens First run/Accounts. A gear icon opens Settings. Azure loads in the background (slow CLI) and shows pending.
- Terse technical copy everywhere; keyboard hints as small `kbd` chips.

### Today / Inbox (U3, health items U12)
- Queue of waiting items, oldest first: asking, then Failed, then Interrupted; selection kept; J/K move, A approve, E edit/note, R reject.
- Detail pane of key/value rows by request kind: approval (in short, files, what happens next, assumptions), question (choices, recommended answer, accept defaults), stopped (Try again, Close). Approve / note / reject buttons; "Don't ask after this step" hidden for forced confirmations with the reason shown.
- "Working on its own" list with the round strip. Pipeline-health items (finding opened, tuning applied) with Undo.
- Empty state when nothing waits.

### Ticket (U4; R7 for the pipeline select, R3 for `when_unsure`)
- Top bar: ticket title, `worktree · isolated` chip, project, status.
- Centre: conversation of cards (assistant steps, check chips, rounds with check results, skipped steps dashed with reason, errors in red). Newest last. Click a card opens the drawer.
- Composer: modes `code | ask | pipeline`, `/btw` side note, `when_unsure ask|decide` for this ticket, approvals link, attach, send. "Ask or change something" targets: Just ask, This ticket's code, How it runs.
- Right panel: pipeline select (changes only steps not started) with the hint, vertical stepper (one row per step with its state and ask/locked tag), cost, context use, actions (Re-run from here, Open the worktree, Open in a terminal, Explain in plain words; greyed with reasons until S6).
- Step drawer with 4 tabs: Summary, Output, Log, Changes.
- Finished ticket: review card (grade, tokens, context peak per step) after U12.

### Plan review (U5; R1, R2)
- Full page from the Plan output: criteria with verification id, files, test plan, assumptions to correct; a criterion with `unknown` verification shows why approval is blocked; Approve; "Send back with notes" and a note per criterion (hidden until R1); assumption corrections (hidden until R2).

### New ticket (U6, U10, U11; R3, R5, R7)
- Project select with a "+" to create one; pipeline select (suggested from kind, shown as `suggested · bug` or `chosen`, "edit" link to Pipelines; locked to `ask-only` for "no project").
- Kind segment: bug, change, ask. Description box, attach button and file chips, workspace line (`read-only`, or "no workspace" for no project).
- Overlap warning with the other ticket, "merge into it" / wait in line (R5).
- Aside: the chosen pipeline's steps as a table (ask, run with round limit, `if needed`, off, locked), `when_unsure` ask|decide (R3), budget (project default, edit link), models ("after understand"), a `rules.md · always on · not committed` row.
- Start button `↵`, `⇧↵` newline, Esc closes.

### Settings (U7, U12)
- **Asking:** `pipeline · ask_after` graph with one switch per step; Design and PR drawn locked; `when_unsure` as two cards; forced-ask rules spelled out from `protected_paths`, risk and budgets; Save bar that shows the diff first, writes through `profile_edit`.
- **Limits:** budget inputs; limits can only be raised here.
- **Checks, Agents, Accounts:** detected checks, agent choice (Claude default, Codex when built), accounts with the fix command.
- **Review:** Pipeline review off / notify / notify + tune, thresholds (context %, cost multiple, window), what may change automatically, reviewer model and spend limit.
- **Look:** theme System / Light / Dark, notifications.

### Board (U8; R4)
- Five columns: Needs you, Running, Queued (R4), PR open (after S6), Done; project filter; cards with status dot, project, round.

### First run (U9)
- Readiness checklist: agent, project folder, GitHub (optional), each with state and a copy-able fix command; "Check again"; shown only when there are no tickets and no saved workspace.

### Projects (U10; R7, R8, R13)
- List with name, path, ticket count, default pipeline chip, needs-you chip; "no project" row (ask-only, words and attachments only).
- Detail: stack chip, Open folder, workspace (choose), `default_pipeline`, agent, GitHub repo, detected checks, `protected_paths`, budget.
- `rules.md` editor: chips "always included", "not committed", token estimate, "unsaved"; Save, Revert; path line; note that it cannot loosen policy; refused if it holds a key.
- "New project": name, workspace chooser, detected checks as chips, default pipeline, GitHub repo, Create project.
- Get to know (U10b; R15): built: project card with token estimate and size, "Scan again", "Add as check" per detected command, suggested skills and MCP servers with maintainer, licence, source, release looked at, permissions and an "unreviewed" chip, "advice only" note. Not built: "Summarize with agent", a "scanned at" time, the re-scan diff.

### Pipelines (U11; R7, R9)
- List of presets with a mini dot graph, `default` / `custom` / `soon` / `draft · agent` chips.
- Detail: node graph (solid = runs, amber = asks, dashed = optional), step table (step, mode, ask_after, model), rounds, off list, locked note. Duplicate; Set as default for the project; built-ins read-only.
- Build with agent: description box, example chips, Draft/Redraft, then dashed draft with "why", Discard, Refine, Save to profile.toml. `+ Blank` for a manual pipeline.

### Pipeline health (U12; R10 to R12)
- Pipeline list with status dot (watch, ok, tuned, new) and grade; review mode switch.
- Stat tiles: grade, runs, peak context, cost per run.
- Findings with evidence and run ids; states: applied (tier 1) with Undo and the guard line, proposed with Apply, needs you with Approve and Dismiss.
- Tokens by step: in, out, peak-context bar with the 80% line, cost, rounds. "not reported" where an adapter reports nothing.
- Quality rubric as pass/fail squares for the last 5 runs, rows from `docs/rules`.
- Footer: reviewer model, cost and limit.

### Search and commands (U13; R14 for step output)
- Ctrl+K overlay and a search field in the nav head; results grouped (Tickets, Projects, Pipelines, Settings, Commands); arrows or J/K, Enter, Esc; prefixes `t:` `p:` `pl:` `>`; project chip filter; match highlight; recent tickets when empty; commands greyed with the reason when gated. Local, in memory, never indexes `rules.md` text or secrets.

### Cross-cutting
- Themes: dark and light from one token set (U1); System follows the OS.
- Status colours: asking amber, running blue, passed green, stopped red, skipped dashed with its reason. Design and PR show "ask · locked".
- Accessible names on controls, 4.5:1 text contrast in both themes, visible focus, no information by colour alone (chips carry text).
- Greyed controls always show the reason (S6, S4, request id).
- Desktop notifications for: a ticket needs you, a ticket failed, a pipeline finding (open item in `WORK-PACKAGES.md`).
- Global search and command palette on Ctrl+K (U13). See the next section.
- No invented numbers: anything not measured shows `[value]` or "not reported".

## Project rules file

One Markdown file per project holds the owner's general rules (style, "ask before touching X", wording). It is always taken into account and never committed.

- **Where it lives:** the app's data directory, `projects/<project>/rules.md`, next to the project's other app state. Never inside the workspace, a worktree or the repository, so no commit, diff or pull request can contain it. The window shows the path. A ticket with no project has no rules file.
- **How it is used:** the engine adds it to the prompt of every step of every ticket in that project, under a fixed heading that says it is the owner's instructions. It is read at the start of each step, so a saved edit applies to steps not started yet. The step log records that the file was included, its size and a hash, not its text, so the log never copies it.
- **What it cannot do:** it guides the agent; it does not change policy. Design and PR still ask, forced asks, `protected_paths` and budgets still apply, and the pipeline validator ignores it. A rule that conflicts with them loses, and the step note says so.
- **Limits:** size capped (default 8 KiB, in the profile; saving a larger file is refused with the limit shown). Text that looks like a key or token is refused on save. UTF-8 only. A missing file means no rules, not an error.
- **Token cost is visible:** the editor shows an estimate per step, and the pipeline reviewer counts it. If context is high in every run, a finding can propose trimming the rules file; it never edits the file itself, because it is owner text.
- **Difference from `AGENTS.md`:** `AGENTS.md` is in the repo and shared; `rules.md` is private to this machine and this project.

## Get to know

When a project is created, the engine builds a short **project card**: what this project is made of and how to build and test it, without spending the agent's context.

- **Scan (no tokens). Built.** Plain Rust reads names, not source: it checks for `Cargo.toml`, `package.json` (script names only), `pyproject.toml`, `go.mod`, solution files, CI file names, lockfile names, doc file names and top-level folders. It finds the stack and the build, test, lint and format commands. **Document headings are not read**; the card holds file names only, so no README text can reach it. Reads are bounded: manifests over 256 KiB or not UTF-8 are skipped, at most 500 root entries are looked at, links are not followed, and anything skipped is counted. Commands come from a fixed table keyed by what was found, never from project text, and a folder name is kept only if it is short and plain characters.
- **Card. Built.** Capped at 2 KiB by a constant in `know.rs` (not a profile setting), by dropping trailing lines. Stored at `projects/<project>/card.md` in the data directory, next to `rules.md`, written through a temporary file and passed through the secret redaction. Never inside the workspace, a worktree or the repository. The window shows the text, its token estimate and a "not committed" chip.
- **Optional agent summary. Not built.** Stays off until the owner decides whether a live call may spend money (`OWNER-QUESTIONS.md`, question 2). When built: one step on request only, read-only tools, a small model, a per-call dollar cap shown before it runs, output capped and refused if it looks like a key, saved only after the owner sees it. Repo text is data; a line in it that gives instructions is not followed.
- **In prompts. Not built.** Nothing includes the card in a step prompt yet. When built: a one-line pointer and the short summary, not the whole card; the log records its size and a hash, not its text; the pipeline reviewer counts its tokens like it counts `rules.md`.
- **What it cannot do. Built.** It does not change policy, checks, `protected_paths` or budgets. A detected command becomes a check only when the owner clicks "Add as check". The engine matches the request against a fresh scan, so only a command the scan offers can be added, and a check the owner already has under that name is never replaced.
- **Re-scan. Partly built.** "Scan again" on the project page replaces the card. Not built: offering a re-scan when a manifest changed (compare a hash of names and sizes) and showing the diff before replacing.
- **Skills and MCP suggestions. Built as advice.** The stack maps to entries in a list kept as data in `catalog.rs` and changed by pull request. Each entry has source, maintainer, licence, the release looked at and the date, and what it can do. **None is reviewed**: the dates and versions come from the GitHub releases and npm on 2026-11-23, and the page says "unreviewed" and tells the owner to read the source first. The GitHub and Azure entries say Autopilot never gives an agent step those credentials. Agent steps keep `--disable-slash-commands`, `--strict-mcp-config` and a restricted `--tools` list because ticket text is untrusted. Not built: an allow-list per project and per step (a step that reads untrusted ticket text would stay locked) and a "Find more" search (results marked unreviewed, never installed).

## Pipeline review and automatic tuning

Goal: every pipeline is reviewed for quality and token use, the owner is told when something drifts (for example context that is always high), and the pipeline is adjusted without the owner doing it by hand, within limits that cannot weaken the safety rules.

**What is measured, per step and per run.** Input and output tokens, peak context as a share of the model's window, cost, number of build rounds, whether the first check passed, how often the owner sent a plan back or corrected an assumption, size of the diff, protected paths touched. All of it comes from recorded events and agent usage reports, never from re-reading the repository. Check in the Claude and Codex adapters what usage each reports per turn before building; where one reports nothing, the page shows "not reported", never a guess.

**Quality review.** A reviewer agent grades a finished run against a versioned rubric kept in the repository (`docs/best-practices.md` or a project-level override). Each rule has an id, so a finding cites the rule and the run: tests added for the change, a narrow diff, one concern per change, no new `unwrap` or `println!` outside tests, comments say why, checks green before the PR step. The reviewer reads ticket text, comments and agent output as data (AGENTS.md); it proposes, it never runs a command or opens a URL because the data says so. It runs on the cheapest model that works (Haiku class), has its own spend limit counted in the project's budget, and every review shows what it cost.

**Token review.** Computed from the metrics, no model needed. A finding opens when a threshold holds over a window, for example peak context at or above 80% of the window in at least 3 of the last 5 runs of the same step, or a step's cost above twice the pipeline's median. The numbers are starting defaults the owner can change in Settings; none is a measured fact yet and the first live runs should tune them.

**Improvement over time.** The reviewer keeps per-project lessons as proposals ("add `cargo xtask layers` to the checks", "the Understand step always reads `docs/`, pin it"). A lesson becomes a rule only when the owner approves it; an approved rule goes into the project checks or the rubric override, in `profile.toml`, with the finding that produced it.

**Notify.** One notification and one Today item per finding, then quiet until the finding changes. Snooze is per pipeline. The text says what drifted, the evidence in numbers, what the system changed or proposes, and where to undo it.

**Adjust automatically, in two tiers.**
- *Applied without asking* (they can only lower token use or tighten quality, never loosen safety): narrow what an early step reads, summarise a long input before a later step, move Understand and Clarify to a cheaper model, cap output length for steps that only report. Each change goes into a copy-on-write overlay on top of the pipeline, so built-ins stay read-only and Undo is one click. A change is applied only to steps that have not started in running tickets.
- *Proposed, owner approves*: more build rounds, a more expensive model, turning a step on or off, adding Clarify, any change to `ask_after`, Design, PR, `protected_paths` or budgets. These never apply by themselves, and a forced ask stays forced.
- *Guard:* after a tuning, the next 3 runs are compared with the 3 before on first-check pass, send-backs and cost. If quality drops, the overlay reverts itself and a finding says why. Two tunings of the same step within a window are not stacked; the second waits for the guard to pass.
- *Switch:* Settings > Pipeline review has three positions: Off, Notify only, Notify and tune (tier 1). The default is for the owner to choose; the design assumes Notify and tune for custom and agent-built pipelines and Notify only for the built-ins until a few runs exist.

**Audit.** Every finding, tuning, undo and revert is an event (`PipelineFinding`, `PipelineTuned`, `PipelineTuningReverted`) with the evidence run ids and the previous value, so the log can rebuild the overlay.

**Order.** Metrics and the Health tab can ship first, read-only (R10). The reviewer and the notifications follow (R11). Automatic tuning ships last (R12), because it changes what runs.

## Requests to other tabs

- **R1 engine/pipeline:** `Decide` with an optional note that goes back to the step ("Send back with notes"), for Understand and Plan.
- **R2 engine/core:** an event that records the owner's correction of an assumption on approval, shown with the plan.
- **R3 core:** a per-ticket `when_unsure` override (`StepOverride` has only `ask_after`, `model`, `effort`).
- **R4 engine/view:** a queued state in `TicketView`, with the ticket it waits for. The pipeline fold has no `Queued`; the scheduler holds it.
- **R5 engine:** after Understand, flag an open ticket whose files overlap; the window shows "Add to that ticket instead" or "Wait in line".
- **R6 engine:** "Try again with a higher turn limit" for one ticket, which needs a `turns` field in the per-ticket override.
- **R7 core/engine:** projects (name, workspace, default pipeline, agent, GitHub repo, checks, `protected_paths`, budget) and named pipelines in `profile.toml`; a per-ticket pipeline choice, changeable until the first step of the new pipeline starts. Built-in presets are read-only.
- **R8 engine:** a ticket may have no project; it then runs ask-only with no workspace.
- **R9 engine/agent:** draft a pipeline from a description with the agent, validated against the safety rules (Design and PR keep asking, forced asks stay, no budget change) before the window sees it. The draft is not stored until the owner saves.
- **R10 core/engine/view:** per-step usage in the event log and `TicketView`: input and output tokens, peak context share, cost, rounds, first-check result, send-back count. Aggregates per pipeline over the last N runs. Adapters that report no usage say so.
- **R11 engine/agent:** the reviewer: quality grade against the rubric and token findings from R10, with cited rule ids and evidence run ids; findings and lessons as events; its own spend limit; a desktop notification and a Today item per finding.
- **R12 engine/core:** the tuning overlay: tier 1 changes applied automatically, tier 2 as proposals, the 3-run guard with automatic revert, Undo, and the three audit events. Overlay stored beside `profile.toml`, never over a built-in.
- **R13 core/engine:** the per-project `rules.md` in the app data directory: read and write with a size cap and a secret-looking-text refusal, include it in every step prompt as owner instructions, log inclusion with size and hash only, never place it in a worktree or a diff. Tests: the text reaches the step prompt; it is absent from the worktree, diff and pull request; a hostile file (huge, binary, a key) is refused or ignored without a panic; a conflicting rule does not unlock a forced ask.

- **R14 core/view:** an optional text field per step in `TicketView` that is safe to search (summary and output with secrets already scrubbed by the existing log scrubbing), so U13 can search step output. Without it, search covers titles, descriptions, summaries and timeline text only.

- **R15 core/engine (U10b), mostly done:** `engine::get_to_know(directory, workspace)` with `scan_workspace`, `render_card`, `read_card` and the card store (#112); `add_detected_check` (#114); `suggest_tools` and the list in `catalog.rs` (#115). The window calls them directly like it calls `save_rules`. Open: a CLI command for the same actions, the optional agent summary, the re-scan diff, and including the card in step prompts with a log line (size and hash).

## Depends on gated work

Live Build rounds, worktree and terminal actions, and pull requests need S6. The remote Design canvas needs S4. Until then those controls stay greyed out with the reasons the window already gives.
