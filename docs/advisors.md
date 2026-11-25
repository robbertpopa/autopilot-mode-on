# Advisors, model roles and watchers

Goal: spend tokens where they buy quality, and let the tool find its own waste.

## Built

- **Model roles.** A step with no model in the profile gets one that fits its job: opus for Plan, haiku for Clarify and the pull request, sonnet for the rest. A model in the profile still wins, and an orchestrator or ticket override wins over that.
- **Advisor.** `[advisor]` in the profile: `model` (default `opus`), `effort`, `max_calls` per ticket (default 3, 0 is off). `advise_ticket` asks the advisor one question, read-only (Read, Glob, Grep, nothing that changes a file), keeps the answer as an `Advised` event, answers the same question from the history for free, and refuses past the cap. The ticket page has an "Ask the advisor" box with the Claude agent.
- **Watcher.** The pipeline review reads finished runs (peak context, cost against the median) and writes findings to Health. In "Notify and tune" mode it lowers the effort of an early step of a custom pipeline, and a guard reverts it after three runs if more of them fail. It is off until switched on under Settings, Pipeline review, because the tests state that off is the default. It costs no tokens.

## Cost of Plan (measured 2026-11-25)

Live runs through `autopilot run-step --auth api` with the Claude Code CLI 2.1.332, `--max-budget-usd 0.05`, one tiny ticket (a Rust `reverse(&str)` function; two clarify questions answered with the recommended option). A Plan call has no tools and runs from a neutral folder, so this is the cost of the answer alone, not of reading a repository. Cost is the `total_cost_usd` the CLI reports, as the ticket history records it.

| Step | Model | Cost | Result |
|---|---|---|---|
| Understand | haiku | $0.0010, $0.0008 (two runs) | passed |
| Plan | haiku | $0.0034 | 6 criteria, 0 errors, 0 warnings, all covered |
| Plan | sonnet | $0.0140 | 5 criteria, 0 errors, 0 warnings, all covered |
| Plan | opus | not measured | Opus is outside this round's spend rule. The one recorded Plan in A13 stopped on `error_max_turns` after $1.32 with no result, on a real repository with read tools. |

What this shows and does not show:

- On a tiny ticket and no repository, Sonnet costs about four times Haiku for a Plan, and both are cents or less. The large Plan costs seen before come from Opus and from many turns on a real repository, which this run did not reproduce.
- Both plans passed the engine's own analysis. Nothing here says which plan is better; Health has no quality score yet (item 3 under "Not built yet"), so a cheaper Plan model is not proven good enough.
- Default stays as it is: Plan uses `opus` (`crates/profile/src/lib.rs`), because no measurement here supports lowering it. To try Sonnet for Plan, set `model = "sonnet"` for the step in the profile and compare the finished tickets' cost and review findings.
- The per-call cap of $0.05 held for every call. A Plan on a real repository with Opus needs a higher cap; a Plan that stops at the turn limit or the cap now says how much was spent ("stopped at the turn limit after 20 turns, $1.32 spent, with no result") and shows a running `progress: turn N, about $X so far` line.
- Running a Plan on a ticket whose earlier plan still waits for approval spent money, recorded no cost, and left the ticket history invalid ("event 29 was rejected: Plan is still active"). That is an engine behaviour, reported to the owners of `engine`.

## Not built yet

1. **The agent asks the advisor itself.** Today the owner asks. The build agent needs an `advise` tool through `autopilot-mcp`, with the same cap, so it can ask when it is unsure. An automatic ask before a plan with open assumptions, and after two failed build rounds, comes after that.
2. **Prompt and output watcher.** A haiku or sonnet call over a finished ticket's step prompts and results that proposes a better prompt. It is a proposal the owner applies; it needs the corpus from M7 to say whether a prompt is better.
3. **Quality score.** The model-graded reviewer (M9). Until it exists Health cannot say whether a cheaper model lost quality, so tuning only lowers effort.
4. **Cost per role.** Health lists cost per step; per role (advisor, orchestrator, watcher) needs the advisor's cost recorded as usage.
5. **Conversation advice** (new chat or compact). This is about the owner's own coding sessions, not tickets, so it lives outside the app: a small tool that reads a session transcript and reports its size.
