<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M7: evaluation

**Owner crates:** `eval` (not part of the CI gate: it costs money). **Depends on:** M4. **Uses:** Contest Keeper's merged PRs as the first corpus.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M7-evaluation.md`."

## Goal
Measure quality and cost of the pipeline on real history so models, effort, round caps and prompts are tuned by evidence.

## Tasks
- [ ] Corpus builder: pick about 20 merged PRs with their issue text; record the base commit, the tests the PR added or changed, and the real diff for reference. Strip anything private.
- [ ] Runner: from the base commit, create the ticket from the issue text, run the pipeline with fixed settings, answer fixture questions with a scripted policy (accept recommended), and stop at the local publication request. Use non-UI tickets until M6 is available.
- [ ] Judge: the tests the original PR added must pass on our result (end state, not exact steps), plus one rubric call returning 0.0 to 1.0 and pass or fail for what tests cannot see.
- [ ] Report (done in `eval::Report`: pass rate over the tickets that ran, cost; open: time, turns, rounds, questions): resolve rate, dollars (API exact, subscription estimate), wall time, turns, rounds used, questions asked, questions skipped that the real PR needed answered, checks that failed first.
- [ ] Experiments to run first: model per step (cheap versus strong), effort per step, 2 versus 3 refine rounds, Understand with and without the ambiguity map, Codex as the independent reviewer (needs M8).
- [x] Budget guard: the runner refuses to start without a stated dollar cap; start with 5 tickets (`crates/eval`: `Plan::new`, `DEFAULT_TICKETS`; the runner is a closure, so no real run is wired to a CLI command yet).

## Tests that must exist
- The judge marks a known-good result pass and a known-bad one fail (fixtures).
- The runner stops at its dollar cap.

## Exit criteria
A report for at least 5 tickets with settings and cost, and the first tuning decisions written to `docs/decisions.md`.
