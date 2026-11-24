<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Owner prompts and handoff (written 2026-11-25)

The three prompts the owner wants to run, one after another and in parallel, through the Autopilot window itself, and the state of play for whoever continues on a new machine. The acceptance scenarios that go with them are in [owner-scenarios.md](owner-scenarios.md). Rules: [AGENTS.md](../../AGENTS.md); the way the work was split: [ORCHESTRATOR.md](ORCHESTRATOR.md).

## The three prompts (verbatim)

1.

> Let's remove the top bar of the autopilot app, make it similar to claude top bar. Next, let's add a way to create a pipeline or remove it, i dont see any yet. Also, every pipeline has the same steps, but they are disabled, I think we should have specific steps for each pipelines, and maybe have a way to add steps that can be reused, that can be added. Also, add an easy way to move the pipelines by dragging. And, I see "design" step, ok, but what design means? Does it uses claude design? Does it offer an artifact, what does it do? I should be able to view the prompt or the agent details and also edit them, manual or using the AI there. Rules.md should have an option to be verified there by an AI and make the rules being more useful for other AIs, maybe a haiku/sonnet should do the job. There area a lot of things that are not centered / well positioned, check the app for those.

2.

> For every pipeline step I want to see a detailed plan for the steps, similar to what there is in claude code. Each step should show what it will do, in order, as a short checklist that ticks off as the step runs, and the plan should be visible before the step starts and stay visible after it finishes.

3.

> Add the possibility to archive, delete and restore a chat (ticket), and also to stop a conversation and resume it, from the ticket page and from the sidebar list. Archived tickets are hidden from the main list but can be restored; delete asks for confirmation; stop ends the current run safely and resume continues from the last finished step.

Owner request added during testing: a running step must show, in real time, what it is doing (thinking, editing, testing) with a heartbeat every few seconds, so a stalled run can be told from a working one (task W11, merged as PR 170).

## How to run them on a Mac

1. Install the tools once: `tools/macos/setup.sh` (pinned `srt`, Claude Code). Sign in to Claude (`claude auth login`) and put a dedicated, spend-limited API key where the app can read it as the environment variable `AUTOPILOT_ANTHROPIC_API_KEY`. Never store the key in the repository. Rotate any key that was ever printed.
2. Build and install: `cargo build --release --workspace`, then `packaging/macos/build-dmg.sh 0.1.0 target/release/autopilot-ui assets/icon-512.png out.dmg target/release/autopilot-process-guardian target/release/autopilot-fake-agent target/release/autopilot`, open the dmg and drag the app to Applications. The app is signed ad hoc: approve it once in System Settings, Privacy and Security.
3. A profile for the run (put it next to the data folder; no secrets in it):

   ```toml
   [steps.understand]
   auth = "subscription"
   model = "sonnet"

   [steps.clarify]
   auth = "subscription"
   model = "sonnet"

   [steps.design]
   auth = "subscription"
   model = "sonnet"

   [steps.plan]
   auth = "subscription"
   model = "opus"

   [steps.build_check]
   auth = "api"
   model = "haiku"

   [steps.build_check.limits]
   budget_microusd = 250000

   [checks.build]
   executable = "cargo"
   args = ["check", "--offline", "--workspace"]
   ```

   Build stays on the API key because a real Build only runs in API mode inside the sandbox. Do not use a `git` command as a check: git cannot read the home directory configuration inside the check role.
4. Start the window with the key in its environment and a clean data folder, from a terminal, because an app opened from the Dock does not see the variable:

   ```bash
   mkdir -p ~/autopilot-app/data && cd ~/autopilot-app
   AUTOPILOT_ANTHROPIC_API_KEY="$(head -n1 /path/to/key.txt | tr -d '\r\n')" \
     "/Applications/Autopilot Mode On.app/Contents/MacOS/autopilot-ui" \
     --data-dir ~/autopilot-app/data --profile ~/autopilot-app/profile.toml
   ```

5. Go through the welcome screens, choose the project folder (a clone of this repository), then create the three tickets from the prompts above. Run one ticket's Build at a time until the per-ticket worktree and approval fixes are confirmed live, then try all three in parallel. Approve Design (forced), then press "Make the plan", approve the plan, then Build.

## Where it stands

- Merged into `develop` through 2026-11-25: the four work streams' first waves, macOS support (srt and Seatbelt, 21-row probe, real Build in the sandbox), the macOS sign-in fix (`USER` in the environment allowlist), parallel tickets in one project, request ids that stay unique, live progress inside a running step, step plans as a checklist, the pipelines page (create, rename, delete, reorder, own steps per pipeline, step details with AI edit, rules.md improvement with AI), the slim title row.
- Open when this was written: archive, restore, delete, stop and resume (PR 179); the black strip beside the sidebar and the clipped Inbox card in the installed build; the status pill that overprints the step label on the ticket page; per-call budget defaults and a "raise the cap and retry" action; git checks in the sandbox; what the Design source options really do.
- Proven end to end: one small ticket went from Understand to a real draft pull request on the throwaway repository (`robbertpopa/autopilot-pr-test`, PR 1) through the command line with the real model in the macOS sandbox. None of the three prompts above has gone from zero to a merged result through the window yet: all three reached Plan or Build; Build produced code that did not compile on the first rounds and stopped for the owner.
- Live spend so far is above the $1 in the guide because the owner raised the per-call limit; about $2.1 on the API key. Set a new total before the next run.
- Owner decisions still open: computer-use scope (2), more agents (3), subscription mode in the sandbox (4).
