<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# Pack: tests

Sources: storytold's testing standard and app-agents rules (SC), craft-anything pitfalls (CA), our testing doc (OJ), recent work on tests before refactoring (R). In a **review** these produce findings (candidates to add or to cut). A review never deletes a test by itself; cuts happen in their own pull request that lists every removed test.

| id | rule | source | check | sev |
|---|---|---|---|---|
| TS-01 | A test earns its place: it states one invariant, a plausible change that breaks it, and why no other test catches that. Candidates to cut: tests of readable code, wiring, defaults and constants, compiler-checked properties, library behaviour, duplicates, several values through one branch. | SC testing standard | agent | low |
| TS-02 | Tests assert relations, not exact values we chose or timings; no `sleep` as a synchroniser; inject the clock. | SC app-agents 28; testing standard F1 F2 | script (`sleep`, `Instant::now` in tests) + agent | low |
| TS-03 | A bug fix ships a test that fails without the fix, unless the fixed code is a few lines with one branch and no state. | SC app-agents 29 | agent (compare diff and tests) | high for fix PRs |
| TS-04 | No test is deleted, weakened, `#[ignore]`d or skipped to make a check pass. A quarantine names an issue. | SC app-agents 31 and 32 | script (`#\[ignore\]` without a reason, removed `#[test]` in the diff) | high |
| TS-05 | One integration test binary per crate (`tests/it/main.rs`), not one per file. | SC app-agents 30 | script (files directly in `tests/`) | low |
| TS-06 | Tests never touch the network, installers, the real configuration folder or a real agent; they use temporary folders and fakes. | CA pitfalls; OJ | script + agent | medium |
| TS-07 | Tests that touch process-wide state (environment, statics, current directory) take a shared lock or live in their own binary. | CA pitfalls | script (`set_var`, `static mut`, `set_current_dir` in tests) | medium |
| TS-08 | Every parser, decoder and command handler has a hostile-input test (truncation, corruption, junk parameters) asserting it never panics; a crash fix adds its input to that test. | SC testing standard | agent | medium |
| TS-09 | Untested behaviour is evidence: where mutation testing is cheap, surviving mutants show tests that guard nothing. Report as information, never as a failure. | OJ (cargo-mutants) | tool | info |
| TS-10 | Tests that run in no CI job check nothing: `#[ignore]` and platform-gated tests must be run by a named job or be cut. | SC testing standard | script + agent | low |
