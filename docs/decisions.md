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

Open: plan-limit behaviour (S2), Codex auth switching and terms (S8), and review quality and cost (S9).
