<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# S1: the headless contract of Claude Code

**Question:** what exactly does `claude -p` emit and accept? **Gates:** M1. **Budget:** within the shared $1, Haiku, `--max-budget-usd 0.05` per call, dedicated API key with a Console workspace limit.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S1-headless-contract.md`."

## Method

Start with a synthetic prompt, a structured response, a resumed response and a terminal failure. Record the CLI version. Sanitize credentials, local paths and session identifiers before writing versioned fixtures under `fixtures/claude/`. Use consistent fake session values where needed.
- [x] `claude --version`; `claude auth status` (JSON, exit code) for the signed-in state and for a state with `ANTHROPIC_API_KEY` set.
- [x] Minimal API call: `claude -p "reply with OK" --output-format stream-json --verbose --model haiku --max-turns 1 --max-budget-usd 0.05`. Note every message type and field, the `system` init fields, and the final `result` fields.
- [x] `--json-schema` with a tiny schema: does `structured_output` appear in the stream-json `result`, or only in `--output-format json`? What happens when the model cannot satisfy the schema (`error_max_structured_output_retries`)?
- [x] `--session-id <uuid>` then `--resume <uuid>` with all flags repeated: does it continue? What is not restored?
- [x] `--max-turns 1` on a task that needs a tool: confirm `error_max_turns` and its fields.
- [x] `--max-budget-usd 0.001`: confirm `error_max_budget_usd`; is the response that crosses the cap still returned?
- [ ] `--allowedTools` and `--disallowedTools`: a denied call shows up in `permission_denials`; in Manual mode with no tools allowed, what exactly is denied and how is it reported?
- [ ] Optional after the minimal live path: `--permission-prompt-tool`. Build a tiny stdio MCP server that logs the request it receives; find the response shape by trial and by the docs' error page (`errors.md#permission-prompt-tool-returned-an-invalid-result`). Record both a request and a valid allow and deny response.
- [x] `--settings` with a `PreToolUse` hook that denies `Write` outside the working directory and `Bash` containing `git push`: confirm the deny reaches the model and the stream.
- [x] A long prompt on stdin (more than 40,000 characters) instead of an argument.
- [x] Which credentials were used: with `ANTHROPIC_API_KEY` set and unset (compare the init message and the cost field).
- [x] `--include-partial-messages`: what extra events appear, and is it worth the volume?
- [ ] A subagent call: confirm `parent_tool_use_id` on its messages.

## Done when
M1's live path needs sanitized fixtures for the minimal invocation, structured output, resume and terminal failure. `docs/headless-contract.md` records the observed flags and output for the tested CLI version. Optional cases can be recorded when a milestone needs them.

## Local preflight

On 2026-11-20, native Windows Claude Code reported version `2.1.329`. Its help listed print mode, stream JSON, structured output, budget, tool selection, session and resume options, and strict MCP configuration. This was a local CLI check. The emitted wire format, credentials used, actual budget behavior and per-error fields still need the dedicated live spike.

Capture from a disposable working directory with synthetic content and the dedicated API workspace. Keep raw output out of the repository. Record the CLI version and launch flags without secrets. Retain the event types and field shapes needed to test the parser. Replace session identifiers consistently and replace local paths and arbitrary text with synthetic values. Scan the sanitized fixture before staging it.

## Local test runbook

Run from a disposable directory with a spend-limited key. The adapter in `crates/agent-claude` is untested against real output, so a failure here is useful evidence.

~~~powershell
cargo build
mkdir D:\scratch\s1; cd D:\scratch\s1
$exe = "D:/autopilot-mode-on/target/debug/autopilot.exe"
$ticket = "D:/autopilot-mode-on/fixtures/tickets/understand.json"
& $exe run-step --step understand --agent claude --ticket $ticket --data-dir .ap-sub
$env:AUTOPILOT_ANTHROPIC_API_KEY = "<dedicated key>"
& $exe run-step --step understand --agent claude --auth api --ticket $ticket --data-dir .ap-api
& $exe watch sample-export --data-dir .ap-api
~~~

For raw wire output, save the stdout of `claude -p --output-format stream-json --verbose --model haiku --max-turns 1 --max-budget-usd 0.05 "reply with OK"` to a file, then run `cargo xtask sanitize-claude RAW fixtures/claude/<name>.jsonl`. The command refuses to write if a key, address or local name survives. Read the result before staging it.

## Result

Run on 2026-11-21 against Claude Code 2.1.330 on native Windows. The observations are in [the headless contract](../headless-contract.md) and the sanitized transcripts are `fixtures/claude/live-2.1.330-*.jsonl`.

What the live run changed in the adapter:
- `schemars` adds a `$schema` URI that Claude Code rejects, so every real run failed until the key was dropped (PR 31).
- An unsatisfiable schema ends as `success` with exit 0 and no `structured_output`, not as `error_max_structured_output_retries`. The adapter already treated a missing structured output as a failure.
- A limit stop (`error_max_turns`, `error_max_budget_usd`) has `is_error: true` and exit 1, which the adapter already treated as a stop first.
- `authMethod` is `api_key` for API use, matching the preflight guess.

Not done: `--allowedTools` and `--disallowedTools` with the denial list (only `dontAsk` with `--tools Bash` was seen, and it still ran a read-only command), the optional `--permission-prompt-tool`, and the subagent check. The loopback-only S2 caveats are unchanged.
