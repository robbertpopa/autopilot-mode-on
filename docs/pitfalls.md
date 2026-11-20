# Pitfalls: past mistakes and traps found in other projects

Add an entry whenever something cost time. One line each: the trap, then what to do.

## Claude Code headless
- `--output-format stream-json` needs `--verbose`. Parse every line as JSON; ignore unknown types and fields.
- A set `ANTHROPIC_API_KEY` outranks the subscription login in `-p`. For subscription steps remove it (and `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_*`) from the child's environment.
- `--bare` is key-only and skips CLAUDE.md, hooks and MCP. Do not use it for API steps that need the project's rules.
- `-p` with no permission mode starts in Manual mode (from v2.1.302): calls that need approval are denied. Give each step an explicit allowed-tools list.
- `--resume` does not restore `--mcp-config`, `--add-dir`, `--settings` or permission flags: pass them again.
- `--max-budget-usd` is a client-side estimate and ignores totals restored by resume.
- There is no exit code for a plan or rate limit. Watch for `system/api_retry` events and limit phrases in the result.
- Session transcripts under `~/.claude/projects` are an internal format. Never parse them.
- The docs do not say how to hide the console window on Windows. Spawn `claude.exe` directly with the no-window flag.
- Pass long prompts on stdin, not as an argument (Windows limits a command line to about 32,000 characters).

## Workspace checks

- Cargo-run binaries inherit package environment variables. Invoke `cargo-machete` directly from `xtask` so it interprets arguments as paths.
- Lock dependency resolution at the `cargo xtask` alias. An unlocked alias can rewrite `Cargo.lock` before the checks run.

## Process supervision

- Assign the Windows job during process creation so supervisor death during startup cannot strand a child waiting for a later assignment.
- On Linux, keep the group leader unreaped until the group receives its stop signal. A reaped leader's ID can be reused. The guardian remains in a separate group to clean up when the owner exits.
- Bound cleanup waits as well as execution waits. A descendant that retains a pipe can otherwise keep the caller blocked after the worker exits.

## Processes on Windows
- A child can outlive the parent. Use a Job Object with kill-on-close, and wait with a limit after cancelling.
- Deleting a worktree can fail on a file lock: retry with backoff.
- Use native backslash paths for external tools; avoid `.cmd` shims without PATHEXT handling.

## Loops and cost
- A failed build is a normal result, not an error: cap loops on the result's `ok` flag, not on exceptions. (craft-anything)
- Stop an agent whose stream has been silent too long (Gru uses 15 minutes) and turn it into "Needs you".
- Two turns that change nothing in the worktree stop the build.
- After two failed refine rounds, start a fresh session with a summary of what failed.
- Switching model mid-session throws away the prompt cache: route per session.

## UI (egui)
- Spinners and progress bars request a repaint every frame. Repaint on events and use paced widgets.
- The UI must not write files or launch programs; it only calls engine actions.

## Other tools' traps
- Gru's older design text and its current docs disagree on retry counts: trust the code and the newest docs. Its default uses `--dangerously-skip-permissions`; we do not.
- Vibe Kanban's sustaining plans changed after its company shut down: do not depend on a project in transition.
- oven-cli's dates and counts did not line up between docs.rs and GitHub: verify before relying on a version.
