<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Claude Code headless contract

What `claude -p --output-format stream-json --verbose` emitted and accepted when observed. Version tested: **2.1.330**, native Windows, 2026-11-21. Everything here was seen in a real run unless marked **not observed**. Sanitized transcripts are in `fixtures/claude/live-2.1.330-*.jsonl`. Each fixture was made by `cargo xtask sanitize-claude`, which maps identifiers, drops prose and empties machine inventory fields.

Spend: four API calls on the dedicated key, about $0.0025 in total, and about a dozen subscription calls on a Pro plan. Haiku (`claude-haiku-5-5`) throughout.

## Frames

One JSON object per line. Types seen: `system`, `assistant`, `user`, `rate_limit_event`, `result`, and with `--include-partial-messages` also `stream_event`.

| Frame | Fields the adapter uses | Notes |
|---|---|---|
| `system` / `init` | `model`, `tools`, `apiKeySource`, `claude_code_version`, `session_id`, `permissionMode` | Also lists plugins, skills, slash commands and MCP servers of the machine. `apiKeySource` is `none` for a subscription and `ANTHROPIC_API_KEY` when that variable is set. |
| `system` / `thinking_tokens`, `hook_started`, `hook_response` | none | Ignored. Hook frames appear only with `--include-hook-events` and a configured hook. |
| `system` / `api_retry` | `attempt`, `error_status` | From the S2 loopback runs. Not seen against the real API. |
| `assistant` | `message.content[]` blocks (`text`, `thinking`, `tool_use`) | An optional top-level `error` string appears on failed turns (S2). |
| `user` | none | Carries `tool_result` blocks, including hook errors. |
| `rate_limit_event` | `rate_limit_info.status` | Subscription runs only. The API-key runs had none. |
| `result` | `subtype`, `is_error`, `structured_output`, `total_cost_usd`, `usage`, `terminal_reason`, `errors` | One per run. |

The Bash tool's input carries a short `description` of why the command runs; the adapter keeps it, bounded and redacted, in the activity history (PR 146). The same version, 2.1.330, also ran on macOS under the sandbox (S6) without a change in the frames.

## Result subtypes seen

| Case | `subtype` | `is_error` | Exit | Notes |
|---|---|---|---|---|
| Normal run | `success` | false | 0 | `terminal_reason: completed`. |
| `--max-turns 1` with a tool call | `error_max_turns` | true | 1 | `terminal_reason: max_turns`, `errors: ["Reached maximum number of turns (1)"]`, cost still reported. |
| `--max-budget-usd 0.0001` | `error_max_budget_usd` | true | 1 | `terminal_reason: budget_exhausted`. The response that crossed the cap was still emitted and billed ($0.000433 against a $0.0001 cap). |
| Schema the model cannot satisfy | `success` | false | 0 | **No `structured_output` key.** The model explained in `result` text after the tool rejected its output twice. `error_max_structured_output_retries` was **not observed**. |
| S2 loopback 429 and 529 | `success` | true | 1 | See the S2 note. The subtype alone never decides. |

## Structured output

`--json-schema` makes Claude Code add a `StructuredOutput` tool. With `--tools ""` and `--strict-mcp-config` it is the only tool listed. The answer arrives in the `result` frame as `structured_output`, and the run takes **two turns** (`num_turns: 2`, `stop_reason: tool_use`), so `--max-turns 1` cannot work with a schema. The adapter uses 4.

The schema must not carry a `$schema` URI. `schemars` writes `https://json-schema.org/draft/2020-12/schema` and Claude Code exits 1 with `--json-schema is not a valid JSON Schema: no schema with key or ref ...`.

## Cost and usage

`total_cost_usd` is present for API and subscription runs. On a subscription `modelUsage[model].costBasis` is `list`, meaning list price, not a bill. `usage` uses the Messages API names, with `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens` for the cache-write split. The price table in `crates/agent-claude/src/pricing.rs` reproduced the reported cost exactly in both runs checked (1,383 and 1,150 micro-dollars).

## Credentials

- `claude auth status --json`: `authMethod` is `claude.ai` for a subscription and `api_key` when `ANTHROPIC_API_KEY` is set. It also reports `apiKeySource`. Exit code 0 in both cases.
- With `ANTHROPIC_API_KEY` set, stderr prints a warning that claude.ai connectors are disabled. Without it the account's claude.ai connectors can appear as tools.
- The adapter's clean environment (an allowlist, no `CLAUDE_CODE_*` session variables) ran correctly with the sign-in found through `CLAUDE_CONFIG_DIR`.

## Flags accepted

`-p`, `--output-format stream-json`, `--verbose`, `--model haiku`, `--max-turns`, `--max-budget-usd`, `--tools ""` (no built-in tools), `--strict-mcp-config`, `--permission-mode dontAsk`, `--json-schema`, `--append-system-prompt`, `--no-session-persistence`, `--disable-slash-commands`, `--session-id <uuid>`, `--resume <uuid>`, `--settings <json>`, `--include-hook-events`, `--include-partial-messages`. The prompt can be sent on stdin; a 63,000-character prompt worked.

## Sessions

`--session-id <uuid>` was honoured: `init.session_id` and `result.session_id` equal the id given. `--resume <uuid>` with the same flags continued the conversation (it recalled a word from the first call) and kept the same session id. Both used persistence, so `--no-session-persistence` must stay off for any step that resumes.

## Permissions and hooks

- `--permission-mode dontAsk` with `--tools Bash` and no allow list **still ran `echo hi`**. Treat read-only shell commands as allowed by default, and never rely on `dontAsk` as a deny list.
- A `PreToolUse` hook passed with `--settings` denied `git push origin main`. The model received a tool error ("PreToolUse:Bash hook error: ..."), the stream showed `hook_started` and `hook_response` frames, and the result's `permission_denials` listed the command. The same hook let `echo hello` through. The hook ran as `pwsh -NoProfile -File <script>` and replied with `hookSpecificOutput.permissionDecision: "deny"`.

## Permission prompt tool

Observed 2026-11-25 with Claude Code **2.1.332**, Haiku, API key, `--tools Read --strict-mcp-config --mcp-config <file> --permission-prompt-tool mcp__prompt__approve`, two calls, $0.0014 in total. Fixtures: `live-2.1.332-permission-prompt-{mcp,deny,allow}.jsonl`.

- The tool is called only when the built-in rules do not decide: a `Read` inside the working folder ran without a prompt, a `Read` outside it asked.
- Request: an MCP `tools/call` of the named tool with `arguments` `{tool_name, input, tool_use_id}` (and `_meta.claudecode/toolUseId`). The server is first probed with `server/discover` and then initialised with protocol `2025-11-25`; a server that answers `{}` to the probe and echoes the protocol version works.
- Reply: one text content block whose text is JSON. `{"behavior":"deny","message":"..."}`: the model got a tool error with that message, the run went on, and `result.permission_denials` listed the call. `{"behavior":"allow","updatedInput":{...}}` (the input as received): the tool ran and `permission_denials` was empty.
- Not observed: a reply that is not valid JSON, `updatedPermissions`, a server that never answers, and any tool other than `Read`.

## Streaming volume

`--include-partial-messages` added 17 `stream_event` deltas for a three-sentence answer and roughly doubled the stream (16 KB against 8 KB). Not worth it for the pipeline.

## Not observed

subagent messages and `parent_tool_use_id`, `error_max_structured_output_retries`, `api_retry` frames against the real API, a real subscription rate limit, and the PowerShell-vs-bash tool behaviour. S2 still owns the limit evidence.
