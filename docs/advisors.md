# Advisors, model roles and watchers

Goal: spend tokens where they buy quality, and let the tool find its own waste.

## Built

- **Model roles.** A step with no model in the profile gets one that fits its job: opus for Plan, haiku for Clarify and the pull request, sonnet for the rest. A model in the profile still wins, and an orchestrator or ticket override wins over that.
- **Advisor.** `[advisor]` in the profile: `model` (default `opus`), `effort`, `max_calls` per ticket (default 3, 0 is off). `advise_ticket` asks the advisor one question, read-only (Read, Glob, Grep, nothing that changes a file), keeps the answer as an `Advised` event, answers the same question from the history for free, and refuses past the cap. The ticket page has an "Ask the advisor" box with the Claude agent.
- **Watcher.** The pipeline review reads finished runs (peak context, cost against the median) and writes findings to Health. In "Notify and tune" mode it lowers the effort of an early step of a custom pipeline, and a guard reverts it after three runs if more of them fail. It is off until switched on under Settings, Pipeline review, because the tests state that off is the default. It costs no tokens.

## Not built yet

1. **The agent asks the advisor itself.** Today the owner asks. The build agent needs an `advise` tool through `autopilot-mcp`, with the same cap, so it can ask when it is unsure. An automatic ask before a plan with open assumptions, and after two failed build rounds, comes after that.
2. **Prompt and output watcher.** A haiku or sonnet call over a finished ticket's step prompts and results that proposes a better prompt. It is a proposal the owner applies; it needs the corpus from M7 to say whether a prompt is better.
3. **Quality score.** The model-graded reviewer (M9). Until it exists Health cannot say whether a cheaper model lost quality, so tuning only lowers effort.
4. **Cost per role.** Health lists cost per step; per role (advisor, orchestrator, watcher) needs the advisor's cost recorded as usage.
5. **Conversation advice** (new chat or compact). This is about the owner's own coding sessions, not tickets, so it lives outside the app: a small tool that reads a session transcript and reports its size.
