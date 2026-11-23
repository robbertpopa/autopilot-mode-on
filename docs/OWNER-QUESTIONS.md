# Questions for the owner (session note, untracked)

Parked decisions from the window redesign work. Each has a recommendation and what is blocked.

## 1. Reviewer default mode (2026-11-22, R11)

Question: should the pipeline review start Off, Notify only or Notify and tune?
Recommendation: Off for everyone, as shipped. Notify only for custom and agent-built pipelines once a few real runs exist, so the thresholds have been looked at against real numbers.
Blocked: nothing; Off is the shipped default. Notify and tune is greyed until the tuning overlay (R12) exists.

## 2. May a live Claude draft or review spend money? (R9, R11)

Question: "Build with agent" and the model-graded quality reviewer both need a live model call.
Recommendation: not yet. Both stay greyed for the Claude choice. Allow one capped Haiku call (at most $0.05) as a live test behind the `live` feature, then decide.
Blocked: live pipeline drafts and the quality reviewer. The practice agent drafts in the meantime.

## 3. JetBrains Mono for code text (UI-REDESIGN U1)

Question: bundle JetBrains Mono (adds a font file and a licence line) or keep the system monospace?
Recommendation: keep the system monospace for now; it adds nothing to install.
Blocked: nothing.

## 4. The develop merge after the squash merges

Question: PR 88 and PR 90 to 92 carry merge commits of the request branches, so after those are squash-merged into develop they will conflict in `core/lib.rs`, `engine/view.rs` and the engine tests.
Recommendation: yes, let the agent do that merge from develop when the owner says the request PRs are in.
Blocked: merging the window pages.

## 5. Diff-size limit (docs/WORK-PACKAGES.md)

Still unanswered, and not touched: the checks have no diff-size limit. No number was invented.
