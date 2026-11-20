# S8: Codex on a ChatGPT subscription, and switching to an API key

**Question.** Can `codex exec` use a ChatGPT plan, can the project switch auth modes without sharing a cached login, and what do OpenAI's terms say about unattended use? S8 gates the Codex adapter (M8).

## Documentation checked on 2026-11-21

- The CLI supports a ChatGPT sign-in and API keys. Eligible Enterprise deployments also have admin-granted access tokens for unattended jobs. The CLI, IDE extension and desktop app can share cached credentials. Codex supports a file store under `CODEX_HOME`, an OS credential store and other configured storage modes. The project should leave interactive sign-in to Codex and treat the file store as secret. [Codex authentication](https://learn.chatgpt.com/docs/auth) · [Configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference#cli-auth-credentials-store)
- `codex login status` reports credential presence. The documented success exit code is 0. It does not establish that a credential will pass a model request. Interactive setup can pass an API key to `codex login --with-api-key` on stdin. [CLI commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli)
- OpenAI recommends API keys for script and CI use. `codex exec` documents the `CODEX_API_KEY` environment variable for per-invocation authentication. A provider launch should supply a dedicated key only to the verified provider environment. The account-authenticated CI guidance covers trusted private workflows. [Non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
- `codex exec --json` emits JSONL events for threads, turns, items and errors. The guide documents a read-only default sandbox and an optional approval mode. Network policy can depend on sandbox and configuration. Actual process containment, approvals, network and event schemas are still to be tested. [Non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode) · [Configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)

## Terms assessment

API integration is the documented automation route, subject to the applicable agreement and account controls. The OpenAI Services Agreement permits API integration in customer applications and restricts actions including bypassing usage limits or safeguards. The European individual Terms of Use restrict automatic extraction of data or output. The Codex docs describe a trusted private account-authenticated CI workflow, whose scope depends on account and organization permissions. Those sources leave the applicability of ordinary subscription terms to general unattended use by this project unclear. Keep that route gated pending confirmation for the intended use. The Codex service terms also address third-party licenses for generated output. [Services Agreement](https://openai.com/policies/services-agreement/) · [Europe Terms of Use](https://openai.com/policies/eu-terms-of-use/) · [Codex service terms](https://openai.com/policies/service-terms/#4-codex-and-code-generation) · [Non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)

## Local Windows preflight on 2026-11-21

The installed VS Code-bundled executable reported `codex-cli 0.169.0-alpha.11.1`. A child process ran `codex -c 'cli_auth_credentials_store="file"' login status` with its own `CODEX_HOME` and inherited `OPENAI_*`, `CODEX_*` and `CHATGPT_*` variables removed before adding its own home. Alternating two freshly created empty directories (A, B, A) gave exit 1 and “not logged in” every time. Neither directory acquired an `auth.json`. This verifies the empty-directory preflight for this executable and store. It leaves authenticated isolation and precedence untested.

Local help advertised status, stdin API key, device authentication and stdin access token flows. It also advertised `exec --json`, `--ephemeral`, `--ignore-user-config`, sandbox modes and automatic approval options. The preflight made no model request and recorded no credentials.

## Method

- [x] Read the official authentication, CLI and terms pages and record the scope of their automation guidance.
- [x] Confirm the status command and document credential-presence semantics and the observed unauthenticated exit code.
- [ ] Capture a tiny read-only `codex exec --json` run using a confirmed ChatGPT sign-in and redact identifiers before committing fixtures.
- [ ] Capture the matching API-only run with a dedicated credential and verified spending controls. Record available cost and usage fields.
- [ ] Alternate authenticated directories and verify credential mode and precedence on each invocation.
- [ ] Test approval behavior, sandbox boundaries and network policy with the intended launch settings.
- [ ] Identify a plan-limit outcome from a versioned provider fixture or other verified evidence.
- [ ] Confirm which auth modes the adapter may offer and which modes may run unattended.

## Result

The documentation and logged-out status preflight are complete. API credentials are the candidate for unattended execution. Authenticated fixtures, switching, usage limits and sandbox checks are pending, so S8 remains open.
