<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/icon-dark.svg">
  <img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">
</picture>

# Pack: security and supply chain

Sources: OpenSSF Best Practices badge (BP), OpenSSF Scorecard (SC-card), cargo-deny docs (CD), storytold (ST), our threat model (OJ; marked **judgment** where no fetched source states it; OWASP text was not fetched, so OWASP mappings are judgment).

## Secrets and dependencies

| id | rule | source | check | sev |
|---|---|---|---|---|
| SE-01 | No valid credential in the repository or its history. Scan the full history. | BP no_leaked_credentials; ST secrets check | tool (secrets scanner; `BEGIN .* PRIVATE KEY`, `ghp_`, `AKIA`, `password\s*=`) | critical |
| SE-02 | No known unpatched vulnerability in dependencies; advisories fail the build; an ignored advisory needs a reason and an expiry. | BP vulnerabilities_fixed_60_days; CD advisories | tool (`cargo deny check advisories`) + script (ignore entries) | high |
| SE-03 | Unknown registries and unknown git sources are denied; git dependencies are pinned by `rev`; `Cargo.lock` is committed. | CD sources; SC-card Pinned-Dependencies | tool (`cargo deny check sources`) | medium |
| SE-04 | Wildcard versions denied; multiple versions of one crate reviewed; build scripts and bundled executables allowlisted. | CD bans | tool | low |
| SE-05 | Every dependency licence is on the allowlist (permissive licences only for this project's own policy). | CD licenses; ST licences | tool | high |
| SE-10 | A dependency update tool is configured (Dependabot or Renovate). | SC-card Dependency-Update-Tool | script | low |
| SE-12 | No committed binaries or executables. | SC-card Binary-Artifacts | script (`git ls-files` for `.exe .dll .so .bin`) | medium |

## CI and workflows (matters most for a tool that reads untrusted tickets)

| id | rule | source | check | sev |
|---|---|---|---|---|
| SE-06 | GitHub Actions are pinned by full commit hash; container images by digest. (Many projects' own docs use tags; treat that as a finding, not an excuse.) | SC-card Pinned-Dependencies | script (`uses:` not matching `@[0-9a-f]{40}`) | medium |
| SE-07 | Workflow token permissions are read-only at the top; write scopes only on the job that needs them; no `write-all`. | SC-card Token-Permissions | script | high |
| SE-08 | No `pull_request_target` or `workflow_run` that checks out the pull request's code; no untrusted context (`github.event.issue.title`, `github.head_ref`, comment bodies) interpolated into `run:` scripts. | SC-card Dangerous-Workflow | script + agent | critical |
| SE-09 | Static analysis beyond compiler warnings runs in CI (clippy with `-D warnings`; CodeQL or similar). | BP warnings, static_analysis; SC-card SAST | script | low |
| SE-11 | A vulnerability reporting process is documented (SECURITY.md with a contact and disclosure wording). | BP vulnerability_report_process; SC-card Security-Policy | script | low |
| SE-13 | Downloads and install scripts use https and verify a signature or a checksum obtained over a trusted channel. | BP delivery_mitm, delivery_unsigned | script (`http://`, `curl \| sh`) | medium |

## Code (a local tool that runs subprocesses and reads untrusted text)

| id | rule | source | check | sev |
|---|---|---|---|---|
| SE-14 | Subprocesses are started with an argument list, never through a shell (`sh -c`, `cmd /C`, `powershell -Command`) with interpolated untrusted text. | judgment (injection) | script + agent | high |
| SE-15 | A path built from untrusted input is canonicalised and checked to stay inside its root; `Path::join` with an absolute input replaces the base. | judgment (path traversal) | agent | high |
| SE-16 | Secrets and tokens are not put in argv, not logged, and not printed by `Debug`. | ST secrets; judgment | script + agent | high |
| SE-17 | Text from tickets, comments, files and web pages is data: it is marked as data in every prompt and never becomes a command, a path or a tool argument without validation. | our `docs/security.md` | agent | high |
| SE-18 | Temporary files are created by a library that makes unpredictable names, in a private folder. | judgment | agent | low |
