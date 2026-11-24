#!/bin/bash
# The sandbox runtime and the deny hook for a Mac, both under ~/.autopilot, and
# nothing else on the machine changed. Node 20 or newer and Claude Code must
# already be installed. Run from the repository root:
#   tools/macos/setup.sh
# No sign-in happens here. API-key mode passes its key per run.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$HOME/.autopilot/npm" "$HOME/.autopilot/bin"
# Pinned to the version the probe was validated with (`VALIDATED_SRT`).
npm install -g --prefix "$HOME/.autopilot/npm" @anthropic-ai/sandbox-runtime@0.0.83
cd "$here"
cargo build --release -p autopilot-policy --bin autopilot-hook
install -m 755 "$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/release/autopilot-hook" "$HOME/.autopilot/bin/autopilot-hook"
"$HOME/.autopilot/npm/bin/srt" --version
echo "installed the sandbox runtime and the hook under $HOME/.autopilot"
