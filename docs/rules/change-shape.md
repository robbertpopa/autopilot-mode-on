<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# Pack: change shape (rules for the fix pull requests themselves)

These rules govern the changes the pipeline makes, not the code it reviews. They are checks on every fix ticket. Sources: Google Engineering Practices (GG) incl. large-scale changes (LSC), Linux kernel submitting-patches (KP), storytold pull-request rules (SC), craft-anything pitfalls (CA), recent tests-first refactoring studies (R), our settings (OJ).

| id | rule | source | check | sev |
|---|---|---|---|---|
| CS-01 | One logical change per pull request and per commit; solve one problem; include its tests. | GG small CLs; KP | script (diff stays in the batch's file list) + agent | high |
| CS-02 | Refactors, code moves and formatting are separate from behaviour changes; a code move is not edited in the same change as the move. | GG; KP; SC pull-requests | agent (compare moved and changed hunks) | high |
| CS-03 | Size: about 100 changed lines is a good size; over about 1,000 needs a split unless it is a tool-made sweep. The default limit is configurable (our default: 300 lines per fix pull request). | GG small CLs (the 100 and 1,000 figures); the 300 default is OJ | script | medium |
| CS-04 | Every commit builds and passes the tests, so history can be bisected. | KP | script (check each commit of a batch) | medium |
| CS-05 | The description states the problem and its impact first, then the change; performance claims come with numbers; it says what was not run. | KP; SC pull-requests; SC app-agents 33 | agent | medium |
| CS-06 | A mechanical sweep of more than about 500 edits is made by a tool (clippy fix, a codemod, ast-grep), reviewed as a pattern plus samples, and sharded into independently testable batches, each of which can be rolled back alone. | GG LSC | agent + script | high |
| CS-07 | Behaviour-risky refactors get characterization tests first, then change in bounded slices. | R (case studies); Sourcegraph guide; GG | agent | high |
| CS-08 | A fix does not change CI workflows, dependency revisions or release configuration unless the finding is about them; then it asks. | SC app-agents 17, pull-requests | script (touched paths) | high |
| CS-09 | A changed `Cargo.toml` ships with its `Cargo.lock`. | CA pitfalls | script | low |
| CS-10 | No scratch files, agent tool folders or `--no-verify`; no secrets; no screenshots committed. | SC app-agents 35 | script | medium |
| CS-11 | A finding that is dismissed gets a reason, an owner and an expiry in the accepted-risk ledger; "clean up later" without a tracked item is not allowed. | GG pushback ("cleaning up later rarely happens") | script (ledger) | low |
| CS-12 | A fix never silences the rule that found it (no new `#[allow]`, no rule disabled, no test removed) to make the finding go away. | SC app-agents 13 and 31 | script | high |
