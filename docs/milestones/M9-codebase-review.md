<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# M9: codebase review and fix

**Owner crates:** `rules` (pack loader), `review` (units, findings, verification, triage, batching). **Depends on:** M1 through M3 and S9 for report-only. Apply the validated S6 runtime when executing repository code or agents on a real project. Fixes and draft PRs also need M4. **Design:** `docs/REVIEW-PIPELINE.md`; rules in `docs/rules/`.
**Start prompt:** "Read `AGENTS.md`, `docs/IMPLEMENTATION-PLAN.md` and `docs/REVIEW-PIPELINE.md`, then do the first unchecked item of `docs/milestones/M9-codebase-review.md`."

## Goal
Point the tool at a repository and get a verified findings report; then, on request, small fix pull requests that resolve chosen findings through the normal ticket pipeline.

## Tasks
- [ ] `rules` crate: convert `docs/rules/*.md` tables to `rules/*.toml` (id, rule, source, check kind, severity, fix guidance, how-to-check data such as clippy lint names, grep or ast-grep patterns, tool commands); loader with presets; project overrides with a reason; schema validation; the accepted-risk ledger (finding, reason, owner, expiry).
- [ ] Inventory (code, no model): file tree, languages, sizes, module and dependency map, test map, git churn; skip generated and vendored code; build review units that one agent can read in full; estimate tokens and cost.
- [ ] Mechanical scans from the packs, run as checks and parsed into findings: Clippy with the pack's lint set (JSON output), `cargo deny`, `cargo machete`, secrets scan, grep and ast-grep patterns, workflow checks (pinned actions, token permissions, dangerous patterns).
- [ ] Review step: one fresh read-only agent per unit, in parallel up to the slot limit; stable prompt prefix (rules plus lens); structured findings via `--json-schema`; per-unit caps; findings without evidence are rejected by code.
- [ ] Verify step: a separate session (a different model when available) per batch; neutral prompt; extra evidence (build or check results, caller search, an independently run functional check for correctness findings or a defensive control assertion for security findings); `verified | refuted | unverified`; an error keeps the finding as unverified.
- [ ] Triage: dedupe by location and rule, cluster, rank; write `review-report.md` and the events; the inbox asks which findings to fix and which to dismiss with a reason.
- [ ] Fix planning: batches of one logical change each under the size limit; refactors and moves apart from behaviour changes; flag batches that need a characterization test first; mechanical sweeps go to a tool and are sharded.
- [ ] Fix: each batch becomes a ticket whose Plan is filled from the findings (criteria: the rule no longer fires there, the verifier's test passes, nothing new fires in touched files); extra checks from `docs/rules/change-shape.md` (no lint silencing, no test removal, no workflow changes without a confirmation, `Cargo.lock` with `Cargo.toml`, diff inside the batch's files).
- [ ] Re-review and report: re-run the mechanical scans on the result; before and after counts; cost; per-rule hit and dismissal rates.
- [ ] Modes `report-only` (default), `fix-safe`, `fix-all`; incremental runs over units changed since the last reviewed commit.
- [ ] Profile `[review]` section: presets, rule overrides, ignore globs, unit size, parallelism, models per step, size limit, dollar cap; the window shows the unit map, findings table, per-finding actions and the batch plan (events `ReviewScoped`, `UnitReviewed`, `FindingRaised`, `FindingVerified`, `FindingDismissed`, `BatchPlanned`).
- [ ] CLI: `autopilot review run|report|dismiss|fix`.

## Tests that must exist
- A finding without evidence, file and line, or impact is rejected before it is stored.
- Verification: a planted false positive is refuted; a planted real defect is verified; a verifier error leaves the finding unverified rather than dropping it (fake agent).
- Dedupe: the same rule at one location twice is one finding.
- Mechanical parsers (Clippy JSON, cargo-deny output) never panic on truncated or corrupted output; every fixture parses.
- The accepted-risk ledger suppresses a finding until its expiry and no longer after.
- A fix ticket that adds `#[allow]`, removes a test, or edits a workflow without a confirmation fails its check.
- Report-only uses a read-only target snapshot, with generated reports and build output outside it. Compare the target tree before and after.
- The cost estimate stops a run that would pass the cap before it starts.

## Exit criteria
A report-only review of a real repository (start with one Contest Keeper crate), with verified findings, a measured false-positive rate and cost per verified finding from the seeded sample; then one fix batch that opens a draft pull request through the normal pipeline.
