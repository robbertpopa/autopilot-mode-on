<img src="../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Codebase review and fix pipeline

A second kind of run next to the ticket pipeline. Input: a repository (and optionally paths, a git ref and rule presets). Output: a verified list of findings, and, when you ask, small fix pull requests that resolve them. It reuses the ticket pipeline for every fix, so nothing here bypasses the checks, the inbox or the budgets.

Research basis (2026-11-20): storytold's shared standards and craft-anything's xtask checks, Google's Engineering Practices (code review) and its large-scale changes chapter, the Rust API Guidelines, Rust for Linux coding guidelines, the Linux kernel's patch rules, OpenSSF Best Practices and Scorecard, cargo-deny, and recent work on LLM review (verification, false positives, refactoring with tests first). Sources are listed per rule in `docs/rules/`.

## Principles (what the research agreed on)

1. **A finding is a claim with evidence.** File and line, the code quoted, why it is wrong or costly, and a failure scenario or maintenance cost. A claim without evidence is dropped, not shown.
2. **Verify with different evidence than the finder had.** A verifier that shares the finder's reasoning shares its false positives. The verifier gets tool output (a build, a grep for callers, an independent functional check or a defensive control assertion) and a neutral question, not "refute this" and not "confirm this". If verification fails or errors, the finding stays but is marked unverified.
3. **Test the reviewer.** Plant known defects and known false positives in a sample repo; measure recall and how often the pipeline takes the bait.
4. **Mechanical first.** Anything a compiler, clippy, cargo-deny, a secrets scanner or a grep can find is found by code, for free, before any model reads a file.
5. **Review and fix are separate stages with a human decision between them.** Report-only is the default.
6. **Small, separate changes.** One logical change per pull request; refactors and moves apart from behaviour changes; mechanical sweeps by tool and sharded; characterization tests before risky refactors. Roughly 100 lines is a good size and 1,000 is too many (Google's guidance, applied as a configurable limit).
7. **Improve code health, not perfection.** A change is good when it definitely improves the codebase (Google's approval standard). Nits are labelled and never block.
8. **Cap the cost.** Estimate before starting; stop when yield falls; cache the rule prefix; use the strong model only where it pays.

## Flow

```
Scope -> Inventory (code) -> Review (agents, read-only, parallel) -> Verify -> Triage -> [you choose] -> Plan fixes -> Fix (ticket pipeline, per batch) -> Re-review and report
```

| Step | Done by | Output | Asks you |
|---|---|---|---|
| **Scope** | you + Rust | repository, ref, paths, presets, mode, budget cap, ignore globs | the mode and the cost estimate |
| **Inventory** | code, no model | file tree, languages, sizes, module and dependency map, test map, git churn, excluded generated and vendored code; **review units** (a module or a folder of a size one agent can read in full); mechanical findings from the packs (lints, cargo-deny, machete, secrets scan, grep and ast-grep patterns, workflow checks) | no |
| **Review** | one fresh agent per unit, parallel, read-only tools, no network | structured findings (below), at most N per unit, each tied to a rule id or to a lens category | no |
| **Verify** | a different session, ideally a different model, per finding batch | `verified`, `refuted`, or `unverified`, with the evidence used | no |
| **Triage** | code plus a cheap model for clustering | deduplicated findings by location and rule; clusters; ranked by severity, confidence and fix risk; a report (`review-report.md` in the data folder) | yes: which findings to fix, which to dismiss with a reason |
| **Plan fixes** | code plus the Plan step's agent | batches, each one logical change; order; which need a characterization test first; which are mechanical sweeps for a tool | yes, when fixing is on |
| **Fix** | the ticket pipeline: each batch is a ticket with its plan filled in from the findings | draft PRs, small and separate | per the step checkboxes |
| **Re-review and report** | code | re-run the mechanical scans on the result; before and after counts; cost; rule hit rates | no |

## Modes
- `report-only` (default): stop after Triage. Nothing is changed.
- `fix-safe`: after you approve the list, fix only findings marked mechanical or low behaviour risk.
- `fix-all`: also behaviour-risky findings, each with a characterization test first and an approval at the Plan step.

## The finding

```json
{
  "id": "F-0042",
  "rule": "RC-01",
  "category": "correctness|security|design|complexity|tests|naming|comments|docs|consistency|dependencies",
  "severity": "critical|high|medium|low|nit",
  "confidence": 0.0,
  "where": {"file": "crates/x/src/y.rs", "lines": [118, 131]},
  "claim": "one sentence",
  "evidence": "the quoted code or tool output",
  "impact": "a failing scenario, or the maintenance cost",
  "fix": {"summary": "one sentence", "kind": "mechanical|local|behaviour", "needs_test_first": false, "estimate_lines": 12},
  "source": "mechanical|agent",
  "status": "raised|verified|refuted|unverified|dismissed|fixed"
}
```
Rules: no finding without `where`, `evidence` and `impact`; `nit` findings are prefixed "Nit:" and never block; style that rustfmt handles is never a finding; the same rule raised twice at one location is one finding.

## Rule packs and presets
Rules live in `docs/rules/` (later in `rules/*.toml`, see M9). Each has an id, a source, a check kind (`lint`, `script`, `tool`, `agent`), a default severity and fix guidance. Projects choose presets in `profile.toml`:

| Preset | Contains | Origin |
|---|---|---|
| `craft` | the storytold house rules that apply to any Rust codebase: no panics, no lint silencing, unsafe with SAFETY, hostile input, test keep rule, one integration binary per crate, no hand-rolled date math, layering, secrets, PR shape | storytold craft-contrib and craft-anything |
| `rust-api` | the Rust API Guidelines checks worth a review (for libraries) | rust-lang |
| `kernel` | safety documentation, panics docs, `#[expect]` over `#[allow]`, one logical change, describe the problem first | Rust for Linux, Linux kernel |
| `supply-chain` | secrets, advisories, pinned actions, token permissions, dangerous workflows, SAST, security policy | OpenSSF Best Practices and Scorecard, cargo-deny |
| `lens` | Google's reviewer checklist as the agent's lens: design, functionality, complexity, tests, naming, comments, consistency, documentation | Google Engineering Practices |
| `ai-tooling` | untrusted text as data, subprocess and path safety, secrets in argv and logs (for tools like this one) | our threat model |

A project can disable a rule with a reason, change a severity, add its own rules, and keep an **accepted-risk ledger** (finding, reason, owner, expiry) that later runs respect until it expires.

## Read-only review

Use an immutable target snapshot. Put build output, scanner caches and reports in the run's own data directory. Run repository build and check commands through the verified S6 boundary. This protects the target tree even when a normal tool writes build artifacts.

## Fixing safely
- Each batch becomes a normal ticket: the Plan step starts from the finding (criteria: the rule no longer fires at that location, the verifier's failing test now passes, nothing new fires in the touched files), then Build and Check.
- Extra checks for fix tickets: no lint silencing to make a finding disappear (`#[allow]` is a finding itself); no test deleted, ignored or weakened; no CI workflow change unless the finding is about workflows (then a forced confirmation); a changed `Cargo.toml` comes with `Cargo.lock`; the diff stays inside the batch's file list.
- Characterization tests first for behaviour-risky refactors: pin the current behaviour, then change the code.
- Mechanical sweeps above about 500 edits use a tool (clippy fix, a codemod, ast-grep) and are reviewed as a pattern plus samples, in shards that each pass the checks alone.
- Order: tests and tools first, mechanical sweeps next, local fixes, then behaviour changes. Stack small pull requests; never one big one.

## Cost control
- **Estimate first:** units times the token size of each plus the rule prefix; show it at Scope and ask before starting if it passes the cap.
- **Cheap first pass:** a fast model reviews units; the strong model verifies only critical and high findings.
- **Cache the prefix:** rules and the lens are one stable prompt prefix.
- **Incremental runs:** after the first full run, review only units changed since the last reviewed commit.
- **Stop early:** end a unit when yield drops; cap findings per unit.
- **Dollar caps** per step and per run, as everywhere.

## What the window shows
A unit map (findings per unit), a findings table (severity, rule, file and line, verified chip, fix kind), per-finding actions (accept for fixing, dismiss with a reason into the ledger, open the file), the batch plan with sizes, and the status of each fix pull request. Events: `ReviewScoped`, `UnitReviewed`, `FindingRaised`, `FindingVerified`, `FindingDismissed`, `BatchPlanned`.

## Evaluation of the reviewer itself
- A sample repository with seeded defects (recall) and seeded false-positive bait (precision), run in M9 and re-run when prompts or models change.
- Per rule: how often it fires, how often a human dismisses it, how often a fix sticks. A rule with a high dismissal rate is proposed for demotion.
- Cost per verified finding.

## Open questions
- How good is a model-only review without the mechanical findings (S9 measures it)?
- Is a different model family a better verifier (needs the Codex adapter, M8)?
- Which presets suit which projects (the Contest Keeper repositories are the first test).
