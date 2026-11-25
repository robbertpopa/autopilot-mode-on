<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# The window (egui)

`autopilot-ui` is the desktop window. The visual design comes from a private Claude Design canvas that is not part of this repository; the code in `crates/ui` is the reference for what each page shows. The style is a graphite shell with a ticket sidebar, status chips, amber approval cards, a step list and a cost bar.

Start it with `cargo run -p autopilot-ui -- --data-dir <dir> --profile <file>`. `--demo` fills an empty data directory with sample tickets for every state, and `--view <name>` opens on one page (`today`, `new`, `plan`, `board`, `projects`, `pipelines`, `health`, `review`, `settings`, `design`, `checks`, `look`, `sandbox`, `cost`, `accounts`, `run`, `step`; run `autopilot-ui --help` for the current list). The first time it opens with no tickets it shows a full-screen welcome: Agents, Tools, Environment, Look. A copy started from a bare binary needs `autopilot-process-guardian` and `autopilot-fake-agent` next to it; the disk image and the installer put them there.

## Pages and what drives them

| Page | Shows | Driven by |
|---|---|---|
| Sidebar | New ticket, every ticket grouped by project or status with a live chip (running, needs you, queued, CI failing, interrupted, done), the spend meter, one icon per account and the settings gear | the folded state of each ticket's events |
| Today | what needs you now: inbox questions and approvals, tickets that moved, tuning proposals with Undo | request and answer files, the event stream |
| New ticket | the composer: text, project, pipeline; the window drives Understand, Clarify, Design and Plan by itself | `engine` start actions |
| Plan review | two panes: the criteria and the decision beside them | the approved-plan record |
| Ticket (Run) | a live strip of every step with skip reasons; the Build and Check panel with each round, each check's note and cost, and the review's findings (`path:line [kind, confidence%] issue`); timeline; cost split | the event stream, replayed when the window opens |
| Ticket (Step) | tabs Overview, Input, Prompt, Output, Log, Changes, Cost, Settings; per-step chat; "In short" line and an on-request explanation. Input, Prompt and Changes say when they were not recorded | the log of that step |
| Design | the mockups a Design step saved, opened in the system browser from the approval | `engine::design` |
| Board, Projects, Pipelines | all tickets by state; project cards from a read-only scan; the built-in and custom pipelines with Simple and Advanced views | `profile.toml`, the project card store |
| Health, Review | pipeline health and cost from real tickets; tuning proposals the owner approves or dismisses | `engine::metrics`, `engine::proposals` |
| Settings | Asking, Checks, Accounts, Sandbox, Look. Sandbox runs the build probe on a button, lists every row and the commands that fix what is missing | `autopilot_agent_claude::sandbox_boundary`, the same path Build uses |
| Ask for a change | a change to the pipeline read by a fixed grammar, shown as a diff of `profile.toml`, applied only when you press Apply, to this ticket or to all future ones | `engine::change` |

Build, Ask to open a pull request, Open the pull request and Open the worktree are wired to the engine. Open in a terminal (More menu, step page) opens a terminal in the ticket's copy of the project. Reset the Build (More menu, Build step page) asks first, then deletes that copy and its branch and puts Build back to idle; it is refused while a step runs or waits for you and once a pull request was asked for. Shown off with the reason: resuming the provider's session in a terminal.

## Rules

- The window is a thin client: no process spawning of its own, no file writes outside the engine. Every action is an `engine` action that the CLI also exposes and tests.
- Repaint when an event arrives, not every frame.
- "In short" lines come from code templates and cost nothing. "Explain in plain words" asks a small model once, only on request, and keeps the answer in the log.
- A step that is asking is amber. A skipped step is dashed with its reason. A forced confirmation shows why ("asks anyway: protected path").
- Notifications: a ticket needs you, fails or finishes (desktop notifications through `notify`).

## Known gaps

The open window bugs are tasks W11 to W16 in [stream 4](work-streams/04-window-and-orchestration.md). On macOS the welcome screen still prints the Windows command `setx` for the API key (set the environment variable in your shell instead, see the [README](../README.md#real-build-needs-the-sandbox-and-an-api-key)). Full keyboard navigation and accessible names are not done (W9).
