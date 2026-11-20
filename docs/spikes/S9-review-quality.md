# S9: how good is a model-only code review, and what does it cost?

**Question:** on a real Rust crate, how many of an agent's findings are real, how many are noise, what does a unit cost, and how much do the mechanical findings and a verifier improve it? **Gates:** the design of M9 (unit size, caps, which steps need the strong model). **Spending:** use the remaining shared $1 live-spike budget, with a dedicated capped workspace and at most $0.05 per call. If the measured experiment needs more, obtain a separate budget first. Prepare a synthetic corpus locally while M1 is in progress.
**Start prompt:** "Read `AGENTS.md`, `docs/REVIEW-PIPELINE.md` and `docs/rules/`, then do `docs/spikes/S9-review-quality.md`."

## Method
- [ ] Select a synthetic or approved public Rust sample of about 2,000 to 5,000 lines, then record its size in tokens. Obtain project permission before sending private source to a provider.
- [ ] **Seed a copy** with about 10 known defects across the packs (an `unwrap` on parsed input, an undocumented `unsafe`, a shell-interpolated command, a path join with untrusted input, a restating comment, a `#[allow]` that hides a lint, an unpinned action, a secret-looking string, a test that sleeps, a dead function) and about 5 pieces of **bait** (code that looks wrong but is guarded or intentional).
- [ ] Run three variants and record findings, true positives, false positives, missed defects, tokens, dollars and time:
  1. model-only review with the lens, no mechanical findings;
  2. the same with the mechanical findings (Clippy with the pack lints, `cargo deny`, grep patterns) given to the agent;
  3. variant 2 plus a separate verifier with the neutral prompt and extra evidence (a build or caller search; an independent functional check for correctness or a defensive control assertion for security findings).
- [ ] Vary unit size (one file, one module, a whole crate) and the findings cap; note where quality drops.
- [ ] Try a different model family as verifier if the Codex adapter is available (S8 first); otherwise a different Claude model.
- [ ] Record, per rule, how often it fired and how often it was wrong.

## Done when
`docs/REVIEW-PIPELINE.md` is updated with: the recommended unit size and cap; whether the mechanical findings are given to the finder; whether the verifier is worth its cost and which model; measured precision and recall on the seeded repository; cost per verified finding; and the rules to demote or tighten. The seeded repository is kept as the M9 test fixture.

## Result
(fill in)
