<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# The reviewer's lens

What the review agent is told to look at for each unit, how severe things are, and how it must report. Based on Google's Engineering Practices (reviewer guide) plus the verification findings in `docs/REVIEW-PIPELINE.md`.

## What to look for (in this order)
1. **Design:** do the interactions make sense; does this code belong here or in a shared place; is it more than the problem needs.
2. **Functionality:** does it do what was intended, and is that good for users and for the next developer; think about edge cases, concurrency and failure paths.
3. **Complexity:** can it be understood quickly; is it over-engineered or speculative.
4. **Tests:** are there tests that would fail if the code broke; are they correct and useful (see `tests.md`).
5. **Naming and comments:** clear names; comments that say why (see `structure-and-comments.md`).
6. **Consistency and style:** only where the style guide is silent; never what rustfmt handles.
7. **Documentation:** docs updated where behaviour, build, test or release changed.
Then the rules of the enabled packs, for the unit's files. Read the whole file around any line you comment on, and every line of the unit; say which files you did not read.

## Standard
Favour findings that **definitely improve overall code health**. There is no perfect code, only better code. Label optional points "Nit:" or "Optional". Explain why, comment on the code not the author, and do not report taste as a defect.

## Severity
| Level | Meaning | Examples |
|---|---|---|
| critical | exploitable now, data loss, or credential exposure | secret in history; workflow that runs untrusted input as code |
| high | a bug or security flaw on a reachable path, or a rule whose violation causes bugs | unchecked input reaching a shell; `unwrap` on parsed input |
| medium | likely to cause bugs or real maintenance cost | undocumented `unsafe`; error matched by message text |
| low | worth fixing in passing | restating comment; dead code |
| nit | optional; never blocks | naming preference |

## What every finding must contain
`where` (file and line range), `evidence` (the quoted code or tool output), `impact` (a failing scenario, or the maintenance cost), `fix` (kind: mechanical, local or behaviour; whether a test is needed first), a rule id or a lens category, and a confidence from 0 to 1. A claim without evidence is dropped.

## Limits
At most N findings per unit (default 15), highest severity first; stop when new findings are all `low` or `nit`. Read-only tools only; no network; never run the code under review.

## For the verifier
Ask neutrally: "Does this claim hold? Cite the evidence." Do not frame it as "find the flaw in this finding" or as "this is correct". Give it evidence the finder did not have: the result of building or running a check, a search for callers or guards, and for critical and high findings a failing test it must try to produce. If it cannot decide, return `unverified`; an error in verification keeps the finding (as unverified), never silently drops it.

## Testing the reviewer
Seed a sample repository with known defects and with known false-positive bait (code that looks wrong but is guarded). Measure recall, and how often the verifier is fooled.
