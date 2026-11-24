<p align="center">
  <img src="assets/icon.svg" alt="autopilot-mode-on icon" width="128">
</p>

# autopilot-mode-on

A local program, written in Rust, that takes a codebase and a pile of stories, bugs and wishes, and runs each one through a pipeline of coding agents
until a draft pull request is ready for you to review. It asks you only when it needs something.

Status: a ticket goes from Understand through Plan, Build and the checks to an approval request for a draft pull request, from the command line and from the window (egui). A real agent builds only inside a validated sandbox: WSL2 on Windows, directly on Linux, and `srt` with Seatbelt on an Apple silicon Mac. **No real pull request has been opened on GitHub yet**; the pull request path has run against a local bare repository and a fake `gh`. Milestone status is in the [implementation plan](docs/IMPLEMENTATION-PLAN.md).

## What it does

```
ticket -> Understand -> [Clarify] -> [Design] -> Plan -> Build <-> Check -> Pull request (draft)
```
- Steps a ticket does not need are skipped, with a stated reason. The AI decides whether it has anything to ask.
- There are no gates. A step asks you in an inbox when it needs a confirmation or more information, and every step has an "Ask me after this step" checkbox.
- Each step runs a coding agent (Claude Code; Codex is planned) as a headless child process. Build runs in its own git worktree. Subscription sign-in is the default for the steps that read; Build and Check inside the sandbox use an API key only, because subscription mode is refused there (see [docs/security.md](docs/security.md)).
- Build and Check loop: checks run cheapest first and stop at the first failure; the failed note goes back to the build. After the checks, up to four fresh reviewers read the change and only findings they are sure of count.
- You watch every ticket live: an event log drives a window (egui), a CLI and an MCP server.
- The tool never merges, never pushes to a protected branch, and the agent never holds the GitHub token. `autopilot pr watch` reads a published pull request and names the one next action for it; it changes nothing.

The roadmap also includes a read-only codebase review, followed by approved fixes through the ticket pipeline. See the [review pipeline design](docs/REVIEW-PIPELINE.md).

## Install

Download the Windows installer or a macOS disk image from the [releases page](https://github.com/robbertpopa/autopilot-mode-on/releases/latest). The app checks for a signed update when it opens. How releases are built and signed: [docs/releasing.md](docs/releasing.md). No release has been published yet; until then build from source (below).

- **Windows:** run `autopilot-mode-on-setup-x64.exe`. It installs per user and needs no administrator rights.
- **macOS:** open `autopilot-mode-on-macos-arm64.dmg` (Apple silicon) or `autopilot-mode-on-macos-x64.dmg` (Intel) and drag the app to Applications. Without Apple signing secrets the app is signed ad hoc: right-click it and choose Open the first time. The sandbox was validated on Apple silicon only.
- **From source on a Mac:** `cargo build --release -p autopilot-ui -p autopilot-process -p autopilot-fake-agent`, then `packaging/macos/build-dmg.sh <version> target/release/autopilot-ui assets/icon-512.png out.dmg target/release/autopilot-process-guardian target/release/autopilot-fake-agent`.

### First run

The window opens a welcome once: pick an agent (it reads `claude auth status`, `gh auth status`, `az account` and Codex's own status, and never stores a sign-in), connect your Git identity and GitHub or Azure DevOps account, choose the project folder tickets work in (optional), and pick a theme. Sign in with the provider's own tool, for example `claude auth login`.

### Real Build needs the sandbox and an API key

Build and Check run a real agent only after an escape probe passes on your machine. Set up the sandbox once, then check it:

| Host | Set up | Needs first |
|---|---|---|
| Windows | In a WSL2 Ubuntu distro: `wsl -d Ubuntu -u root -- bash tools/wsl/setup-root.sh`, then `wsl -d Ubuntu -- bash tools/wsl/setup-user.sh` and `... install-hook.sh` | WSL2 with Ubuntu |
| Linux (x86_64) | The same three scripts, run directly (`sudo bash tools/wsl/setup-root.sh`) | nothing |
| macOS (Apple silicon) | `tools/macos/setup.sh` installs the pinned `srt` and the deny hook under `~/.autopilot` | Node 20 or newer, Claude Code and a Rust toolchain already installed |

The Linux scripts install Node, the pinned `srt`, Claude Code and the Rust toolchain under the user's home.

```sh
cargo run --locked -p autopilot-cli -- --data-dir "$HOME/autopilot-data" isolation check
```

The command prints every probe row and exits with an error if one fails. Pass an absolute `--data-dir` outside `/tmp`: a relative path is refused and a data folder under `/tmp` fails the macOS probe on purpose. The window shows the same result under Settings, Sandbox.

The API key comes from the environment variable `AUTOPILOT_ANTHROPIC_API_KEY` (the ordinary `ANTHROPIC_API_KEY` is never read, so a key you use elsewhere is not spent by accident). The app never stores or shows it; inside the sandbox the agent holds only a placeholder. On Windows `setx AUTOPILOT_ANTHROPIC_API_KEY <key>` saves it for the user and the app must be restarted. On a Mac the app reads the variable from its own environment, so export it in a shell and start the app from that shell (`"/Applications/Autopilot Mode On.app/Contents/MacOS/autopilot-ui"`); a copy opened from Finder does not see shell variables. Details and limits: [docs/security.md](docs/security.md) and [S6](docs/spikes/S6-containment.md).

## Documents

| File | What it is |
|---|---|
| `AGENTS.md` | Rules for any agent or person working in this repository. Read first. |
| `docs/IMPLEMENTATION-PLAN.md` | The master plan, status table and how to work in several tabs. |
| `docs/DESIGN.md` | What we build and why. |
| `docs/decisions.md` | Decisions made, with dates. |
| `docs/milestones/` | One file per milestone M0 to M9, with tasks, tests and exit criteria. |
| `docs/spikes/` | One file per spike S1 to S10: questions to answer before dependent work starts, and their results. |
| `docs/work-streams/` | The four parallel work streams, the orchestrator guide and the owner's scenarios. |
| `docs/testing.md` | The testing standard for this repository. |
| `docs/security.md` | Threat model, rules and the containment status per platform. |
| `docs/pitfalls.md` | Past mistakes and traps found in other projects. |
| `docs/ui.md` | The window: pages and the rules it follows. |
| `docs/headless-contract.md` | What Claude Code's headless mode emits and accepts, from recorded runs. |
| `docs/REVIEW-PIPELINE.md`, `docs/rules/` | Design of the codebase review and its rule packs (M9). |
| `docs/releasing.md` | How a release is built, signed and updated. |

## Try the fixture-backed CLI

Run these commands from the repository root. The fake agent replays a fixed synthetic transcript, and the result and event history are saved in the selected data directory.

```sh
cargo build --locked -p autopilot-process -p autopilot-fake-agent -p autopilot-cli
cargo run --locked -p autopilot-cli -- --data-dir target/fixture-demo run-step --step understand --ticket fixtures/tickets/understand.json --agent fake
cargo run --locked -p autopilot-cli -- --data-dir target/fixture-demo watch sample-export
```

Add `--after 3` to `watch` to replay only events after sequence 3. `watch` reads committed events and exits. Use a fresh data directory to run the sample ticket again.

## Development

Run `cargo nextest run --workspace` for the local test suite (`cargo test --workspace` also works, but some window tests are flaky when they share one process). `cargo xtask ci` also checks formatting, Clippy, crate layers, dependency licenses, advisories and unused dependencies. The CI workflow pins Nextest, cargo-deny and cargo-machete. Start the window with `cargo run -p autopilot-ui -- --demo` to see every page filled with sample tickets.

Run `cargo xtask secrets` with Gitleaks installed to scan staged changes before committing.

## Licence

Apache-2.0, see `LICENSE`. Dependencies must be permissive (MIT, Apache-2.0, BSD, ISC), enforced by `cargo deny`.
The names "Claude" and "Claude Code" are Anthropic's; this tool launches the user's own `claude` binary and uses neither in its name.
