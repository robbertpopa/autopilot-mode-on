<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# S4: can a headless session publish a Claude Design canvas?

**Question:** can `claude -p` use the Artifact tool and the Design artifact type, under which login, and how does feedback reach it? **Gates:** the Claude Design path of M6 only. **Needs:** the user's hands on the subscription (a few minutes).
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S4-claude-design-headless.md`."

The official docs do not mention the Artifact tool or Claude Design for headless or SDK sessions. This spike settles it by trying.

## Method
- [ ] Signed in with the subscription, run `claude -p "make a one-screen design canvas of a login form with Claude Design and give me its link"` with the tools the Artifact tool needs allowed. Does it publish? Does the result contain the link?
- [ ] Repeat with an API key only: does the tool exist and work, or is a claude.ai login required?
- [ ] Ask the same headless session (resumed) to change the design: does the same URL update?
- [ ] Can a headless session read comments on the artifact, so feedback can come from the canvas instead of our inbox?
- [ ] What permission prompts appear in headless mode for publishing, and can they be pre-approved safely (allowed-tools entry) without opening other doors?
- [ ] Where does the design live, who can open it (private by default?), and what project information leaves the machine? Write this down for the privacy switch in M6.

## Done when
A yes or no with the exact flags and login that work. If no or too fragile, `docs/milestones/M6-design-step.md` is updated to make the HTML-mockup fallback the default path.

## Result
(fill in)
