<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# S11: can a request filter close the subscription-mode exfiltration path?

**Question (task S-e, owner decision 4):** subscription mode puts a sign-in inside the sandbox. A token the agent can read can be sent to the provider host under an attacker's own key and read back from their console. Can a small Node launcher that uses `srt`'s request filter reject every provider request that does not carry the placeholder? **Spending:** none. **Status:** a reading spike. No product code was written, nothing was run, nothing ships.

## What was read

The `@anthropic-ai/sandbox-runtime` 0.0.83 package (the version we pin as `VALIDATED_SRT`), downloaded from the npm registry into an empty folder and read there without running anything: `README.md`, `dist/index.d.ts`, `dist/sandbox/request-filter.d.ts`, `dist/sandbox/decider-client.d.ts`, `dist/sandbox/sandbox-config.d.ts`, `dist/cli.js` and `dist/proxy-cli.js`. Only type declarations and the README were read closely; the JavaScript behind them was not audited. Read on 2026-11-25.

## What the filter is

- `network.filterRequest` is a library option, **not** a JSON setting: `sandbox-config.d.ts` types it as a function (`FilterRequestCallback`), and the README says it is "set by library consumers". The `srt` command line has no way to give it one. This is why our JSON settings cannot do it.
- It is called once per HTTP request the proxy parses, on plain HTTP and, when `network.tlsTerminate` is set, on terminated HTTPS. We already use TLS termination for the API key.
- It receives a web-standard `Request` (method, URL, all headers, a lazy body) and an info object with the scheme, the TLS server name, the canonical target host and port, the request target as spelled and normalised, and the raw header lines.
- It returns `allow` or `deny`. A deny can carry a status and a reason that the client sees. An allow can **remove headers and set headers** on the forwarded request.
- **It fails closed.** A throw, a rejection or a malformed decision denies the request.
- Related switches that matter here: `refuseOpaqueTunnels` (refuse every tunnel the filter cannot see into), `requireHostMatch` (421 if the Host header, TLS name and target disagree), and `ambiguousRequestTarget` (default `refuse`).

## The answer

**Yes, in principle, and there is a stronger option than rejecting.** All of this is read from the declarations and the README, none of it exercised.

1. **Reject.** The callback sees the host and every header after TLS termination. For a request to the provider host it can deny unless the credential header holds exactly the placeholder. That is what the question asked, and the declarations support it.
2. **Overwrite (better).** The same callback may, on allow, remove every credential header the agent sent and set the real one itself. Then the agent's own key never reaches the provider whatever it does, and there is no comparison to get wrong. The header-name matching folds case and treats `-`, `_` and `.` as one character, so `x-api-key`, `X_Api_Key` and `x.api.key` are one header. This is the design to prefer.
3. **Not an option here:** `srt proxy` (a standalone proxy that asks an external decider process for every request, with a framed JSON protocol and its own `cred` and `credential` verdict fields). It is built for a host that runs the workload elsewhere and does not wrap a sandboxed child, so it does not replace the sandbox. It is also not exported from the library index.
4. The same hole exists in **API-key mode today**: the agent holds only a placeholder, but nothing stops it from sending a request to `api.anthropic.com` with a key of its own. [S6](S6-containment.md) records this ("the provider host is still a way out"). A filter that overwrites the credential on provider requests would close it there too. This is a finding for the owner beyond decision 4.

## What a launcher costs us

- **A new program inside the validated surface.** Today Rust writes a settings file and runs the `srt` command (`crates/process/src/wsl.rs`). A filter needs a Node script that calls `SandboxManager.initialize` with the callback, then `wrapWithSandbox`, then starts the command. That script becomes part of what the probe must validate: the probe has to run through the launcher, not through `srt`, or it proves nothing about the real launch. The pin (`VALIDATED_SRT`) would extend to the launcher's own version, and every probe row, control and mutation check is re-run against it.
- **The launcher is security code we would write.** AGENTS.md prefers a well-known crate to hand-written security code. The callback itself is short (host check, header removal, header set), but it must be tested with hostile spellings of the headers and hosts.
- **Where it runs.** WSL2 and Linux hosts use the same library. macOS also, but see the next point. Native Windows is not covered by anything here.
- **macOS.** The README says credential *files* are masked on Linux only; on macOS a masked file is **denied** instead. A subscription sign-in is a file. Inside the sandbox on a Mac the agent could not read it, so Claude Code would have to start from something else (an environment variable the launcher sets to a placeholder). Whether Claude Code accepts that was **not tested**.

## What was not found out

- **Which hosts the subscription sign-in and token refresh need.** Not read from any source. The filter must cover every one of them, and the domain list must contain exactly those. Candidates to check against the installed Claude Code (not verified): the API host, and the host that issues and refreshes OAuth tokens. A refresh request carries the refresh token in its **body**, which the callback can read but which is a different shape from a header: the design must say what happens to it.
- Whether Claude Code accepts a placeholder in place of a sign-in, and what it does when the token is refreshed behind its back by the host.
- Whether the filter holds against a client that does not use a header (a token in a query string or body). The provider's API takes the key in a header, so the deny rule is "any request to the provider host without our overwritten header is refused", which does not depend on where the agent put its own.
- Terms of use: whether running a subscription sign-in through an automated launcher is allowed is for the owner and the provider, not for this document.

## Recommendation for decision 4

- **Keep subscription mode refused for now.** The filter makes closing the path plausible, but the pieces that would make it safe (the exact set of sign-in hosts, a placeholder Claude Code accepts, a probe that runs through the launcher, the macOS file case) are unproven, and a wrong guess puts a real sign-in inside a sandbox.
- **If the owner wants it,** the next step is a spike with a small Node launcher and no real sign-in: check that Claude Code starts on a placeholder, record the hosts it calls, and show with a control that a request carrying another key is replaced or refused. Budget: $0 for the filter tests; a live call only within the S6 spike budget.
- **Separately, consider the same filter for API-key mode** (point 4 above): the same launcher and the same probe work, and it removes the "provider host is a way out" caveat from the S6 document.

## Launcher spike, 2026-11-25 (fake host, fake credentials, no spend)

Asked for by the owner. Code: [`tools/s11-launcher`](../../tools/s11-launcher) (`filter.mjs` the request filter, `launcher.mjs` the `SandboxManager` wiring, tests under `test/`). It is a spike: nothing in the engine, an adapter or `process` loads it, and subscription mode stays refused. `@anthropic-ai/sandbox-runtime` is pinned to exactly 0.0.83. CI does not run these tests (no Node job); they were run by hand.

**What ran.** WSL2 Ubuntu 24.04, Node 22.22.0, the `srt` library 0.0.83 through the public `SandboxManager.initialize` and `wrapWithSandbox` (the same bubblewrap, proxy and TLS termination as a real launch), a fake HTTPS provider on loopback with a throwaway CA, a fake token in the environment and a fake `.credentials.json`. The client inside the sandbox was `curl`. No real sign-in, no real Claude install, no provider call, $0. Not run: native Windows (`srt` needs its service account there), macOS, any Claude Code process.

Results: `test/filter.test.mjs` 32 unit rows (run on this PC's Node, all pass); `test/integration.test.mjs` 17 rows and `test/tunnel.test.mjs` 2 rows in WSL (all pass). Every row is judged on what the fake provider received, not only on the client's status.

### What the filter sees (the question that decided the design)

| | Filter sees | Fake provider receives |
|---|---|---|
| `Authorization` header | `Bearer fake_value_<uuid>…` (the placeholder) | `Bearer <the real fake token>` |
| Body containing the placeholder | the placeholder | the real token |
| TLS name | `localhost` (in `info.sni` and `info.target.host`) | |

The filter runs **before** srt's substitution, on headers and on the body, and it can read the body. It can compare against the placeholder, and it can see a placeholder sent where it must not go.

### The hole a header check alone leaves

srt also substitutes the placeholder in request **bodies** on the provider host. Mutation, run: with a filter that allows everything, a request with a foreign `x-api-key` and a body holding the placeholder reaches the provider with the **real token in the body** under the foreign key. That is the exfiltration path, and it also lets the token be sent as prompt text for the model to repeat. So the filter does three things: exactly one credential header holding exactly the placeholder; the placeholder nowhere else (other headers, request target, body scanned whole up to 32 MiB, larger refused); host, scheme and target checks.

### Rows

| Row | Result | Control or mutation |
|---|---|---|
| Placeholder in `Authorization` | allowed, provider sees the real token | the control for every refusal below |
| Foreign `x-api-key`, no placeholder | 403 from the filter, nothing arrives | allow-all filter: the foreign key arrives |
| Placeholder plus a foreign key in a second header | 403 `more than one credential header` | |
| Valid header, placeholder in the body | 403 `placeholder in the body` | allow-all filter: the real token arrives in the body under a foreign key |
| `Host` header unlike the target | 421 from srt (`requireHostMatch`) | |
| `/v1/x/../messages` (dot segment) | 400 from srt (`ambiguousRequestTarget: refuse`) | |
| Host not on the domain list | CONNECT refused 403 by the domain rule; the filter was never asked | shows which layer refused |
| Plain `http://` to the provider host | 403 `provider requests must be https` | |
| Filter throws | 403 `sandbox policy check failed`, nothing arrives | fails closed |
| Opaque tunnel (host allowed, not terminated) | refused with `refuseOpaqueTunnels` | switch off: the foreign key lands at the provider |
| Overwrite mode, three spellings of a foreign key | allowed, provider gets only the launcher's header | |
| Fake credential file masked with `extract` | still valid JSON, access and refresh values are placeholders, rest kept, real file unchanged | |
| Refresh request (placeholder in body) | refused by the filter | allow-all: the real refresh token goes out **and the new real access token comes back to the agent in the response** |
| Unit filter with the credential compare disabled | 6 of 32 unit rows fail; restored, 32 pass | |

Unit rows also cover `X_Api_Key` and `x.api.key` spellings, duplicate headers, text after the placeholder, a placeholder split across body chunks, a percent-encoded placeholder in the query, an oversized body, missing request info, and a filter given no placeholders (refuses everything).

### Hosts, from documentation only

[Claude Code network configuration](https://code.claude.com/docs/en/network-config), read 2026-11-25: `api.anthropic.com` (API requests), `claude.ai` (account authentication), `claude.com` (sign-in page that redirects to claude.ai), `platform.claude.com` ("OAuth token exchange, refresh, and revocation also go to this host for claude.ai accounts"), `sso.claude.com` (SSO sign-in only). Refresh is **not** on the API host. [Authentication](https://code.claude.com/docs/en/authentication): `claude setup-token` makes a one-year OAuth token for `CLAUDE_CODE_OAUTH_TOKEN`; the Linux login is `~/.claude/.credentials.json` (mode 0600), macOS uses the Keychain. The pages do **not** say which HTTP header `CLAUDE_CODE_OAUTH_TOKEN` travels in, do not document the credential file's schema, give no refresh-token lifetime and say nothing about terms for launcher use. The probe's fake file format is invented and is not claimed to match the real one.

### Answers to what S11 left open

- **Hosts:** above. The sandbox needs only `api.anthropic.com`. Interactive sign-in happens on the host, so `claude.ai`, `claude.com`, `sso.claude.com` and `platform.claude.com` need not be reachable from inside.
- **Refresh must stay outside the sandbox.** The filter can refuse a refresh body, but if refresh were allowed, srt masks only what the agent sends: the response with new real tokens reaches the agent unmasked (mutation row). So: refresh on the host, and give the sandbox a token that needs no refresh (the documented one-year `setup-token` value as a masked environment variable) or one the launcher renews between runs. Keep `platform.claude.com` off the domain list.
- **Environment variable, not file:** masking a variable works on Linux and macOS, while a masked file is masked only on Linux (denied on macOS). Prefer the variable.
- **Reject or overwrite:** both work. Overwrite (remove every credential header the agent sent, set the launcher's own) leaves no comparison to get wrong, but the body and target rule is still needed because srt substitutes in bodies.
- **API-key mode has the same hole today**, and the same filter, with the key as the masked variable, closes it.

### Not found out

- Whether Claude Code starts on a placeholder, which header it uses for the OAuth variable, and which hosts it calls at start. That needs a Claude Code process; this spike did not run one (rule: no real Claude). A fresh copy with no sign-in against the fake host would answer it, with the owner's go-ahead.
- macOS and native Windows behaviour; the other headers a real request carries (the credential-header list may need more names).
- Terms of use for a subscription sign-in under a launcher: not stated in the pages read; the owner's question.

### Recommendation for the owner

1. **Keep subscription mode refused.** The filter works on Linux/WSL2 and closes the provider-host path, but a real sign-in inside a sandbox also needs a placeholder Claude Code accepts (untested) and a terms answer only you can give.
2. **Use the launcher for API-key mode first.** It removes the "provider host is a way out" caveat in S6 with no new secret. That means a Node launcher in the validated surface (pinned `srt`, probe rows run through it, a Node job in CI) in place of the `srt` command in `crates/process/src/wsl.rs`: a later stream 3 task, not done here.
3. **If you want subscription mode after that:** the `setup-token` environment variable, never the credential file, never refresh inside, and let me run one fresh Claude Code copy (no sign-in) against the fake host to settle the header and hosts.
