<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

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

## Local checks on 2026-11-25 (stream 2, no model call, $0)

Same executable (`codex-cli 0.169.0-alpha.11.1`, VS Code bundle), Windows. No login flow was started, no credential was read or written, and no OpenAI API key was present in the environment (no `OPENAI_*`, `CODEX_*` or `CHATGPT_*` variable was set).

- **Logged in.** `codex login status` with the owner's default home printed `Logged in using ChatGPT` and exited 0. It says which mode the stored login is in, not whether a request would pass.
- **Logged out.** With a fresh empty `CODEX_HOME` and `cli_auth_credentials_store="file"`, it printed `Not logged in` and exited 1. No `auth.json` appeared. The home did fill with `.sandbox`, `skills`, `installation_id` and several `*.sqlite` files, so a launch must give each run its own throw-away home and delete it.
- **`codex exec --json --ephemeral -s read-only` logged out** (empty home, tiny prompt). It started a thread, then reconnected five times against `api.openai.com/v1/responses` (a WebSocket first, then HTTPS), each answered `401 Unauthorized: Missing bearer or basic authentication in header`. Observed JSONL: `{"type":"thread.started","thread_id":...}`, `{"type":"turn.started"}`, then `{"type":"error","message":"Reconnecting... N/5 (unexpected status 401 ...)"}` lines. The text log lines (`ERROR codex_api::...`) were mixed into the same pipe, so a parser must ignore any line that is not a JSON object. The run took several seconds before giving up, so an adapter needs its own prompt "not signed in" check from `login status` rather than waiting for the retries. I did not capture the final exit code (the output was piped), so that is open.
- **Not run, on purpose.** An authenticated `exec` on the owner's ChatGPT login was not run: S8's terms assessment leaves unattended use of a plan unclear, so that route stays gated, and running it would use the owner's account without their say-so. An API-key run was not possible: there is no dedicated OpenAI key and no verified spend limit.

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

The documentation, the logged-in and logged-out status checks and the logged-out `exec --json` shape are complete (2026-11-25). The authenticated event shapes, cost and usage fields, mode switching, sandbox and network behaviour and the plan-limit outcome are not measured, because no dedicated API key exists and the ChatGPT route is gated. Decision for the adapter: **no-go for now** (see [DESIGN section 7a](../DESIGN.md)). What the owner must do to unblock it is listed there.

Earlier result: the documentation and logged-out status preflight are complete. API credentials are the candidate for unattended execution. Authenticated fixtures, switching, usage limits and sandbox checks are pending, so S8 remains open.
