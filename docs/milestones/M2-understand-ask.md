<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M2: understand, ask, inbox, step settings

**Owner crates:** `pipeline`, `policy`. **Depends on:** M1. **Gate for:** M3, M5.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M2-understand-ask.md`."

## Goal
A ticket goes through Understand, an optional Clarify and a Plan stub, and stops exactly where it should ask.

## Tasks
- [x] Step machine in `pipeline`: states idle, running, asking (`Needs you`), skipped(reason), passed, failed, paused, interrupted; transitions only through events; the state is `fold(events)`. `queued` is a scheduler fact with no event behind it, so the scheduler holds it and the fold reports such a step as idle. Verified against history written by the real engine for a pass, a limit stop and a failure.
- [ ] Understand schema (via `--json-schema`, generated with `schemars`): a map of 10 ambiguity categories each `clear|partial|missing` (functional scope, domain and data, interaction and UX, non-functional, integrations, edge cases and failures, constraints and trade-offs, terminology, completion signals, misc), at most 5 `open_questions` (each with 2 to 5 options and a recommended answer), `assumptions`, `needs_design`, `ui_files`, `files_touched`.
- [ ] Clarify: asks through the inbox; accepting "recommended" is one action; records `Q → A` pairs in the ticket. No open questions means Clarify does not run (`StepSkipped{reason: "no open questions"}`).
- [ ] Inbox: mirror durable requests as owner-only `<id>.request.json` files and accept atomically written `<id>.answer.json` files. Support `Question` (choices, free text) and `Approval` (approve, edit, reject, and project-scoped allowance for optional confirmations). Apply each pending answer once and clean up files after the database commit. Forced confirmations remain specific to the requested action.
- [ ] `policy`: forced confirmations (protected path, risky tier, security label, exceeded budget, suspicious diff) that make a step ask even if "Ask me after this step" is off; the checkbox adds confirmations but can never remove a forced one; `when_unsure` (`ask` | `decide_and_tell`).
- [ ] Per-step "Ask me after this step": after the step finishes, show its output and wait for approve, edit or reject.
- [ ] Pause and resume: store a restricted provider resume reference, repeat captured launch settings on explicit resume, and show interrupted attempts as `Interrupted`.
- [ ] Code-made "In short" lines (templates over structured results) and an on-request "Explain in plain words" (small model, once, cached in the log).
- [ ] Profile overrides: `SettingsOverridden{scope: ticket|project}`; a change applies when the next step starts, never to a running step.
- [ ] CLI: `autopilot inbox list|answer|approve`, `autopilot status --json`.

## Tests that must exist
- Policy table: every forced-confirmation rule asks; the checkbox cannot loosen any rule; `decide_and_tell` still records assumptions.
- Skip reasons: no open questions skips Clarify; no UI change skips Design (snapshot of the reason text).
- Inbox: request written, process killed, restart, answer applied exactly once.
- Resume after kill continues the saved session with all flags repeated.
- Settings override takes effect at the next step start only.

## Exit criteria
A real ticket runs Understand, asks (or skips Clarify), reaches the Plan stub, and stops where it should, driven from the CLI, with the full history replayable from the log.
