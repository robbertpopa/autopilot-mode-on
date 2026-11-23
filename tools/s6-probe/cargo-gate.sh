#!/bin/bash
# Check-runner gate: cargo build and test in the sandbox with the network off.
export PATH="$HOME/node/bin:$HOME/.npm-global/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
if false; then
  curl -fsSL https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init -o /tmp/rustup-init && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain 1.99.1 2>&1 | tail -3
fi
rustc --version || exit 1
B=/mnt/d/autopilot-s6-gate; W=$B/crate; rm -rf "$W"; mkdir -p "$W/src"
cat > "$W/Cargo.toml" <<'EOF'
[package]
name = "sample"
version = "0.1.0"
edition = "2024"
[dependencies]
itoa = "1"
EOF
cat > "$W/src/lib.rs" <<'EOF'
pub fn show(n: u32) -> String { itoa::Buffer::new().format(n).to_owned() }
#[cfg(test)] mod t { #[test] fn it_shows() { assert_eq!(super::show(42), "42"); } }
EOF
cd "$W"
echo "--- fetch OUTSIDE the sandbox (host side, network allowed) ---"
time cargo fetch 2>&1 | tail -2
cat > "$B/cargo.json" <<EOF
{"network":{"allowedDomains":[],"deniedDomains":[]},
 "filesystem":{"denyRead":["$HOME","/mnt"],
  "allowRead":["$W","$HOME/.cargo","$HOME/.rustup","$HOME/.npm-global/lib/node_modules/@anthropic-ai/sandbox-runtime/vendor/seccomp"],
  "allowWrite":["$W","$HOME/s6-target"],"denyWrite":[]}}
EOF
mkdir -p "$HOME/s6-target"
mkdir -p "$HOME/s6-target/tmp"
run() { env -i PATH="$PATH" HOME="$HOME" CARGO_HOME="$HOME/.cargo" RUSTUP_HOME="$HOME/.rustup" TMPDIR="$HOME/s6-target/tmp" "$@"; }
echo "--- A: cargo test --offline, target on drvfs, writable TMPDIR ---"
rm -rf "$W/target"
( time run srt -s "$B/cargo.json" -c "export TMPDIR=$HOME/s6-target/tmp; cd $W && cargo test --offline 2>&1 | tail -9" ) 2>&1 | tail -16
echo "--- B: target on ext4 ---"
( time run CARGO_TARGET_DIR="$HOME/s6-target" srt -s "$B/cargo.json" -c "export TMPDIR=$HOME/s6-target/tmp; cd $W && cargo test --offline 2>&1 | tail -5" ) 2>&1 | tail -10
