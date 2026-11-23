<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M3: plan, build and check loop

**Owner crates:** `checks`, `pipeline` (plan and build steps). **Depends on:** M2. Real Build and Check also require M4 part 1 and S6 for the specific platform, launcher and auth mode. **Gate for:** M4 part 2 and M6.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do the first unchecked item of `docs/milestones/M3-plan-build-check.md`."

## Goal
A real small ticket on a sample repository goes from an approved plan to green checks, with the Build and Check loop refining on failure.

## Tasks
- [x] Plan step (no `passes` field exists for an agent to set; it will come from executed check evidence): acceptance criteria as JSON (`id`, text, `verify: test | command | unknown`, verification ID), a test plan, the file list. Store the approved plan outside the agent's write scope. The engine updates `passes` from executed evidence. Record agent progress separately.
- [x] Analyze pass (read-only, spec-kit style) before approval: every criterion has a verification; no vague words; coverage table; severity levels; at most 50 findings.
- [ ] Build step, adapter half done (PR 5a: `build_args` with the `PreToolUse` deny hook through `autopilot hook` and `policy::hook`, `--session-id` and `--resume` with an opaque `session-xxxxxxxx` reference mapped in `claude-sessions.json`, remaining turns and dollars as the call limits, a git-credential-free child environment, `Agent::start_build`); engine half done (PR 5b: `run_build` with a real `Round`, host-side commit after every turn, `Needs you` through `Asked` and an inbox request, fake-agent build scenarios, `autopilot build`, the S6 refusal for real agents); the agent-backed `Reviewer` and real-agent runs stay open (also carries what M1 deferred: `--settings` deny hooks that block `git push`, `--session-id` and `--resume` with repeated launch flags and an opaque local session reference, per-step effort from the profile, and M2's resume relaunch): one agent in a worktree (use the `git` crate's minimal API), repo map and touched files as context, stable prompt prefix and explicit tool grants. Require validated S6 isolation for the agent and all repository checks. Hold tickets requiring Design until the design is approved or the project configuration explicitly skips Design.
- [x] `checks` crate: `Check` trait (`id`, `applies`, `can_refine`, `configuration`, `run`) returning `passed`, `note`, `artifacts`, `unknowns`; an ordered runner that stops at the first failure; green results reused only for the same base, complete worktree fingerprint, approved plan and effective check configuration (a hash of the resolved command lines). Only passes are kept. Profile checks run in the worktree through `process` (`CommandSpec` gained `cwd`) with an allowlisted environment.
- [x] Checks in order: (1) build, fmt, clippy from the profile (keys `build`, `fmt`, `clippy`); (2) tests (key `test`); (3) protected paths and diff size; (4) plan criteria (each verified by its test or command; `unknown` cannot pass); (5) review by an agent in a fresh context, run only when 1 to 4 passed. `standard_checks` builds this order. Review is the `Reviewer` trait; its agent-backed implementation comes with the Build step. Check 3 fails on a protected or out-of-repository path and only reports the size: no limit exists in the docs or profile, so there is no diff-size failure yet. A `test` criterion appends its name to the profile `test` command and needs that name in the output; a `command` criterion must name a profile key. The agent never supplies a command.
- [x] Refine (`pipeline::refine`, driven through the `Round` trait so the agent and checks are injected): the failed note goes back to the same session; rounds capped (default 3); turn and dollar caps; the not-working counter (consecutive turns that leave the worktree unchanged; two stop the build) is checked before paying for another build turn; after two failed rounds start a fresh session with a summary of what failed. A failure with unknowns or a check that cannot refine goes to the owner at once. Tested with a scripted agent. The real `Round` comes with the Build step.
- [x] A stuck loop or an exhausted cap becomes `Needs you` with the last failure note: `run_build` appends `Asked` (`build_check-stuck`) and writes a forced approval request. Approve or edit continues with fresh round counters and the owner's note as a hint; reject interrupts. Tested with the fake agent in `crates/engine/tests/it/build.rs` (Windows only).
- [x] Code-made summaries for each check and round (`summary::check_line`, and a round line in `from_history`).

## Tests that must exist
- Checks against throwaway git repositories (assert_cmd, tempfile): each check passes and fails as it should; the runner stops at the first failure.
- "Unknown cannot pass": a criterion with `verify: unknown` never lets check 4 pass.
- Kept results: any file change invalidates; an unchanged tree reuses; a failing round is never kept.
- Caps: rounds, turns, dollars and the not-working counter each stop the loop (fake agent that never fixes anything).
- An agent report claiming `passes: true` cannot pass a criterion without a successful check. An altered copy of the plan cannot replace the approved plan.

## Exit criteria
A small real ticket on a sample repo reaches green checks, with at least one refine round exercised, driven from the CLI.

## Exit evidence (2026-11-23)

Two small tickets on a sample Rust crate went from an approved plan to green checks from the CLI with the real agent inside the WSL2 boundary (`docs/spikes/S6-containment.md#result`). Neither needed a refine round with the real model; refinement inside the sandbox is covered by `a_rust_build_is_checked_inside_the_sandbox_and_refined` (fake agent, real cargo, ignored, needs the distro). The agent review (check 5) ran on the second ticket.
