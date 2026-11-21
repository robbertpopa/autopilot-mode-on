#!/usr/bin/env bash
# Write latest.json for a release and sign it.
#
#   manifest.sh <tag> <directory holding the installers> <notes file>
#
# The signing key is read from the UPDATE_SIGNING_KEY environment variable
# (an ed25519 private key in PEM form). The signature is raw ed25519 over the
# exact bytes of latest.json, written as hex, which is what autopilot-update
# checks against the public key built into the program.
set -euo pipefail

tag="$1"
dir="$2"
notes_file="$3"
version="${tag#v}"
base="https://github.com/${GITHUB_REPOSITORY:-robbertpopa/autopilot-mode-on}/releases/download/${tag}"

entry() {
  local key="$1" file="$2"
  [ -f "$dir/$file" ] || return 0
  local sum
  sum="$(openssl dgst -sha256 -r "$dir/$file" | cut -d' ' -f1)"
  printf '"%s":{"url":"%s/%s","sha256":"%s"}' "$key" "$base" "$file" "$sum"
}

assets=""
for pair in "windows-x64:autopilot-mode-on-setup-x64.exe" \
            "macos-arm64:autopilot-mode-on-macos-arm64.dmg" \
            "macos-x64:autopilot-mode-on-macos-x64.dmg"; do
  item="$(entry "${pair%%:*}" "${pair#*:}")"
  [ -n "$item" ] && assets="${assets:+$assets,}$item"
done
[ -n "$assets" ] || { echo "no installers found in $dir" >&2; exit 1; }

# The notes become a tooltip, so keep them short and strip anything JSON would need escaped.
notes="$(tr -d '"\\\r' < "$notes_file" | tr '\n' ' ' | cut -c1-300)"
printf '{"version":"%s","notes":"%s","assets":{%s}}' "$version" "$notes" "$assets" > "$dir/latest.json"

key_file="$(mktemp)"
trap 'rm -f "$key_file"' EXIT
printf '%s\n' "$UPDATE_SIGNING_KEY" > "$key_file"
openssl pkeyutl -sign -rawin -inkey "$key_file" -in "$dir/latest.json" -out "$dir/latest.json.bin"
xxd -p -c 256 "$dir/latest.json.bin" | tr -d '\n' > "$dir/latest.json.sig"
rm "$dir/latest.json.bin"
echo "wrote latest.json for $version"
