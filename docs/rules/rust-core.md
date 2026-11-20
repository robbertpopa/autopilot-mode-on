<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Pack: rust-core

Panics, lints, unsafe, errors, input, API shape. Sources: storytold house rules (SC), Rust API Guidelines (API), Rust for Linux (RFL), Clippy docs (CL), craft-anything pitfalls (PF), our judgment (OJ).

| id | rule | source | check | sev |
|---|---|---|---|---|
| RC-01 | Non-test code never panics: crate roots or workspace lints deny `unwrap_used`, `expect_used`, `panic`, `unimplemented`, `todo` and `unreachable`; failures return errors. Tests may use them. | SC house-rules-rust | lint + script (crate roots without the deny) | high |
| RC-02 | Fix the lint; do not silence it. A lint that truly must be waived uses `#[expect(lint, reason = "...")]`, not `#[allow]`, so a stale waiver is reported (`unfulfilled_lint_expectations`). Exceptions: cfg-dependent code and macro expansion. Editing lint configuration to make a finding disappear is itself a finding. | SC app-agents; RFL | script (`#\[allow\(`, diffs of lint config) + lint | medium |
| RC-03 | `unsafe` only in modules the repository names; every `unsafe` block has a `// SAFETY:` comment saying why it is sound; every `unsafe fn` documents its preconditions under `# Safety`. | SC; RFL | lint (`undocumented_unsafe_blocks`, to be confirmed) + script (`unsafe` without the comment) | high |
| RC-04 | Public functions that can fail or panic document it (`# Errors`, `# Panics`). Libraries only; an application may skip. | API C-FAILURE; RFL | agent (lints `missing_errors_doc`, `missing_panics_doc` exist in Clippy's pedantic group) | low |
| RC-05 | Branch on the typed error kind, never on message text; retry calls that return `Interrupted`. | PF | script (`to_string().contains(`, comparing error strings) + agent | medium |
| RC-06 | A "program not found" from a spawn does not prove the program is absent: look it up first. Waits after a kill have a deadline. | PF | agent + script (`.kill()` followed by an unbounded `.wait()`) | medium |
| RC-07 | Treat input as hostile: bound recursion, allocation and read sizes; parsers, decoders and command handlers never panic on any input. | SC house-rules-rust; testing standard | agent; a hostile-input test must exist (see TS-08) | high |
| RC-08 | Arguments carry meaning through types, not bare `bool` or `Option` flags; no out-parameters; functions validate their arguments. | API C-CUSTOM-TYPE, C-NO-OUT, C-VALIDATE | script (functions with two or more `bool` parameters) + agent | low |
| RC-09 | Types with invariants keep their fields private; public types implement `Debug`, and types holding secrets implement it with the secret redacted. | API C-STRUCT-PRIVATE, C-DEBUG; OJ for redaction | agent + script (`derive(Debug)` on a type with a field named like a key, token or password) | low |
| RC-10 | Calendar and date arithmetic uses a date library; no hand-rolled leap-year or days-to-date math. | SC time standard | script (`% 400`, `146097`, `is_leap`, `86400` arithmetic) | medium |
| RC-11 | Lint configuration enables Clippy's correctness, suspicious, style, complexity and perf groups; `pedantic` is cherry-picked; `restriction` lints are enabled one by one, never as a group. `correctness` lints are never allowed. | CL lint groups | script (`Cargo.toml` and `clippy.toml`) | low |
| RC-12 | Each member manifest sets `[lints] workspace = true`, so one lint policy applies everywhere. | SC craft-anything xtask test | script | low |

Notes
- Panics in `main`-level startup code that cannot continue are allowed if they are `expect` with a clear message and the repository says so (a documented, listed exception).
- Clippy restriction lints we use in our own repositories (`unwrap_used`, `expect_used`, `print_stdout`, `dbg_macro`, `todo`) are named from those configurations; confirm each name against Clippy's lint pages when writing the pack's lint file.
