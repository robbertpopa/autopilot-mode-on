#!/bin/bash
# Build the deny hook from this checkout and install it where the agent can run
# it but not change it (a folder outside every writable root). Run after
# setup-user.sh, and again whenever crates/policy changes:
#   wsl -d Ubuntu -- bash tools/wsl/install-hook.sh
# The sandbox probe runs the installed hook on a `git push` and a `git status`
# payload and refuses to start Build if it answers wrongly, so a stale build
# does not pass quietly.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
# The target folder is kept off the Windows drive: it is large and slow there.
export CARGO_TARGET_DIR="$HOME/.cache/autopilot-hook-target"
cd "$here"
cargo build --release -p autopilot-policy --bin autopilot-hook
install -d -m 755 "$HOME/.autopilot/bin"
install -m 755 "$CARGO_TARGET_DIR/release/autopilot-hook" "$HOME/.autopilot/bin/autopilot-hook"
echo "installed $HOME/.autopilot/bin/autopilot-hook"
