# S9 seed for the review experiment

The public Autopilot repository supplies 14 Rust files from its fixture-backed CLI, engine, local history, configuration and fake agent. The seeded copy has 2,154 physical lines and 67,322 UTF-8 bytes with LF endings. The sample uses the pinned Apache-2.0 revision in [manifest.json](manifest.json).

## Prepare the sample

Apply the [seed patch](seed.patch) only in a fresh detached worktree at the exact manifest revision. The patch has zero context and relies on that revision's original line positions. From the repository root in PowerShell, run the following commands.

~~~powershell
$root = git rev-parse --show-toplevel
$manifestPath = Join-Path $root 'fixtures/review/s9/manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$patch = Join-Path $root 'fixtures/review/s9/seed.patch'
$sample = [IO.Directory]::CreateTempSubdirectory('autopilot-s9-').FullName
git -C $root worktree add --detach $sample $manifest.base
if ($LASTEXITCODE) { throw 'Could not open the pinned sample revision' }
git -C $sample apply --check --unidiff-zero $patch
if ($LASTEXITCODE) { throw 'The seed does not apply to the sample' }
git -C $sample apply --unidiff-zero $patch
if ($LASTEXITCODE) { throw 'Could not apply the seed' }
~~~

Use only the paths under `sources` as model input. Their line counts and SHA-256 digests describe the seeded files after CRLF-to-LF normalization. Keep this directory, the seed patch, the manifest and the base checkout out of the reviewer's and verifier's view. Any verification test results should cite behavior in the seeded copy.

## Score a run

- Match each distinct claim to a case by file, overlapping lines and the issue described in the manifest. The rule or lens category helps distinguish claims near each other. Count a seeded defect at most once, even when both the mechanical and model checks raise it.
- Count a claim matching a bait case as a false positive when it asserts the behavior that the label's evidence rules out. Independently adjudicate claims outside the labels. Record genuinely new findings and unresolved claims separately.
- Record true positives, false positives and unresolved claims at the finder and verifier stages. For the verifier, `unverified` claims remain visible and do not count toward verified precision. Report that count alongside the metrics.
- Seeded recall is unique seeded defects found divided by seeded defects in scope. Precision is confirmed true positives divided by adjudicated true and false positives, with the unresolved count alongside it. Report bait separately as unique bait cases raised divided by bait cases in scope. Record input and output tokens, elapsed time, measured spend and spend per verified true finding. A zero denominator means there is no value to report.

The sample should build. The seed intentionally changes behavior that existing profile and fixture-agent tests cover, and it produces an unused-function warning. Real model token counts, precision, recall and cost remain to be measured.
