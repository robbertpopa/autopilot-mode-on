<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# S1: the headless contract of Claude Code

**Question:** what exactly does `claude -p` emit and accept? **Gates:** M1. **Budget:** within the shared $1, Haiku, `--max-budget-usd 0.05` per call, dedicated API key with a Console workspace limit.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S1-headless-contract.md`."

## Method

Start with a synthetic prompt, a structured response, a resumed response and a terminal failure. Record the CLI version. Sanitize credentials, local paths and session identifiers before writing versioned fixtures under `fixtures/claude/`. Use consistent fake session values where needed.
- [ ] `claude --version`; `claude auth status` (JSON, exit code) for the signed-in state and for a state with `ANTHROPIC_API_KEY` set.
- [ ] Minimal API call: `claude -p "reply with OK" --output-format stream-json --verbose --model haiku --max-turns 1 --max-budget-usd 0.05`. Note every message type and field, the `system` init fields, and the final `result` fields.
- [ ] `--json-schema` with a tiny schema: does `structured_output` appear in the stream-json `result`, or only in `--output-format json`? What happens when the model cannot satisfy the schema (`error_max_structured_output_retries`)?
- [ ] `--session-id <uuid>` then `--resume <uuid>` with all flags repeated: does it continue? What is not restored?
- [ ] `--max-turns 1` on a task that needs a tool: confirm `error_max_turns` and its fields.
- [ ] `--max-budget-usd 0.001`: confirm `error_max_budget_usd`; is the response that crosses the cap still returned?
- [ ] `--allowedTools` and `--disallowedTools`: a denied call shows up in `permission_denials`; in Manual mode with no tools allowed, what exactly is denied and how is it reported?
- [ ] Optional after the minimal live path: `--permission-prompt-tool`. Build a tiny stdio MCP server that logs the request it receives; find the response shape by trial and by the docs' error page (`errors.md#permission-prompt-tool-returned-an-invalid-result`). Record both a request and a valid allow and deny response.
- [ ] `--settings` with a `PreToolUse` hook that denies `Write` outside the working directory and `Bash` containing `git push`: confirm the deny reaches the model and the stream.
- [ ] A long prompt on stdin (more than 40,000 characters) instead of an argument.
- [ ] Which credentials were used: with `ANTHROPIC_API_KEY` set and unset (compare the init message and the cost field).
- [ ] `--include-partial-messages`: what extra events appear, and is it worth the volume?
- [ ] A subagent call: confirm `parent_tool_use_id` on its messages.

## Done when
M1's live path needs sanitized fixtures for the minimal invocation, structured output, resume and terminal failure. `docs/headless-contract.md` records the observed flags and output for the tested CLI version. Optional cases can be recorded when a milestone needs them.

## Local preflight

On 2026-11-20, native Windows Claude Code reported version `2.1.329`. Its help listed print mode, stream JSON, structured output, budget, tool selection, session and resume options, and strict MCP configuration. This was a local CLI check. The emitted wire format, credentials used, actual budget behavior and per-error fields still need the dedicated live spike.

Capture from a disposable working directory with synthetic content and the dedicated API workspace. Keep raw output out of the repository. Record the CLI version and launch flags without secrets. Retain the event types and field shapes needed to test the parser. Replace session identifiers consistently and replace local paths and arbitrary text with synthetic values. Scan the sanitized fixture before staging it.

## Result

Live fixtures pending.
