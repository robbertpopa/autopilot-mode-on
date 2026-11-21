//! Finding and fetching a newer release of the window.
//!
//! A release publishes `latest.json` and `latest.json.sig`. The signature is
//! checked against the public key built into this program before one byte of
//! the manifest is believed, and the installer's SHA-256 from the manifest is
//! checked before it is ever opened. Anything that fails a check is an error,
//! never a guess. The network is reached through the system `curl`, which both
//! Windows 10+ and macOS ship, so no TLS stack is linked into the program.

use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

use autopilot_process::{CommandSpec, Limits, Outcome, Runner};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The public half of the release signing key. The private half is the
/// `UPDATE_SIGNING_KEY` secret of the release workflow.
pub const PUBLIC_KEY_HEX: &str = "0b3858bdc8e28a848edfea4290de5d71838c993a1dc9377c08df92bcd02fc5f7";

/// Where releases live. Downloads are only accepted from here.
pub const RELEASES: &str = "https://github.com/robbertpopa/autopilot-mode-on/releases";

const MANIFEST_LIMIT: usize = 64 * 1024;
const INSTALLER_LIMIT: usize = 400 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    pub assets: BTreeMap<String, Asset>,
}

/// A newer release this computer can install.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Update {
    pub version: String,
    pub notes: String,
    pub asset: Asset,
}

/// `windows-x64`, `macos-arm64` or `macos-x64`: the keys used in `assets`.
#[must_use]
pub fn platform() -> Option<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x64"),
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        _ => None,
    }
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

/// Check `signature_hex` over `manifest` with `key_hex`, then read the manifest.
///
/// # Errors
/// Returns a sentence if the key or signature is malformed, the signature
/// does not match, or the manifest is not the expected JSON.
pub fn verify(manifest: &[u8], signature_hex: &str, key_hex: &str) -> Result<Manifest, String> {
    let key: [u8; 32] = hex(key_hex.trim())
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("the update key is malformed")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "the update key is not valid")?;
    let signature: [u8; 64] = hex(signature_hex.trim())
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("the update signature is malformed")?;
    key.verify(manifest, &Signature::from_bytes(&signature))
        .map_err(|_| "the update signature does not match; the release was not trusted")?;
    serde_json::from_slice(manifest)
        .map_err(|error| format!("the update manifest is unreadable: {error}"))
}

/// Plain `major.minor.patch`. A pre-release or anything else is not a version
/// this program will move to.
fn parts(version: &str) -> Option<[u64; 3]> {
    let mut numbers = version.trim_start_matches('v').split('.');
    let found = [
        numbers.next()?.parse().ok()?,
        numbers.next()?.parse().ok()?,
        numbers.next()?.parse().ok()?,
    ];
    numbers.next().is_none().then_some(found)
}

/// Whether `candidate` is a strictly newer plain version than `current`.
#[must_use]
pub fn is_newer(current: &str, candidate: &str) -> bool {
    match (parts(current), parts(candidate)) {
        (Some(current), Some(candidate)) => candidate > current,
        _ => false,
    }
}

/// The update to offer, if the manifest has one for this platform.
///
/// # Errors
/// Returns a sentence if the asset's address is outside the releases, or its
/// checksum is not 64 hex digits.
pub fn pick(manifest: &Manifest, current: &str, platform: &str) -> Result<Option<Update>, String> {
    if !is_newer(current, &manifest.version) {
        return Ok(None);
    }
    let Some(asset) = manifest.assets.get(platform) else {
        return Ok(None);
    };
    if !asset.url.starts_with(&format!("{RELEASES}/download/")) {
        return Err("the release points outside its own downloads".into());
    }
    if asset.sha256.len() != 64 || hex(&asset.sha256).is_none() {
        return Err("the release has no valid checksum".into());
    }
    Ok(Some(Update {
        version: manifest.version.clone(),
        notes: manifest.notes.clone(),
        asset: asset.clone(),
    }))
}

fn find_program(names: &[&str]) -> Option<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_default();
    names.iter().find_map(|name| {
        env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_absolute() && candidate.is_file())
    })
}

/// Only what a download needs: where programs and temp files are, and the
/// proxy. No token variable is passed on.
fn download_env() -> Vec<(OsString, OsString)> {
    const ALLOWED: [&str; 8] = [
        "PATH",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "HOME",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
    ];
    env::vars_os()
        .filter(|(name, _)| {
            ALLOWED.iter().any(|allowed| {
                name.to_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(allowed))
            })
        })
        .collect()
}

fn curl(guardian: &Path, url: &str, seconds: u64, bytes: usize) -> Result<Vec<u8>, String> {
    let program =
        find_program(&["curl.exe", "curl"]).ok_or("curl was not found on this computer")?;
    let mut args: Vec<OsString> = [
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--max-time",
    ]
    .map(OsString::from)
    .to_vec();
    args.push(seconds.to_string().into());
    args.push(url.into());
    let spec = CommandSpec {
        program,
        args,
        env: download_env(),
        cwd: None,
    };
    let limits = Limits {
        timeout: Duration::from_secs(seconds + 5),
        bytes_per_stream: bytes,
    };
    let output = Runner::new(guardian.to_path_buf())
        .run(&spec, &[], limits, &AtomicBool::new(false))
        .map_err(|error| error.to_string())?;
    match output.outcome {
        Outcome::Exited(status) if status.success() => Ok(output.stdout),
        Outcome::Exited(_) => Err("the download failed".into()),
        Outcome::TimedOut => Err("the download timed out".into()),
        Outcome::OutputLimit => Err("the download was larger than allowed".into()),
        Outcome::Cancelled => Err("cancelled".into()),
    }
}

/// Ask GitHub for the newest release and say whether to offer it.
///
/// # Errors
/// Returns a sentence if the release cannot be fetched or trusted.
pub fn check(guardian: &Path, current: &str) -> Result<Option<Update>, String> {
    let platform = platform().ok_or("updates are not offered on this system")?;
    let base = format!("{RELEASES}/latest/download");
    let manifest = curl(guardian, &format!("{base}/latest.json"), 20, MANIFEST_LIMIT)?;
    let signature = curl(guardian, &format!("{base}/latest.json.sig"), 20, 1024)?;
    let signature =
        String::from_utf8(signature).map_err(|_| "the update signature is unreadable")?;
    let manifest = verify(&manifest, &signature, PUBLIC_KEY_HEX)?;
    pick(&manifest, current, platform)
}

/// Whether `bytes` hash to `expected` (case-insensitive hex).
#[must_use]
pub fn checksum_matches(bytes: &[u8], expected: &str) -> bool {
    let digest = Sha256::digest(bytes);
    to_hex(&digest).eq_ignore_ascii_case(expected)
}

/// Download the installer into `directory` and check its SHA-256.
///
/// # Errors
/// Returns a sentence if the download fails or the file is not the one the
/// signed manifest named. A file that fails the check is never written.
pub fn download(guardian: &Path, update: &Update, directory: &Path) -> Result<PathBuf, String> {
    let name = update.asset.url.rsplit('/').next().unwrap_or_default();
    if name.is_empty() || name.contains(['\\', ':']) || name.starts_with('.') {
        return Err("the installer has an unusable name".into());
    }
    let bytes = curl(guardian, &update.asset.url, 600, INSTALLER_LIMIT)?;
    if !checksum_matches(&bytes, &update.asset.sha256) {
        return Err(
            "the downloaded installer does not match its checksum and was discarded".into(),
        );
    }
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let target = directory.join(format!("{}-{name}", update.version));
    fs::write(&target, bytes).map_err(|error| error.to_string())?;
    Ok(target)
}

/// Hand a downloaded installer to the system, which runs it (Windows) or
/// mounts it (macOS). The caller then closes the window.
///
/// # Errors
/// Returns a sentence if the system opener is missing or cannot start.
pub fn open_installer(guardian: &Path, installer: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        let root = env::var_os("SYSTEMROOT").ok_or("SYSTEMROOT is not set")?;
        PathBuf::from(root).join("explorer.exe")
    } else {
        PathBuf::from("/usr/bin/open")
    };
    // explorer.exe exits with 1 even when it works, so only a launch error counts.
    let spec = CommandSpec {
        program,
        args: vec![installer.into()],
        env: Vec::new(),
        cwd: None,
    };
    let limits = Limits {
        timeout: Duration::from_secs(15),
        bytes_per_stream: 4096,
    };
    Runner::new(guardian.to_path_buf())
        .run(&spec, &[], limits, &AtomicBool::new(false))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    const MANIFEST: &str = r#"{"version":"0.2.0","notes":"n","assets":{"windows-x64":{"url":"https://github.com/robbertpopa/autopilot-mode-on/releases/download/v0.2.0/pipeline-setup-x64.exe","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}}}"#;

    fn signed(manifest: &str) -> (String, String) {
        let signing = SigningKey::from_bytes(&[7; 32]);
        (
            to_hex(&signing.verifying_key().to_bytes()),
            to_hex(&signing.sign(manifest.as_bytes()).to_bytes()),
        )
    }

    // Invariant: only a manifest signed by the release key is believed, so a
    // tampered or re-signed manifest cannot push an installer.
    #[test]
    fn a_manifest_is_trusted_only_with_a_matching_signature() {
        let (key, signature) = signed(MANIFEST);
        assert_eq!(
            verify(MANIFEST.as_bytes(), &signature, &key)
                .unwrap()
                .version,
            "0.2.0"
        );
        let tampered = MANIFEST.replace("0.2.0", "9.9.9");
        assert!(
            verify(tampered.as_bytes(), &signature, &key)
                .unwrap_err()
                .contains("does not match")
        );
        let (_, other) = signed("something else");
        assert!(verify(MANIFEST.as_bytes(), &other, &key).is_err());
        assert!(
            verify(MANIFEST.as_bytes(), "zz", &key)
                .unwrap_err()
                .contains("malformed")
        );
        assert!(
            verify(MANIFEST.as_bytes(), &signature, "00")
                .unwrap_err()
                .contains("malformed")
        );
    }

    // Invariant: the built-in key is a real ed25519 point, so a typo in it
    // fails here instead of silently disabling updates.
    #[test]
    fn the_built_in_key_is_valid() {
        let bytes: [u8; 32] = hex(PUBLIC_KEY_HEX).unwrap().try_into().unwrap();
        assert!(VerifyingKey::from_bytes(&bytes).is_ok());
    }

    // Invariant: only a strictly newer plain version is offered; a downgrade,
    // the same version or a pre-release never is.
    #[test]
    fn only_a_newer_plain_version_is_offered() {
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.9.9", "v0.10.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.2.0-rc.1"));
        assert!(!is_newer("0.1.0", "banana"));
    }

    // Invariant: an asset outside this project's downloads or without a real
    // checksum is refused, and a platform without a build gets no offer.
    #[test]
    fn assets_are_picked_per_platform_and_vetted() {
        let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
        let update = pick(&manifest, "0.1.0", "windows-x64").unwrap().unwrap();
        assert_eq!(update.version, "0.2.0");
        assert_eq!(pick(&manifest, "0.1.0", "macos-arm64").unwrap(), None);
        assert_eq!(pick(&manifest, "0.2.0", "windows-x64").unwrap(), None);
        let mut foreign = manifest.clone();
        foreign.assets.get_mut("windows-x64").unwrap().url = "https://evil.test/x.exe".into();
        assert!(pick(&foreign, "0.1.0", "windows-x64").is_err());
        let mut short = manifest;
        short.assets.get_mut("windows-x64").unwrap().sha256 = "abc".into();
        assert!(pick(&short, "0.1.0", "windows-x64").is_err());
    }

    // Invariant: a file is accepted only when its SHA-256 equals the signed one.
    #[test]
    fn a_download_must_match_its_checksum() {
        let sum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(checksum_matches(b"abc", sum));
        assert!(checksum_matches(b"abc", &sum.to_uppercase()));
        assert!(!checksum_matches(b"abd", sum));
    }
}
