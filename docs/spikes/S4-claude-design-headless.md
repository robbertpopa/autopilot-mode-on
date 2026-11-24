<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# S4: can a headless session publish a Claude Design canvas?

**Question:** can `claude -p` use the Artifact tool and the Design artifact type, under which login, and how does feedback reach it? **Gates:** the Claude Design path of M6 only. **Needs:** an approved claude.ai account and a synthetic live trial.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S4-claude-design-headless.md`."

## Documentation checked on 2026-11-21

- Claude Code documents the `/design` command from version 2.1.265. It creates artboards in a Claude Design artifact. A qualifying session is backed by a claude.ai sign-in and needs artifact and template availability for the account. API-key, gateway and cloud-provider credentials cannot publish these artifacts. The documented interactive capability leaves availability from `claude -p` to verify. [Claude Code artifacts](https://code.claude.com/docs/en/artifacts)
- A new artifact starts private. Publishing uploads the rendered source to Anthropic and gives it a URL. Claude Code documents revisions to the same artifact URL, and sources describe organization sharing controls and comment permissions. Claude Code can read internal comments in supported sessions. The headless path still needs to demonstrate URL reuse and comment access. [Claude Code artifacts](https://code.claude.com/docs/en/artifacts) · [Share artifacts](https://support.claude.com/en/articles/9547008-share-artifacts)
- Anthropic is moving standalone Claude Design into Claude's Artifacts surface. Use the current Claude Code Design template for the trial. [Move from standalone Claude Design](https://support.claude.com/en/articles/17440474-move-from-standalone-claude-design-to-claude)
- The CLI can suppress browser auto-open with `CLAUDE_CODE_ARTIFACT_AUTO_OPEN=0`. `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` disables artifacts. Keep these controls distinct in a remote trial. The CLI docs list cases where Claude asks to publish, including when it still needs file access. An Artifact allow rule cannot override that file approval. [Artifacts permissions](https://code.claude.com/docs/en/artifacts#when-claude-code-asks-before-publishing) · [Environment variables](https://code.claude.com/docs/en/env-vars)

## Offline CLI preflight

The Linux x64 2.1.330 CLI help was checked in a network-isolated container. It lists `--print`, `--output-format stream-json`, `--resume`, permission modes and tool rules. It exposes no artifact-specific flag in the help text. The preflight was limited to CLI help output.

## Method

- [x] Verify the supported interactive flow and the documented auth, hosting, sharing and browser settings.
- [ ] Use an approved account with the Design template enabled. Create a private synthetic design in a clean scratch directory as an interactive control.
- [ ] With the same authorized login, run a headless `--print --verbose --output-format stream-json` trial on a synthetic brief. Preserve session state for the resume test. Record the exact version, settings, publish decision and sanitized shape of the resulting events or failure. Confirm the final URL opens to a Design canvas and record the reported artifact type.
- [ ] Resume with a revision request and compare the artifact identity and URL. Check audience and permissions without publishing a public link. If the account supports organization comments, confirm whether the headless session can read a synthetic comment and keep the project's approval in the inbox.
- [ ] Verify what content is uploaded and the narrow permission grant, if any, needed to publish in print mode.
- [ ] Follow-ups once publishing works (tracked as A14 in [stream 2](../work-streams/02-agents-and-adapters.md)): revise the same page for the same ticket; read the page's comments; keep one page per project that holds every ticket's mockups, and check that a second ticket adds a section to the same URL.

## Result

**Headless trial on 2026-11-24 (Claude Code 2.1.330, claude.ai subscription login, about $0.22):** `claude -p` created a private artifact of the Design type and returned its URL, so the login is eligible. Uploading the canvas files was refused with an approval that print mode cannot answer, so the artifact stayed empty. Revision of the same URL and reading comments were not tried.

**Next trial:** run `claude -p` with `--permission-prompt-tool` pointing at a tool of ours that allows only uploads from the ticket's `designs/<ticket>/` folder and denies everything else, then check the canvas has content, a revision keeps the URL, and the owner approval stays in the inbox. One Design page per project, holding every ticket's mockups, is the goal; it stays an explicit per-project opt-in because the content is uploaded to Anthropic. Ask before the trial: it sends a synthetic design off this machine.

The local HTML mockup remains the first M6 path. The remote candidate requires an eligible claude.ai login and explicit project opt-in. S4 is open until the subscription headless trial verifies publication, revision and audience.
