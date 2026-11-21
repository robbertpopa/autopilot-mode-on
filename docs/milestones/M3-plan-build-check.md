<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M3: plan, build and check loop

**Owner crates:** `checks`, `pipeline` (plan and build steps). **Depends on:** M2. Real Build and Check also require M4 part 1 and S6 for the specific platform, launcher and auth mode. **Gate for:** M4 part 2 and M6.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M3-plan-build-check.md`."

## Goal
A real small ticket on a sample repository goes from an approved plan to green checks, with the Build and Check loop refining on failure.

## Tasks
- [ ] Plan step: acceptance criteria as JSON (`id`, text, `verify: test | command | unknown`, verification ID, `passes: false`), a test plan, the file list. Store the approved plan outside the agent's write scope. The engine updates `passes` from executed evidence. Record agent progress separately.
- [ ] Analyze pass (read-only, spec-kit style) before approval: every criterion has a verification; no vague words; coverage table; severity levels; at most 50 findings.
- [ ] Build step (also carries what M1 deferred: `--settings` deny hooks that block `git push`, `--session-id` and `--resume` with repeated launch flags and an opaque local session reference, per-step effort from the profile, and M2's resume relaunch): one agent in a worktree (use the `git` crate's minimal API), repo map and touched files as context, stable prompt prefix and explicit tool grants. Require validated S6 isolation for the agent and all repository checks. Hold tickets requiring Design until the design is approved or the project configuration explicitly skips Design.
- [ ] `checks` crate: `Check` trait (`id`, `applies`, `can_refine`, `run`) returning `passed`, `note`, `artifacts`, `unknowns`; an ordered runner that stops at the first failure; reuse green results only for the same base, complete worktree fingerprint, approved plan and effective check configuration.
- [ ] Checks in order: (1) build, fmt, clippy from the profile; (2) tests; (3) protected paths and diff size; (4) plan criteria (each verified by its test or command; `unknown` cannot pass); (5) review by an agent in a fresh context, run only when 1 to 4 passed.
- [ ] Refine: the failed note goes back to the same session; rounds capped (default 3); a not-working counter stops the run before paying for another build turn; after two failed rounds start a fresh session with a summary of what failed; two turns that change nothing in the worktree stop the build.
- [ ] A stuck loop or an exhausted cap becomes `Needs you` with the last failure note.
- [ ] Code-made summaries for each check and round.

## Tests that must exist
- Checks against throwaway git repositories (assert_cmd, tempfile): each check passes and fails as it should; the runner stops at the first failure.
- "Unknown cannot pass": a criterion with `verify: unknown` never lets check 4 pass.
- Kept results: any file change invalidates; an unchanged tree reuses; a failing round is never kept.
- Caps: rounds, turns, dollars and the not-working counter each stop the loop (fake agent that never fixes anything).
- An agent report claiming `passes: true` cannot pass a criterion without a successful check. An altered copy of the plan cannot replace the approved plan.

## Exit criteria
A small real ticket on a sample repo reaches green checks, with at least one refine round exercised, driven from the CLI.
