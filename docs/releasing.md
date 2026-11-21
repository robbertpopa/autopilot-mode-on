# Releasing Autopilot Mode On

Autopilot Mode On ships as a Windows installer and two macOS disk images, offered on a small website and updated from inside the app.

## How a release happens

1. Commits to `master` use `type: subject` titles. `release-please.yml` keeps a release pull request open from them.
2. Merging that pull request tags `vX.Y.Z` and publishes a GitHub release (it also bumps `Cargo.toml` and the changelog).
3. Publishing the release starts `release.yml`:
   - `windows`: builds `autopilot-ui.exe`, then `packaging/windows/installer.nsi` makes `autopilot-mode-on-setup-x64.exe`. The installer is per-user, needs no administrator rights, and upgrades in place.
   - `macos`: builds on Apple silicon and Intel runners, then `packaging/macos/build-dmg.sh` makes `Autopilot Mode On.app` and `autopilot-mode-on-macos-arm64.dmg` / `autopilot-mode-on-macos-x64.dmg`.
   - `publish`: `packaging/manifest.sh` writes `latest.json` (version, notes, URL and SHA-256 per platform) and signs it into `latest.json.sig`. Everything gets a build attestation and is attached to the release.
4. `pages.yml` publishes `site/` when it changes on `master`. The site links to `releases/latest/download/<name>`, and the names never change.

## One-time setup (repository owner)

| What | Where | Why |
|---|---|---|
| `UPDATE_SIGNING_KEY` secret | Settings > Secrets > Actions | ed25519 private key (PEM) that signs `latest.json`. Its public half is `PUBLIC_KEY_HEX` in `crates/update/src/lib.rs`. A release without it fails on purpose. |
| `RELEASE_PLEASE_TOKEN` secret | same | Token with contents and pull-requests write. The default token cannot be used: releases it creates do not start other workflows. |
| Pages source = "GitHub Actions" | Settings > Pages | Needed once for `pages.yml`. |
| Apple secrets (optional) | same | `APPLE_SIGNING_IDENTITY`, `APPLE_CERTIFICATE` (base64 .p12), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD`. Without them the app is signed ad hoc and macOS asks for a right-click, Open on first launch. |

To make a new signing key, run `openssl genpkey -algorithm ed25519 -out key.pem`, then put the 32 raw public bytes in `PUBLIC_KEY_HEX` with `openssl pkey -in key.pem -pubout -outform DER | tail -c 32 | xxd -p -c 64`. Releases already shipped keep trusting the old key, so rotating means shipping one release signed with the old key that contains the new one.

## How the app updates itself

`crates/update` is the whole mechanism, and `autopilot-ui` calls it:

- At start, and every six hours while open, the app fetches `latest.json` and `latest.json.sig` from the latest release through the system `curl` (HTTPS only).
- The signature is checked with the built-in public key before the manifest is read. A manifest that does not verify is dropped.
- Only a strictly newer plain `x.y.z` version is offered. The asset URL must be under this repository's `releases/download/`.
- "Update to X" downloads the installer, checks its SHA-256 against the signed manifest, and only then opens it (the installer on Windows, the disk image on macOS) and closes the window. A file that fails the check is never written.
- Set `AUTOPILOT_NO_UPDATE_CHECK=1` to turn the check off.

The update is one click, not silent: the owner sees what is installed and when.

## What is not covered yet

- Linux packages (the window builds and passes CI on Linux, but no installer is shipped).
- Delta updates and silent background install.
- The macOS build, signing and notarization and the Windows installer are only exercised by `release.yml`, on the first release.
