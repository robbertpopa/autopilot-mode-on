<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# S8: Codex on a ChatGPT subscription, and switching to an API key

**Question:** can `codex exec` run on a ChatGPT subscription, how do we switch between subscription and API key without the cached login interfering, and what do OpenAI's terms say about driving Codex from a local tool? **Gates:** the Codex adapter (M8). **Spending:** a few cents on the API key route; the subscription route costs plan usage.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S8-codex-auth.md`."

## What a search of OpenAI's auth pages (2026-11-20) said, all unverified until this spike
- Two sign-in methods: ChatGPT sign-in (plan credits) and an API key (standard API pricing). The login is cached and reused.
- OpenAI's page recommends the API key for scripted and CI use.
- An API key is stored with `printenv OPENAI_API_KEY | codex login --with-api-key`; Codex does not accept keys as command-line arguments.
- A third-party guide says `CODEX_API_KEY` works only with `codex exec`, and that the two methods do not switch cleanly (clear the cached login first). Official pages did not mention `CODEX_API_KEY`.

## Method
- [ ] Read OpenAI's current auth page, the Codex CLI docs and the terms of use for ChatGPT plans and Codex. Note what they say about automation, scripts and third-party tools. Link each source.
- [ ] Find the status command (`codex login status` or similar) and its exit codes, to use as the preflight.
- [ ] With a ChatGPT sign-in: run `codex exec --json` on a tiny read-only task; record the event stream (thread, turn, item, usage) as fixtures.
- [ ] With an API key only: run the same task; record the stream and the cost fields.
- [ ] Find the environment variable or flag that sets Codex's configuration directory. Test two directories, one per auth mode, run alternately: does each keep its own login without interference?
- [ ] Check what `codex exec` does with approval requests (rejected, per the source) and with the sandbox modes; confirm network is off by default.
- [ ] How does a plan limit show up in `codex exec` output?
- [ ] Decide: Codex supports both modes with separate config directories, or Codex runs on the API key only, or Codex on subscription only for steps that run alone.

## Done when
`docs/DESIGN.md` section 7a is updated with verified facts and the chosen rule per auth mode; fixtures for `codex exec --json` are committed; the terms question has a written answer (allowed, forbidden or unclear) with sources.

## Result
(fill in)
