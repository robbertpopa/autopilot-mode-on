#!/bin/bash
# Packages the sandbox needs. Run once, as root, inside the distro:
#   wsl -d Ubuntu -u root -- bash tools/wsl/setup-root.sh
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq bubblewrap socat ripgrep python3 curl ca-certificates build-essential git xz-utils
