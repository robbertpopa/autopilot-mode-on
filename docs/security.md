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

## Reporting

A vulnerability in this tool is reported privately through the repository's security advisory, never in a public issue.
