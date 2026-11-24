<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Security: threat model and rules

## What we protect

- Credentials: the user's Claude Code and Codex sign-ins, any API key, the GitHub token.
- The user's repositories: no push to a protected branch, no change outside the ticket's worktree, no change to CI, lockfiles or scripts without a confirmation.
- The user's machine: no stray processes, no windows, no files outside the worktree and the data folder.
- The user's money: caps on turns, rounds, dollars and wall time.

## Who or what is untrusted

- Ticket text, comments, linked pages and code comments. They may contain instructions aimed at the agent (prompt injection). Research shows adaptive attacks beat most defences more than 85% of the time, so we assume an injection reaches the model and limit what it can do.
- Agent output. It may be wrong, or it may have been steered by the above.
- Fixtures and logs from other projects.

## Rules

1. **Cut one leg of the lethal trifecta** (private data, untrusted input, a way out). The agent has no secrets in its environment; the network is off where the platform allows; our code, not the agent, opens pull requests.
2. **Enforce containment at the runtime boundary.** Validate filesystem and tool egress restrictions for the agent and its descendants, and for the check runner. Deny hooks and per-step tool grants add protection. Recheck path and branch policy before publication.
3. **No blanket permission bypass.** Never `--dangerously-skip-permissions` or equivalents. Each step gets an explicit allowed-tools list.
4. **Per-step environment.** The child process gets a scrubbed environment: only what the step needs. A subscription step never sees `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` or `CLAUDE_CODE_USE_*` (a key outranks the login); an API step gets its key from a dedicated variable of ours.
5. **Keep credentials out of storage and logs.** Use provider-managed sign-in by default. API mode injects a dedicated key only into a verified provider-only environment. The host-side tracker handles GitHub sign-in separately.
6. **Forced confirmations** (the step asks even if its checkbox is off): a protected path or `.github/**`, lockfile or script change, a risky tier, a security label, an exceeded budget, a suspicious diff (new network calls, new dependencies, changed signing).
7. **Ticket text is wrapped** in a clearly marked data block in every prompt, with the instruction to treat it as data. This is a speed bump, not a defence.
8. **Logs and summaries** are secret-redacted before they are stored or shown. Prompts shown in the step detail have secrets removed.
9. **Inbox files** (requests and answers) live in an owner-only folder and are validated like any input.
10. **Spike budget** for live tests: $1 in total, dedicated key with a Console spend limit.

## Containment levels (S6 decides what is real on Windows)

| Level | What it is | Status |
|---|---|---|
| 0 | Worktree, tool grants, deny hooks and scrubbed environment | Fixture work can proceed. |
| 1 | Level 0 plus a validated filesystem, process and egress boundary for agents and checks | Required for real Build, Check and publishing. S6 records each supported launcher, platform and auth mode. |
| 2 | A separately validated WSL or container runtime | Alternative if a native runtime cannot meet level 1. |

| Platform | Launcher | Auth mode | Status (2026-11-23) |
|---|---|---|---|
| Windows native | none (Claude's sandbox is unavailable) | subscription or API | Level 0. Real Build refused. |
| Windows native | Codex MXC `workspace-only` | none (network off) | Probe only; the provider is unreachable, so it cannot host Claude. |
| Windows host, WSL2 Ubuntu 24.04 | `srt` 0.0.83 (bubblewrap, seccomp, domain proxy, TLS termination) | API key | Supported for Build, Check, review and the pull request step after the escape probe passes on this machine. The key is masked inside. Subscription mode refused. |
| Linux host | `srt`, run directly | API key | Supported on the same terms; the probe drops the Windows-only rows. Probed inside the distro as a Linux machine. |
| macOS host (Apple silicon, macOS 27) | `srt` 0.0.83 run directly (Seatbelt, domain proxy, TLS termination) | API key | Supported for Build and Check after its own probe passes on the machine: 21 rows per role (writes, `.git`, a write through a system service, reads of decoys in the home folder, `~/.ssh`, `~/.claude`, loopback, a Unix socket, other hosts, the provider, the hook, the masked key), each refused row with a control. Set up with `tools/macos/setup.sh`. The data folder must not be under `/tmp` (readable by design); the probe fails if it is. Not covered: Intel Macs, other macOS versions, the window's Build button (the CLI path ran), a descendant that calls `setsid` (no subreaper on macOS). |

`srt` is pinned: discovery refuses a version that is not in `VALIDATED_SRT` (`crates/process/src/wsl.rs`), and the probe's output and failure message carry the versions of `srt`, bubblewrap and Claude Code it ran with. **To update `srt`:** install the new version on a test machine, run the real probe (`cargo test -p autopilot-process -- --ignored the_real_boundary_passes_the_probe` on the WSL host, `..._on_a_linux_host` on Linux; CI runs the second in `sandbox-probe.yml`), and only if every row passes add the version to `VALIDATED_SRT`, change the pin in `tools/wsl/setup-user.sh` and `.github/workflows/sandbox-probe.yml`, and update the table above.

Set up a distro with `tools/wsl/setup-root.sh`, `setup-user.sh` and `install-hook.sh`. Details, settings and limits: [S6 Linux sandbox in WSL2](spikes/S6-containment.md#linux-sandbox-in-wsl2-srt). The launcher must scrub the environment itself: `srt` passes it through.

The [S6 Windows preflight](spikes/S6-containment.md#windows-mxc-preflight) exercised one MXC profile against synthetic files, a local TCP listener and a child process.

## Reporting

A vulnerability in this tool is reported privately through the repository's security advisory, never in a public issue.
