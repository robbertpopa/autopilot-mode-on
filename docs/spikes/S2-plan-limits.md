# S2: how a plan or rate limit looks, and how to detect it

**Question:** when Claude Code hits a plan or rate limit in headless mode, what appears in the output, and what rule detects it? **Gates:** M1 (detection), the `fallback = api` setting. **Depends on:** S1 fixtures.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S2-plan-limits.md`."

## Known from the docs
- Messages: "You've hit your session limit", "You've hit your weekly limit", "Request rejected (429)", "API Error: Repeated 529 Overloaded errors".
- Retries are 10 by default with backoff and appear as `system/api_retry` (attempt, max_retries, delay, error `rate_limit` or `overloaded`). `CLAUDE_CODE_RETRY_WATCHDOG=1` retries without end in unattended runs.
- No exit code is documented for limits.

## Method
- [ ] Read the docs' errors page fully and list every limit and overload message.
- [ ] Reproduce without spending a real limit: run a local HTTP server that answers 429 and 529 with the headers a real limit sends, and point Claude Code at it with `ANTHROPIC_BASE_URL` and a throwaway key. Record the `system/api_retry` events, the final `result`, the exit code and the time it takes.
- [ ] Try the same with the retry watchdog on and off, and with a lower retry count if a setting exists.
- [ ] Subscription behaviour cannot be reproduced this way: if you happen to hit a real subscription limit, save that transcript to `fixtures/` immediately and mark it as the only real sample.
- [ ] Write the detector as a pure function over events and result text: `RateLimited{kind: session|weekly|rate|overloaded, retry_after?}`.

## Done when
The detector rule is written with fixtures for each case; unverified cases (real subscription limits) are marked as such in `docs/headless-contract.md`; the `fallback = api` setting is documented as off by default until a real subscription sample exists.

## Result
(fill in)
