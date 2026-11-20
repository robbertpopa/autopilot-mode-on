<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# S2: how a plan or rate limit looks, and how to detect it

**Question:** when Claude Code hits a plan or rate limit in headless mode, what appears in the output, and what rule detects it? **Gates:** M1 (detection), the `fallback = api` setting. **Depends on:** S1 fixtures.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S2-plan-limits.md`."

## Documentation checked on 2026-11-21

- The API can return HTTP 429 with `rate_limit_error` for rate limits and certain spend caps. `retry-after`, when present, is a waiting period in seconds. A tier spend-cap 429 omits it. HTTP 529 with `overloaded_error` indicates service capacity. A streaming HTTP 200 can still contain an error. [API errors](https://platform.claude.com/docs/en/api/errors) · [Rate limits](https://platform.claude.com/docs/en/api/rate-limits)
- Claude Code documents `system/api_retry` with `attempt`, `max_retries`, `retry_delay_ms`, `error_status` and an `error` category. `CLAUDE_CODE_MAX_RETRIES` defaults to 10, with some overload cases stopping earlier. The watchdog can extend waiting for 429 and 529 capacity errors, and recent versions support a time ceiling. [Headless usage](https://code.claude.com/docs/en/headless#handle-api-retries) · [Environment variables](https://code.claude.com/docs/en/env-vars)
- Paid subscriptions can have session and weekly usage limits. Keep the provider auth mode as input to classification. A configured API key can take priority over a subscription and bill API usage. [Usage limits](https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work) · [API key precedence](https://support.claude.com/en/articles/12304248-manage-api-key-environment-variables-in-claude-code)

## Loopback probe

Claude Code 2.1.330 for Linux x64 ran inside a container with `--network none`. Each case used a fresh home and config directory, a synthetic key and `ANTHROPIC_BASE_URL` pointing to a local HTTP server on 127.0.0.1. The CLI ran in bare mode with no tools or session persistence. The server answered only `POST /v1/messages` and supplied synthetic JSON errors. The recovery case returned a complete [Messages SSE stream](https://platform.claude.com/docs/en/build-with-claude/streaming) on the second request. The CLI's external timeout was 20 seconds and `CLAUDE_CODE_MAX_RETRIES` was 1 in every case. Watchdog cases set its wait ceiling to 1800 ms.

| Local response | Retry events | Final assistant error | `result.is_error` | Exit |
|---|---:|---|---|---:|
| 429 with `retry-after: 1`, then HTTP 200 | 1 | Absent | false | 0 |
| Persistent 429 with `retry-after: 1` | 1 | `rate_limit` | true | 1 |
| Persistent 529 without a retry header | 1 | `server_error` | true | 1 |
| Persistent 429 with `retry-after: 1`, watchdog enabled | 2 | `rate_limit` | true | 1 |
| Persistent 529 with `retry-after: 1`, watchdog enabled | 2 | `server_error` | true | 1 |

All five results carried `subtype: "success"` and stderr was empty. Watchdog cases reached attempt 2 even with `max_retries: 1` and exited in under 2.5 seconds in this run. The [projected fixture](../../fixtures/claude/s2-loopback-2.1.330.jsonl) records the observed fields and timings. Its envelopes are observations from an artificial server. S1 will establish the complete live CLI contract.

## Proposed detector

1. Persist each retry as activity. `retry_delay_ms` is the delay to the next attempt. A terminal result with validated output and no errors can succeed after a retry.
2. Interpret terminal `is_error` and process completion together. A `success` subtype with `is_error: true` is a failed result. A timeout, cancellation or invalid stream remains its own outcome.
3. An explicit API rate-limit failure is an API limit signal with unknown scope. The generic 429 cannot distinguish transient throttling from a spend cap. For a terminal server failure, an immediately preceding 529 retry provides an overload hint. Keep the hint distinct from a confirmed terminal status when the terminal response lacks a status code.
4. Treat suspected subscription limits as a reason to pause for input. Confirm plan scope using versioned subscription evidence before classifying a session or weekly limit. A matching phrase in model text or stderr alone is insufficient. API fallback remains off by default and needs verified subscription evidence plus explicit opt-in.

## Method

- [x] Read the current API errors, CLI retry and subscription documentation.
- [x] Reproduce 429, 529 and recovery against a local server. Compare the watchdog off and on with a low retry count and bounded wait.
- [ ] Capture and sanitize a real subscription-limit transcript when one is available.
- [ ] Add the pure detector and replay tests after the S1 live schemas and terminal outcomes are captured. Mark any unobserved subscription cases in the contract.

## Result

The Linux loopback evidence supports the proposed retry and terminal-error rules. Subscription limits, API spend-cap classification and Windows live CLI behavior still need evidence. S2 remains open.
