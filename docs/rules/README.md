<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# Rule packs

Rules the review pipeline checks, each with its source. Packs: `rust-core.md`, `structure-and-comments.md`, `tests.md`, `security-and-supply-chain.md`, `change-shape.md`, and the reviewer's lens in `review-lens.md`. Presets (which rules a project turns on) are listed in `docs/REVIEW-PIPELINE.md`.

## Format
Each rule is a table row: `id | rule | source | check | default severity`.
- **check**: `lint` (compiler or clippy), `script` (grep, ast-grep, xtask-style script), `tool` (cargo-deny, cargo-machete, a secrets scanner, cargo-mutants), `agent` (needs judgment; the finding must carry evidence).
- **severity**: `critical`, `high`, `medium`, `low`, `nit`. A project may change it with a reason.
- Ids are stable: `RC` rust-core, `ST` structure, `CM` comments and docs, `TS` tests, `SE` security, `CS` change shape. Never reuse an id.
- M9 converts these tables to `rules/*.toml` mechanically; until then the tables are the source of truth.

## What gets in: "only the best advice"
A rule is included only if it passes all of these:
1. **High value:** it prevents a bug, a security hole or lasting maintenance cost.
2. **Checkable:** a tool can find it, or a reviewer can point at file and line with evidence.
3. **Not formatting:** rustfmt and the compiler handle style.
4. **Low false-positive cost:** either rare false positives, or cheap to verify.
5. **Overridable:** a project can waive it with a stated reason.
6. **Sourced:** its origin is recorded. Rules that are our own judgment say so.

## Sources used, and how fully they were read
| Source | Read | Notes |
|---|---|---|
| storytold `craft-contrib/standards` (`app-agents.md`, `house-rules-rust.md`, `house-rules-core.md`, `pull-requests.md`, `claims.md`, `licences.md`, `time.md`, testing standard) and `craft-anything` (`docs/pitfalls.md`, `xtask`) | mostly | pitfalls read to about 40% verbatim; xtask check bodies not read; their `testing.md` was read earlier in full |
| Google Engineering Practices: reviewer and developer guides; Software Engineering at Google, large-scale changes | fully, except the tail of the LSC chapter (summary only) | |
| Rust API Guidelines checklist; Clippy lint groups and configuration | fully | the per-lint pages for restriction lints were not fetched; those lint names come from our own repositories' configs and Clippy's naming, and should be confirmed when the config is written |
| Rust for Linux coding guidelines; Linux kernel submitting-patches | the first ~80% of submitting-patches verbatim | |
| OpenSSF Best Practices (passing) and Scorecard checks; cargo-deny config | fully | OWASP text was not fetched; OWASP mappings are our judgment |
| Recent work on LLM review and AI refactoring (search results) | snippets only | used for the verification and tests-first principles; not as hard rules |

## Left out on purpose
| Left out | Why |
|---|---|
| All of Clippy's `restriction` group and all of `pedantic` | Clippy's docs say not to enable `restriction` wholesale and that `pedantic` has false positives. Individual lints are chosen in `rust-core.md`. |
| storytold product rules: egui and accessibility, translations, fonts, trademarks, "Rust only, no webview", git dependencies only from their org, one craft-libs revision | Their product and organisation policy, not general code health. The generic parts (pin git dependencies by revision; layering) are kept. |
| Kernel Signed-off-by (DCO), `checkpatch.pl`, subject length, FFI alias rules | Specific to the kernel's process and C interop. |
| Google's review turnaround (one business day) and escalation process | Team process, not a code check. |
| Scorecard Branch-Protection, Code-Review, Signed-Releases, Fuzzing detection, CI-Tests | Repository settings or process, checked in GitHub, not in code. Reported as information only. |
| OWASP web items (XSS, CSRF, session handling) | Not applicable to a local command-line and desktop tool. The injection and path items that do apply are kept. |
| Formatting, import layout, comment capitalisation | rustfmt and the compiler. |
