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
