#!/usr/bin/env bash
# Build Autopilot Mode On.app and a .dmg from a release binary.
#
#   build-dmg.sh <version> <path to autopilot-ui> <path to icon-512.png> <output.dmg> [helper binaries...]
#
# Helper binaries (the process guardian and the practice agent) go next to the
# window's own binary, where it looks for them.
#
# Signing: without secrets the app is signed ad hoc, which runs after the
# owner approves it once in System Settings. With APPLE_SIGNING_IDENTITY set
# (a Developer ID Application certificate already in the keychain) it is
# signed for distribution, and with APPLE_ID, APPLE_TEAM_ID and
# APPLE_APP_PASSWORD as well it is notarized and stapled.
set -euo pipefail

version="$1"
binary="$2"
icon="$3"
out="$4"
shift 4

work="$(mktemp -d)"
app="$work/Autopilot Mode On.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/autopilot-ui"
chmod +x "$app/Contents/MacOS/autopilot-ui"
for helper in "$@"; do
  cp "$helper" "$app/Contents/MacOS/"
  chmod +x "$app/Contents/MacOS/$(basename "$helper")"
done

# The icon: every size macOS asks for, made from the 512 px picture.
iconset="$work/Autopilot Mode On.iconset"
mkdir "$iconset"
for size in 16 32 64 128 256 512; do
  sips -z "$size" "$size" "$icon" --out "$iconset/icon_${size}x${size}.png" >/dev/null
done
for size in 16 32 128 256; do
  double=$((size * 2))
  cp "$iconset/icon_${double}x${double}.png" "$iconset/icon_${size}x${size}@2x.png"
done
cp "$icon" "$iconset/icon_512x512@2x.png"
rm "$iconset/icon_64x64.png"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Autopilot Mode On.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Autopilot Mode On</string>
  <key>CFBundleDisplayName</key><string>Autopilot Mode On</string>
  <key>CFBundleIdentifier</key><string>io.github.robbertpopa.pipeline</string>
  <key>CFBundleExecutable</key><string>autopilot-ui</string>
  <key>CFBundleIconFile</key><string>Autopilot Mode On</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Each helper is signed before the bundle that holds it.
for binary_in_app in "$app"/Contents/MacOS/*; do
  [ "$binary_in_app" = "$app/Contents/MacOS/autopilot-ui" ] && continue
  if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$binary_in_app"
  else
    codesign --force --sign - "$binary_in_app"
  fi
done
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$app"
else
  codesign --force --sign - "$app"
fi

stage="$work/dmg"
mkdir "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Autopilot Mode On" -srcfolder "$stage" -ov -format UDZO "$out"

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ] && [ -n "${APPLE_ID:-}" ]; then
  codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$out"
  xcrun notarytool submit "$out" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_PASSWORD" --wait
  xcrun stapler staple "$out"
fi

rm -rf "$work"
echo "built $out"
