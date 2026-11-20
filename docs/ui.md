<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/icon-dark.svg">
  <img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# The window (egui)

The visual design lives in a private Claude Design canvas.
The local export in the original `pipeline-scratch/canvas/project/` folder contains `Main.dc.html` (pipeline), `Run.dc.html` (live run) and `Step.dc.html` (step detail). Carry the export into the standalone repository when starting M5. The style follows craft-anything's screens:
dark shell, ticket sidebar with status chips, amber approval cards, progress stepper, cost bar.

## Screens and what drives them

| Screen | Shows | Driven by |
|---|---|---|
| Sidebar | every ticket with a live chip (running, needs you, queued, CI failing, interrupted, done) | the folded state of each ticket's events |
| Pipeline (design mode) | the steps as nodes with settings (agent, auth, model, effort, limit, "Ask me after this step"); Simple and Advanced views | `profile.toml` through `engine`; the graph only edits the file |
| Run | a live strip of every step's state with skip reasons; the Build ⟲ Check panel (round, activity line, checks ticking); timeline; cost bars | the event stream, replayed when the window opens |
| Step detail | tabs Overview, Input, Prompt, Output, Log, Changes, Cost, Settings; actions Re-run, Re-run from here, Pause, Stop, Open worktree, Open in terminal (resume), Copy local session reference; "In short" line and an on-request explanation | the log of that step |
| Inbox | questions (choices, free text, a recommended answer) and approvals (Approve, Edit, Reject) | request and answer files |
| Ask for a change | targets: this ticket's code, the pipeline, just ask; a pipeline change is shown as a diff of `profile.toml` and applies only when you press Apply, to this ticket or to all future ones | a typed patch validated against the profile schema |

## Rules

- The window is a thin client: no process spawning, no file writes. Every action is an `engine` action that the CLI also exposes and tests.
- Repaint when an event arrives, not every frame.
- "In short" lines come from code templates and cost nothing. "Explain in plain words" asks a small model once, only on request, and keeps the answer in the log.
- A step that is asking is amber. A skipped step is dashed with its reason. A forced confirmation shows why ("asks anyway: protected path").
- Notifications: a ticket needs you, fails or finishes. A "stalled" flag when a running step is silent too long.
