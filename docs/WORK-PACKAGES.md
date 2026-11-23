# Work packages (session note, untracked)

Each package is one branch and one PR into `develop`. A package is ticked when its PR is merged and the milestone boxes it covers are ticked. Updated 2026-11-21 at `develop` = `95095c0` (#61).

## Status

- [x] 1 M2 finish: step control (PR #41)
- [x] 2 M1 close-out (PR #42)
- [x] 3 Plan and Analyze (PR #43)
- [x] 4 Checks and Refine (M3, PR #44)
- [x] 5 Build step (M3, engine half, PRs #45 and #46). Open: the agent-backed `Reviewer`; real runs need S6
- [x] 6 Scheduler (M4, PR #47)
- [ ] 7 Tracker and CI (M4). Merged: tracker crate (#48), diff scan (#49), protected-branch guard (#50), publish path (#51), `autopilot publish` (#52), CI watch (#53). Open: CI repair (needs S3), a run against a real sample remote, writes outside the worktree (need S6)
- [ ] 8 Window core (M5). Merged: stop, pause, resume, `engine::follow` (#54). Open: re-run and re-run from here (need a step-reset event), open worktree and terminal, explain
- [x] 9 Window views (M5, PR #55): egui window `autopilot-ui` with pipeline design, run, step detail, accounts, cost, dark and light themes, theme memory, demo data (`--demo`). Graph widget: the strip of step chips stands in for it
- [x] 10 Window polish (M5), mostly: "Ask for a change" (PR #56), repaint only on a changed snapshot, Ctrl+N and Ctrl+Enter. Open: full keyboard navigation and accessible names, desktop notifications
- [x] 10b Window simplification (PR #59): home screen with a start box and what waits for you, one status card per ticket, one row of step chips, collapsed history and plain-words settings, settings pages (steps, cost, accounts) behind one tab row, controls not built are marked coming soon
- [x] 10c Installers, website, release pipeline, updates (PR #57): Windows NSIS installer, macOS `.app` and `.dmg` script, `release-please.yml`, `release.yml`, `pages.yml`, static `site/`, `crates/update` (signed `latest.json`, SHA-256 check, one-click update), `docs/releasing.md`
- [x] 10d App name (PR #58): everything user-facing says Autopilot Mode On
- [x] 10e macOS support (PR #60): `process` macOS backend (process group, no guardian), `git` fingerprints on unix, `macos-latest` in CI. Whole workspace builds and tests pass on a Mac runner
- [x] 10f Step reset (PR #61): `StepReset` and `Described` events, `engine::reset_step` and `rerun_understand`, `autopilot reset`, window Try again and Re-run for Understand. Re-run for Plan, Build and Pull request needs real agents (S6)
- [x] 10g Attachments and composer (PRs #62, #63): `Attached` event, `engine::stage`/`attach` (25 MiB per file, 100 MiB and 20 per ticket, folders by reference, links refused), drop and native pickers (Windows, macOS), Files list, Inter font, centred column. No agent reads attachments yet (S6). Settings pages and step detail not rechecked visually after the restyle
- [x] 10h Real Understand and Plan in the window (PRs #64, #65): agents get a data-only note of attachments, `--attach` in the CLI, Practice/Claude switch (subscription, haiku, no tools), `plan_ticket` and a Make the plan button, helper binaries shipped with installers (not built locally: no NSIS or Mac). Tested live: Understand, Clarify, Plan with real Claude. Not tried live: API-key auth from the window, Retry/Stop with a real run, Build (still S6-gated)
- [ ] 11 Design step (M6): local HTML mockups first; the remote Claude Design path needs S4
- [ ] 12 Evaluation (M7): corpus, runner, judge, report, experiments, budget guard
- [ ] 13 Codex adapter (M8): auth rule, `agent-codex`, auth scrubbing, sandbox, cost, profile `agent` field. Needs S8. Only the Codex line in `accounts` exists
- [ ] 14 MCP and GitHub App auth (M8): `mcp` crate, GitHub App auth. (The Windows installer and release workflow of this package are done in 10c.)
- [ ] 15 Review engine (M9): `rules` crate, inventory, mechanical scans, review, verify, triage. Needs S9 for measured quality
- [ ] 16 Review fix and CLI (M9): fix planning, fix, re-review, modes, `[review]` profile section, `autopilot review` CLI

- [ ] 17 Get to know a project (UI-REDESIGN U10b, R15). Merged: scan and card store (#112), card on the Projects page (#113), Add as check (#114), skill and MCP suggestions as advice (#115). Open: optional agent summary (needs the owner's answer on live spend), re-scan diff, card pointer in step prompts with a log line, CLI command, per-project tool allow-list

Packages left: 7 (partly), 8 (partly), 11 to 17.

## Going live (owner steps, nothing is published yet)

- `develop` is 60 commits ahead of `master`. The release and website workflows run from `master`, so open a `develop` to `master` PR first.
- Repository secrets: `UPDATE_SIGNING_KEY` (private key is at `C:\Users\r.a.popa\.autopilot-update-signing.pem`, public half is `PUBLIC_KEY_HEX` in `crates/update/src/lib.rs`) and `RELEASE_PLEASE_TOKEN`. Optional Apple secrets for signing and notarization.
- Pages source must be "GitHub Actions".
- No release exists yet. The first release is the first real run of `release.yml`, the NSIS installer, `build-dmg.sh` and the update flow.
- Never run on a Mac: the window itself and the `.dmg`. CI proves build and tests only.

## Still open inside finished packages

- The agent-backed `Reviewer` (check 5) and real `Round` runs against a repository: need S6.
- Diff size has no limit: the docs give no number and the profile has no field. Ask the owner before adding one.
- A `test` criterion passes only if its name shows in the combined stdout and stderr of the profile `test` command. Known hole: a test written without `#[test]` makes rustc warn "function `name` is never used" on stderr, the filter matches nothing, the command exits 0, and the name is in the output, so the criterion passes. Decide the fix with the first real runner (for example, require a result line shaped like `test <name> ... ok` or a nextest `PASS` line).
- Check commands run with an allowlisted environment. A real `cargo build`, `cargo fmt --check` and `cargo test` passed through it on the pinned GNU toolchain on this PC. The MSVC toolchain was not tried; its linker discovery may need more variables.
- Check tests that start processes are Windows-only, like the `git` crate tests, because the Linux runner needs the guardian binary that only the `process` crate builds for tests.
- M2 "In short" lines: the code-made lines are done. "Explain in plain words" (small model, once, cached in the log) is not built.
- Ask-after-step is wired for Understand and Plan only.
- The inbox records `allow_for_project` on approvals, but nothing stores or applies a standing allowance yet.
- Re-running: the step-reset event exists (#61) and works for Understand. Still open: re-run from here and "Run it anyway" for later steps, and Try again for Plan, Build and Pull request (all need real agents, S6). The CLI exposes `reset` but no re-run command.
- Open worktree and terminal: no run made from the window creates a worktree (Build needs S6), so there is nothing to open. Decide how to record the path when Build can run for real.
- The window's "Ask for a change" reads a fixed grammar (model, effort, turns, rounds, ask-after). Budget and spend words are refused on purpose; "review" has no separate step yet.
- macOS limits (in `crates/process/src/macos.rs`): a descendant that starts its own session survives a stop, and a killed supervisor does not clean up its worker. One process test is skipped on macOS for that reason.

## How this file is kept

1. Packages were grouped on 2026-11-21 from 63 open checkboxes across the milestone files, by dependency.
2. After each package the PR is merged and this file is edited: tick the package and rewrite anything whose scope changed. The tracked files (milestone checklists, the plan table) are updated inside each PR. This file and `HANDOFF.md` stay untracked.
3. To refresh the counts: `grep -c '^- \[ \]' docs/milestones/*.md docs/spikes/*.md`.
4. Session of 2026-11-21 added packages 9, 10, 10b to 10e (window, installers and updates, rename, macOS) and replaced the earlier plan to build the Windows installer under package 14.
