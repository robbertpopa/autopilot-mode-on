#!/bin/bash
# Node 22 (the sandbox runtime needs Node 20 or newer), the sandbox runtime and
# Claude Code, all under the user's home. Run once as the normal user:
#   wsl -d Ubuntu -- bash tools/wsl/setup-user.sh
# No sign-in happens here. API-key mode passes its key per run, and nothing is
# stored in the distro.
set -eu
cd ~
if [ ! -x ~/node/bin/node ]; then
  version=$(curl -fsSL https://nodejs.org/dist/index.json | grep -o '"version":"v22[^"]*"' | head -1 | cut -d'"' -f4)
  curl -fsSL "https://nodejs.org/dist/$version/node-$version-linux-x64.tar.xz" -o node.tar.xz
  mkdir -p ~/node && tar -xJf node.tar.xz -C ~/node --strip-components=1 && rm node.tar.xz
fi
export PATH="$HOME/node/bin:$HOME/.npm-global/bin:/usr/local/bin:/usr/bin:/bin"
npm config set prefix ~/.npm-global
npm install -g @anthropic-ai/sandbox-runtime @anthropic-ai/claude-code
# The toolchain the check runner uses. The version follows rust-toolchain.toml.
if [ ! -x ~/.cargo/bin/rustup ]; then
  curl -fsSL https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init -o /tmp/rustup-init
  chmod +x /tmp/rustup-init
  /tmp/rustup-init -y --profile minimal --default-toolchain "${RUST_TOOLCHAIN:-1.99.1}"
fi
srt --version
claude --version
