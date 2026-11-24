<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Decisions

Newest last. A decision changes only by adding a new line that supersedes it.

| Date | Decision | Notes |
|---|---|---|
| 2026-11-20 | Local program in Rust; egui window; CLI and engine first, the window is a thin client. | Forge-style egui, but unrelated to Contest Keeper's Tauri apps. |
| 2026-11-20 | Licence Apache-2.0. | Dependencies permissive only. |
| 2026-11-20 | Workers are existing agents (Claude Code first, Codex next). We do not write a coding agent. | mini-SWE-agent shows a thin loop is competitive. |
| 2026-11-20 | Build and Check come from craft-anything's loop. No Study step. | Our Understand reads a ticket, not a target app. |
| 2026-11-20 | No gates. A step asks when it needs something; every step has "Ask me after this step". The AI decides whether to clarify. | Rules (protected path, risky tier, budget) force a question regardless. |
| 2026-11-20 | A Design step using Claude Design, approval required whenever it runs. | Headless use unverified (S4); HTML mockups are the fallback. |
| 2026-11-20 | Summaries are code-made and free; a model-written explanation only on request. | Saves tokens. |
| 2026-11-20 | Name `autopilot-mode-on`. | Free on crates.io when checked. Plain `autopilot` is taken. |
| 2026-11-20 | Default auth is subscription, for Claude Code and Codex; API key per step. | Codex terms and switching: S8. |
| 2026-11-20 | Claude Code is the main agent and the default (`agent = "claude"` for every step); Codex is the second agent, optional, added in M8. | The pipeline must be complete and useful without Codex. The Codex adapter is built against the same normalised events and can start in its own tab after M1 and S8. |
| 2026-11-20 | Implementation continues in separate tabs, one file per milestone and spike. | Plan section 8a. |
| 2026-11-20 | Add codebase review after the core ticket workflow. Report-only is the default, each finding has evidence, and approved fixes use normal tickets. | See [M9](milestones/M9-codebase-review.md) and [S9](spikes/S9-review-quality.md). |
| 2026-11-21 | Use a fixed egui canvas for the pipeline graph. | The probe kept card positions fixed, rendered all six labels and passed the interaction checks on Windows and Linux. [S7](spikes/S7-graph-widget.md#result) |

| 2026-11-23 | Real Build and Check run only inside a validated Linux boundary: WSL2 on Windows or a Linux host, `srt` 0.0.83, API-key mode. Subscription mode and git inside the sandbox are refused. | Reasons and evidence: [S6](spikes/S6-containment.md#result). It narrows the 2026-11-20 default for the steps that run code; read-only steps keep subscription as the default. |
| 2026-11-23 | Review a change with up to four fresh reviewers and act only on findings above a fixed confidence of 80. | PR 140. Not yet run against the real model at scale. |
| 2026-11-24 | `autopilot pr watch` names the one next action for a published pull request and changes nothing; no background polling. | Order: conflict, review feedback, failing CI, unsynced, ok. Only comments from owners, members and collaborators reach a prompt. |
| 2026-11-24 | macOS hosts run the same `srt` boundary with a probe written for Seatbelt (21 rows). A hand-written Seatbelt profile is rejected. | [S6](spikes/S6-containment.md#macos-sandbox-with-srt-2026-11-24). Validated on one Apple silicon Mac. |

Open: plan-limit behaviour (S2), Codex auth switching and terms (S8), review quality and cost (S9), and the owner decisions in [the work-streams overview](work-streams/00-overview.md): the real GitHub test, the computer-use scope, further agents, and subscription mode in the sandbox.
