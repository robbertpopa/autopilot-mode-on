# autopilot-mode-on

A local program, written in Rust, that takes a codebase and a pile of stories, bugs and wishes, and runs each one through a pipeline of coding agents
until a draft pull request is ready for you to review. It asks you only when it needs something.

Status: the fixture-backed Understand CLI runs on Windows and Linux. Follow the [implementation plan](docs/IMPLEMENTATION-PLAN.md) for milestone status.

## What it does

```
ticket -> Understand -> [Clarify] -> [Design] -> Plan -> Build <-> Check -> Pull request (draft)
```
- Steps a ticket does not need are skipped, with a stated reason. The AI decides whether it has anything to ask.
- There are no gates. A step asks you in an inbox when it needs a confirmation or more information, and every step has an "Ask me after this step" checkbox.
- Each step runs a coding agent (Claude Code, later Codex) as a headless child process, in its own git worktree. Subscription sign-in is the default; an API key is an option per step.
- Build and Check loop: checks run cheapest first and stop at the first failure; the failed note goes back to the build.
- You watch every ticket live: an event log drives a window (egui), a CLI and an MCP server.
- The tool never merges, never pushes to a protected branch, and the agent never holds the GitHub token.

The roadmap also includes a read-only codebase review, followed by approved fixes through the ticket pipeline. See the [review pipeline design](docs/REVIEW-PIPELINE.md).

## Documents

| File | What it is |
|---|---|
| `AGENTS.md` | Rules for any agent or person working in this repository. Read first. |
| `docs/IMPLEMENTATION-PLAN.md` | The master plan, status table and how to work in several tabs. |
| `docs/DESIGN.md` | What we build and why. |
| `docs/decisions.md` | Decisions made, with dates. |
| `docs/milestones/` | One file per milestone M0 to M9, with tasks, tests and exit criteria. |
| `docs/spikes/` | One file per spike S1 to S9: questions to answer before dependent work starts. |
| `docs/testing.md` | The testing standard for this repository. |
| `docs/security.md` | Threat model and rules. |
| `docs/pitfalls.md` | Past mistakes and traps found in other projects. |
| `docs/ui.md` | The window: screens and where the visual design lives. |

## Try the fixture-backed CLI

Run these commands from the repository root. The fake agent replays a fixed synthetic transcript, and the result and event history are saved in the selected data directory.

```sh
cargo build --locked -p autopilot-process -p autopilot-fake-agent -p autopilot-cli
cargo run --locked -p autopilot-cli -- --data-dir target/fixture-demo run-step --step understand --ticket fixtures/tickets/understand.json --agent fake
cargo run --locked -p autopilot-cli -- --data-dir target/fixture-demo watch sample-export
```

Add `--after 3` to `watch` to replay only events after sequence 3. `watch` reads committed events and exits. Use a fresh data directory to run the sample ticket again.

## Development

Run `cargo test --locked --workspace` for the local test suite. `cargo xtask ci` also checks formatting, Clippy, crate layers, dependency licenses, advisories and unused dependencies. The CI workflow pins Nextest, cargo-deny and cargo-machete.

Run `cargo xtask secrets` with Gitleaks installed to scan staged changes before committing.

## Licence

Apache-2.0. Add the `LICENSE` file when the repository is created. Dependencies must be permissive (MIT, Apache-2.0, BSD, ISC), enforced by `cargo deny`.
The names "Claude" and "Claude Code" are Anthropic's; this tool launches the user's own `claude` binary and uses neither in its name.
