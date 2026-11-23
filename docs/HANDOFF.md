# Autopilot Mode On implementation handoff

Snapshot verified on 2026-11-21 (end of the session that merged PRs #55 to #60). The implementation repository is `D:/autopilot-mode-on` on Windows. The original planning copy is `D:/contest-keeper/pipeline-scratch/autopilot-mode-on` and is out of date. Use the standalone repository for implementation and milestone status. [GitHub repository](https://github.com/robbertpopa/autopilot-mode-on)

Start with [AGENTS.md](../AGENTS.md), the **Next session starts here** section of the [implementation plan](IMPLEMENTATION-PLAN.md), and the milestone file for the work you pick up. This file and `WORK-PACKAGES.md` are session notes: keep them untracked. `WORK-PACKAGES.md` has the package status and the full open list.

## Resume here

1. **Step reset is done (PR #61)** for Understand: `Event::StepReset { and_after }` and `Event::Described { text }` (both additive; old logs replay unchanged), `engine::reset_step`, `engine::rerun_understand`, `autopilot reset`, window "Try again" and "Re-run this step". The other steps cannot run again until real agents are allowed (S6).
2. **Worktree path:** nothing made from the window creates a worktree yet, so "Open the worktree" and "Open in a terminal" stay greyed out. Decide how to record the path when Build can run for real.
3. **Go live** (owner steps, see `WORK-PACKAGES.md`): `develop` to `master` PR, repo secrets `UPDATE_SIGNING_KEY` and `RELEASE_PLEASE_TOKEN`, Pages source "GitHub Actions". Nothing is published yet; no release exists. Do not set secrets or merge to `master` without the owner's go-ahead.
4. **Gated work:** real Plan, Build and pull request runs need S6; CI repair needs S3; the remote Design step needs S4; the Codex adapter needs S8; measured review quality needs S9. S2 still needs real subscription-limit evidence.
5. **Not built:** MCP server, GitHub App auth, Evaluation (M7), Review (M9), desktop notifications, full keyboard navigation and accessible names, standing "allow for project" approvals, "Explain in plain words".

## What is already implemented

| Area | Verified checkpoint |
| --- | --- |
| M0 foundation | Rust workspace, typed pipeline events and profile settings, SQLite event history and replay, `xtask` quality checks and layer rules. |
| M1 to M4 engine | Claude adapter (Understand, Plan, Build), checks and refine loop, scheduler, tracker, diff scan, protected-branch guard, publish path, CI watch, `autopilot` CLI. Real agents stay refused until S6. See `WORK-PACKAGES.md` for the PR list. |
| Step control | Stop, pause and resume through flags checked between turns; `engine::follow`. |
| `engine::view` | Display models for the window: `ticket_view`, `tickets`, `spent_since`, API cost kept apart from subscription estimates. |
| `engine::profile_edit`, `engine::change` | In-place `profile.toml` edits through `toml_edit` that re-parse before writing; plain-words change requests read by a fixed grammar, shown as a diff, applied to the project or to one ticket. |
| `crates/accounts` | GitHub (every account, active marked), Claude, Codex and Azure status, no tokens. |
| **Window** (`crates/ui`, binary `autopilot-ui`) | egui/eframe 0.33. Home screen (start box, waiting for you, tickets), ticket page (status card, step chips, history, plain-words settings, Pause/Stop/More), step detail with tabs, Settings pages (Pipeline steps, Cost, Accounts), dark and light theme remembered in `ui-theme.txt`, `--demo` sample tickets, flags `--data-dir --profile --view --ticket`. Starting a ticket runs the real engine with the fixture agent through Understand. Controls not built are greyed out and marked coming soon. |
| **Installers and release** | `packaging/windows/installer.nsi`, `packaging/macos/build-dmg.sh`, `packaging/manifest.sh`, workflows `release-please.yml`, `release.yml`, `pages.yml`, static site in `site/`. Asset names are fixed: `autopilot-mode-on-setup-x64.exe`, `autopilot-mode-on-macos-arm64.dmg`, `autopilot-mode-on-macos-x64.dmg`. How it works: `docs/releasing.md`. **None of it has run for real** (no release yet). |
| **Updates** (`crates/update`, layer 2) | The window checks at start and every 6 hours: fetches `latest.json` and `latest.json.sig` through the system `curl` (HTTPS only), verifies an ed25519 signature with the built-in key, offers only a strictly newer plain `x.y.z`, downloads, checks SHA-256 from the signed manifest, then opens the installer. `AUTOPILOT_NO_UPDATE_CHECK=1` turns it off. Tests cover signature, version, asset vetting and checksum, not the network. |
| **macOS** | `crates/process/src/macos.rs` (process group, no guardian), `git` fingerprints on unix, `macos-latest` in CI. Build and all tests pass on a Mac runner. The window and the `.dmg` have never been opened on a Mac. |
| Layer rules | `tools/xtask/src/layers.rs`: `autopilot-update` is layer 2 and may use `autopilot-process`; egui only in `autopilot-ui`; `ui` is layer 6. |
| `deny.toml` | Allow-list adds BSL-1.0, Unicode-3.0, Zlib, and a licence exception for `epaint_default_fonts` (Apache-2.0, MIT, OFL-1.1, Ubuntu-font-1.0). Flagged in PR #55 as a licence-policy loosening. |

## Live findings that affect later work

- A schema from `schemars` carries `$schema: .../2020-12/schema`, which Claude Code rejects. It is stripped.
- A structured-output run takes two turns, so `--max-turns` must stay above 1 (the CLI default is 4).
- An unsatisfiable schema ends as `success`, exit 0, with no `structured_output`.
- A limit stop has `is_error: true` and exit 1; its cost is still reported and the response that crosses a budget cap is still delivered and billed.
- `--permission-mode dontAsk` with `--tools Bash` still ran a read-only command. Do not treat it as a deny list. A `PreToolUse` hook passed with `--settings` did block `git push`, and the denial shows in the stream and in `permission_denials`.
- `--tools ""` removes built-in tools but not MCP tools from the account; `--strict-mcp-config` is what removes those.
- API-key runs emit no `rate_limit_event`; with the key set, claude.ai connectors are disabled and stderr says so.
- `claude auth status --json` reports `authMethod: claude.ai` for a subscription and `api_key` when `ANTHROPIC_API_KEY` is set.
- This PC sets `CLAUDE_CONFIG_DIR`, so it must stay on the child environment allowlist.
- egui details: a Frame inside `ui.horizontal` needs a `ui.vertical` wrapper; Frame content needs `set_width(available - 34)`; `●` and several arrows do not render in the default font (use `▶`, `×`, `✔`).
- On macOS, `killpg` on a group whose only member is an exited, unreaped leader returns EPERM, not ESRCH. The backend treats both as an empty group.
- Earlier spike notes still stand: S2 retry behaviour, S6 Windows MXC containment results, S4 Design preflight, S8 Codex auth status, S9 review corpus. See `docs/spikes/`.

## Git, accounts and verification

- `develop` is at `c83b5e7`. `master` is 60 commits behind and has the old state. There are no open PRs. `docs/HANDOFF.md` and `docs/WORK-PACKAGES.md` are the only untracked files.
- **Ask which GitHub account to use at the start of each prompt that will commit, push or run `gh`** (`robbertpopa` or `sheila-dot`), as the workspace CLAUDE.md requires. The owner has chosen `robbertpopa` every time so far, but the answer does not carry over. After the answer: `gh auth switch --user <account>` and set the repo's local `user.name` and `user.email` (`95902235+robbertpopa@users.noreply.github.com` or `270025227+sheila-dot@users.noreply.github.com`).
- The default Git credential helper selects `sheila-dot` and gets a 403 on this repo. Push and pull with the command-local helper:

  ~~~powershell
  git -c credential.helper= -c 'credential.https://github.com.helper=!gh auth git-credential' push -u origin HEAD
  ~~~

  (In Git Bash: `git -c credential.helper= -c "credential.helper=!gh auth git-credential" push ...` worked.)
- Flow used for every PR: branch from current `develop`, commit with a `type: lowercase imperative subject` title and **no** `Co-Authored-By` or "Generated with" lines (AGENTS.md overrides the default attribution reminder), open the PR, wait for the hosted checks (now Windows, Linux and macOS), squash-merge, delete the branch. The owner asked for PRs to be completed each time ("implement, create PR, review, complete, next, don't stop").
- Local checks that work: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo xtask layers`. Not installed here: `cargo-nextest`, `gitleaks`, `cargo-deny`, `cargo-machete`, so `cargo xtask ci` cannot run locally; the hosted CI runs them. A macOS type-check works from Windows for crates without C dependencies: `cargo check -p autopilot-process -p autopilot-git --target aarch64-apple-darwin` (the target is installed). The sqlite crate cannot be cross-compiled from here.
- Rust is pinned to `1.99.1`. Windows is primary; Linux and macOS must pass too.
- Editing tips on this PC: bash heredocs break on apostrophes and backslashes, so write scripts and file content with the Write tool; `git` warns about CRLF on tracked files, which is harmless.

## Installed copy and testing the window

- The owner's installed copy is a manual copy in `%LOCALAPPDATA%\Programs\Pipeline` with a Start Menu shortcut "Autopilot Mode On"; its working folder and data are in `%LOCALAPPDATA%\Pipeline`. It is not from the NSIS installer and has no uninstall entry. Rebuild with `cargo build --release -p autopilot-ui`, stop the running copy, copy `target\release\autopilot-ui.exe` over it.
- A screenshot script (`shot.ps1`, PrintWindow based, DPI aware) lives in the session scratchpad and may be gone. It starts `target\debug\autopilot-ui.exe --demo --view <screen>` and saves the window only.
- The signing key for updates is `C:\Users\r.a.popa\.autopilot-update-signing.pem`. Never print it or commit it.

## Live testing with Claude

- The dedicated API key is in `D:\contest-keeper\pipeline-scratch\autopilot-mode-on\claude_key.txt`, outside the product repo but inside another git repo (excluded locally). Prefer moving it outside any repo. Never print it. Load it into `AUTOPILOT_ANTHROPIC_API_KEY` inside the command that needs it.
- Budget rule: $1 in total, Haiku, `--max-budget-usd 0.05` per call. About $0.0025 was spent in the S1 run. Subscription calls use plan quota. Ask before any spend beyond that.
- Run live calls from a disposable directory. A ticket can run once per data directory, so use a fresh one each time.
- A parent shell that is itself a Claude Code session carries `CLAUDE_CODE_*` and `CLAUDECODE` variables. The adapter strips them with its allowlist. Raw captures made by hand should strip them too.
- `D:\msg.txt` is a file for the owner. Do not delete it.
