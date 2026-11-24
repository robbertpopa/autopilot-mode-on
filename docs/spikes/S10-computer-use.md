<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# S10: computer use inside the boundary

**Question:** how can an agent operate a desktop without the harness giving up what the S6 boundary guarantees (no key, no host files, no open network, nothing irreversible without the owner)? **Gates:** stream 3 tasks C2 to C6. **Spending:** none. This is a reading spike: no API call was made, no product code was written. **Status:** document only; nothing past it is built until the owner answers decision 2 (see the end).
**Start prompt:** "Read `AGENTS.md`, `docs/spikes/S6-containment.md` and this file."

## What was read, and when

Read on 2026-11-24. Check again before building: the vendors change these pages.

- **Claude.** `platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool`. The first 100,000 of 115,758 characters were read; the tail (probably pricing and notes) was not.
- **OpenAI API.** `developers.openai.com/api/docs/guides/tools-computer-use` (the old `platform.openai.com` address redirects there).
- **Codex.** `learn.chatgpt.com/docs/computer-use` (the Codex documentation moved there; `developers.openai.com/codex` redirects to it). Only that page was read. The pages on Codex permissions, sandboxing and approvals (`/codex/permissions`, `/codex/sandboxing`, `/codex/agent-approvals-security`) were listed in the navigation and **not read**.

Pages were fetched through a summarising tool, not read raw. Every vendor fact below is a summary: re-read the page before relying on an exact field name.

## Claude: what the API does

- Computer use is a **client tool**: Claude returns actions, the caller performs them and sends back the result. Nothing runs on Anthropic's side.
- Current toolset: `computer_toolset_20260801`, declared as `{"type": "computer_toolset_20260801"}`, no beta header, accepted by the Claude 5 family (Fable, Mythos, Opus, Sonnet, Haiku). Older versions (`computer_20251124`, `computer_20250124`) need beta headers and only exist for older models. We use only the current one.
- Seventeen member actions: `screenshot`, `zoom`, `cursor_position`; `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `left_click_drag`, `mouse_move`, `left_mouse_down`, `left_mouse_up`, `scroll`; `type`, `key`, `hold_key`; `wait`. A turn can hold several `tool_use` blocks; run them in order, stop at the first failure and answer the rest with `is_error: true`. Every `tool_result` echoes `toolset_name`.
- The toolset does **not** downscale: the caller resizes screenshots (long edge at most 2,576 px for current models, about 4,784 visual tokens) and maps coordinates back. Suggested display: 1024x768 or 1280x720, never above 1920x1080.
- Cost: each screenshot is roughly 1,000 to 1,800 input tokens, so a long loop is expensive. The documentation advises pruning old screenshots in batches rather than every turn (pruning every turn defeats prompt caching and invalidates thinking blocks on the current models).
- Anthropic's own guidance: a dedicated VM or container with minimal privileges, no credentials near the model, an allowlist of domains, human confirmation for consequential actions, and a warning that instructions in screenshots and pages can steer the model. Anthropic runs classifiers over tool-returned screenshots; that is a second line, not ours.
- Reference implementation: `anthropics/claude-quickstarts`, `computer-use-demo`: Docker, Xvfb, Mutter, Tint2, Firefox. Its licence was not checked; we take the layout (Xvfb plus window manager plus tools), not the code.

## OpenAI API: what the documentation says

- A `computer` tool on the Responses API, also a client tool. Loop: send the task, execute the returned `computer_call` actions in order, return a `computer_call_output` with a screenshot, the matching `call_id` and `previous_response_id`, until no `computer_call` remains.
- Actions: `click`, `double_click`, `drag`, `move`, `scroll`, `keypress`, `type`, `wait`, `screenshot`.
- The page mentions pending API **safety checks** that stop the sample, and says an app needs an approval callback to handle them. It does not document field names (I had expected `acknowledged_safety_checks`; that is from memory, not from the page, so it is not to be relied on).
- Guidance: a fresh browser profile or an isolated desktop; an isolated browser or VM with an allowlist of sites and actions; treat on-screen text as untrusted. Examples cap loops at small step counts and use a 30 second timeout.
- Model access depends on the project. Model names on the page are not repeated here: they change.
- **Not read:** pricing, rate limits, and which models are enabled for the owner's key.

## Codex: what the documentation says

- The page read describes **Computer Use in the ChatGPT desktop app** (macOS and Windows, with ChatGPT Work and Codex), switched on under Settings. It operates the **real desktop**: the user's apps and browser, including signed-in pages. On Windows it takes over the active pointer and keyboard (the page itself says to use a VM or a second device). It asks before using each app, with an "always allow" choice; admins can restrict it.
- The page does **not** mention headless use, the CLI, `codex exec` or an API. So there is **no documented way for our process to drive Codex computer use** inside a display we control. For structured access to an app the page recommends a plugin or MCP server.
- So for Codex there is nothing to adopt yet. The only documented route to a Codex-driven desktop is OpenAI's own API tool above, with our own loop. C6 should be built on that route and only for the OpenAI API's computer tool, after the Claude driver works. If a Codex headless interface turns up, read it then.

## Design

### Where the loop runs

Our own process runs the loop. It calls the Messages API, holds the key, receives actions, validates them, executes them in the display, takes the screenshot and sends it back. The agent model never runs inside the display: it is a remote model that only sees screenshots. So the display holds **no key, no token, no model client**, and nothing in it needs the provider's network. This is a different shape from the coding agent (where Claude Code runs inside the boundary and the proxy supplies the key): here the harness is the trusted side and the display is a plain disposable machine.

No adapter makes HTTPS calls today (agents are launched as programs; the `update` crate shells out to the system `curl` through `process`). Options:

1. **`curl` through `process`**, as `update` does. No new dependency and no TLS code of ours. Cost: a subprocess per call, the request body goes through a file or stdin, the response is capped by the guardian's byte limit, and streaming is not possible. A computer-use turn is one request and one response of a few KB (plus a base64 screenshot up to some hundreds of KB), so this is workable. The key must reach `curl` as a header: pass it by a config file read from stdin (`curl -K -`), never on the command line, where `ps` shows it.
2. **A small HTTP client crate** (`ureq` with rustls is the candidate: blocking, small). Cost: a new dependency tree with TLS, which `cargo deny` must accept (the allow list in `deny.toml` is Apache-2.0, BSD, BSL, ISC, MIT, Unicode, Zlib; `ring` and some TLS crates need an exception entry). Gain: no subprocess, typed errors, an exact byte cap on the body as it is read. AGENTS.md says to prefer a well-known crate to hand-written security code; it also says no abstraction nobody asked for, and this is a new dependency, so it needs the owner's nod.

**Recommendation:** option 1 for C5 (the live run is one harmless task, and the code is the same pattern the repository already has and tests). Revisit option 2 only if a second feature needs HTTPS. Whether `ureq` or any HTTP client is already in `Cargo.lock` through egui or another crate was **not checked**: run `cargo tree -i ureq` and `cargo tree -i reqwest` before deciding.

### The display

A new role (C2), on the existing boundary, inside WSL on Windows and directly on a Linux host:

- A virtual X display: **Xvfb** on its own display number, a Unix socket under the role's private folder, **no TCP listening** (`-nolisten tcp`). A minimal window manager (Openbox or similar) and one or two programs: a browser with a throwaway profile, a text editor or terminal-less viewer. The reference demo uses Docker; we do not require Docker, because `srt` (bubblewrap) is what the probe already knows.
- Input by **`xdotool`** against that display only (the X socket is the only thing it can reach; the probe proves a second display, such as WSLg's, is unreachable). Screenshots by a tool that reads only that display (`import` from ImageMagick, `scrot` or `xwd`), written to a file in a private folder that the harness reads; the harness resizes to the size the API accepts.
- The display runs as a role: no host files except its own throwaway profile folder, no credentials, and **a network allowlist** (the same `srt` proxy: domains named by the task, empty by default; the first live task is a local page served from a file inside the display, with no network at all).
- WSLg and Windows interop stay unreachable as in S6 (the deny list already covers the interop socket). Wayland is not used.
- What leaves the display: the screenshot bytes (bounded) and the exit status. Nothing else.

Everything is disposable: a new display and profile per task, deleted at the end.

### What the model may and may not do

- It may take only the actions in the table of named capabilities (C3): screenshot, move, click, type, key, scroll, wait, with a per-action rate limit and size limit. The Claude toolset has 17 members: the harness enables the subset it implements (the documentation says unused members can be switched off with `configs`) and refuses the rest with an error result. `hold_key` and `wait` have upper bounds of their own that are lower than the 300 second API maximum.
- It may not leave the display, and it may not hold a key: the display has none. A screenshot is untrusted input: the harness never executes, opens or parses anything found in one beyond decoding the image, and treats text in it as data.
- **Irreversible actions need the owner** (C4): submit, send, delete, pay, sign, install, and typing into a field that looks like a password or card number. The harness decides this from the action and the visible window, not from what the model says; a model answer of "allow" cannot lift a forced prompt. A small model may add prompts, never remove one (tighten-only).
- Hard caps, independent of the model: action count, wall time, screenshot count and spend per task. Hitting a cap ends the task and says which one.
- An agent-written configuration cannot raise a cap or unlock an action; only the owner's own configuration can.
- Anthropic's classifiers and OpenAI's safety checks are extra signals, not part of our guarantee.

### What is logged

One line per action in the existing event log: sequence number, time, the action and its arguments, the model's short intent line (one sentence, untrusted text, stored as data), whether a prompt was forced and the owner's answer, and the screenshot's hash and size. Screenshots themselves are kept only for the task (they can show personal data) and deleted with the display unless the owner turns on a debug option. Never logged: the key, request headers, the full request body.

### The probe

Each property is a row with a positive control, as in S6: the display cannot read a host marker file; cannot reach the host's X server or WSLg; cannot reach an unlisted domain; `xdotool` input works inside the display and an input aimed at another display fails; a screenshot can be taken and writes only to its folder; no key or token in the environment or files; the process tree dies with the task. A mutation check weakens the role (open network, shared display socket) and the probe must fail with the row named. Rows are per platform and a missing row is stated, never skipped.

## Real desktop or disposable display (decision 2)

The documentation of all three vendors says the same thing: use a VM or container for anything you care about, and warns that a screenshot can carry instructions. The real-desktop route (Codex Computer Use today) means the agent acts with the owner's signed-in sessions, which is exactly the case where one injected sentence can pay or send. It also cannot be probed: there is no boundary to test, only the app's own approval prompts. Our harness can enforce forced approvals on a disposable display, where it sees every action; on a real desktop driven by someone else's app it could not.

**Recommendation:** disposable display only (a display inside the same WSL or Linux boundary). The real desktop is out of scope until a boundary for it can be probed.

## Questions for the owner

1. **Decision 2 (blocks C2 onward):** disposable display only, as recommended, or also the real desktop? (A yes to the real desktop needs its own spike: nothing here covers it.)
2. **HTTP client:** may C5 shell out to the system `curl` through `process` (recommended), or should the project take on an HTTP and TLS crate?
3. **First task:** is "open a local page inside the display and read a value" the right first live task, within the $1 total spike budget? A rough guess is a few cents with a small model and about ten screenshots at 1,000 to 1,800 tokens each, but pricing was not read and no call was made.
4. **Which models** should the live run use (Haiku-class to stay cheap, or the model that will be used in production)?
5. **Codex and OpenAI:** is it enough to schedule C6 on the OpenAI API tool, given that Codex's computer use has no documented headless interface? Does the owner have API access to a computer-use model on a capped key?

## What was not done

No API call, no display was built, no probe row was written, no dependency was added, and the S6 probe was not run for this document. The Codex permissions and sandboxing pages and OpenAI's pricing were not read. The tail of the Claude page was not read.
