//! A desktop notification, sent through the system's own tool.
//!
//! The title and text come from tickets, which are untrusted, so they never
//! become part of a command line that a shell or a script parses: they travel
//! as separate arguments or as environment variables that a fixed script
//! reads. Nothing is installed and nothing is kept: if the system tool is
//! missing the notification is simply not shown and the reason is returned.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

use autopilot_process::{CommandSpec, Limits, Runner};

/// The longest title and text sent; more is cut.
const MAX_TITLE: usize = 80;
const MAX_TEXT: usize = 240;

/// A fixed Windows PowerShell script. It reads the text from the environment.
const WINDOWS_SCRIPT: &str = "\
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null; \
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument; \
$xml.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text id=\"1\"></text><text id=\"2\"></text></binding></visual></toast>'); \
$texts = $xml.GetElementsByTagName('text'); \
$texts.Item(0).AppendChild($xml.CreateTextNode($env:AUTOPILOT_NOTIFY_TITLE)) | Out-Null; \
$texts.Item(1).AppendChild($xml.CreateTextNode($env:AUTOPILOT_NOTIFY_TEXT)) | Out-Null; \
$toast = New-Object Windows.UI.Notifications.ToastNotification $xml; \
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe').Show($toast)";

/// A fixed `AppleScript`. The title and text arrive as its two arguments.
const MACOS_SCRIPT: &str = "on run argv\n\
display notification (item 2 of argv) with title (item 1 of argv)\n\
end run";

fn clip(text: &str, limit: usize) -> String {
    // Control characters would only make a notification unreadable.
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(limit)
        .collect()
}

/// The command that shows a notification on this system, if the system has a
/// tool for it. Split from [`send`] so it can be checked without showing one.
#[must_use]
pub fn command(title: &str, text: &str) -> Option<CommandSpec> {
    let title = clip(title, MAX_TITLE);
    let text = clip(text, MAX_TEXT);
    if cfg!(windows) {
        let root = env::var_os("SYSTEMROOT")?;
        let program = PathBuf::from(&root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        Some(CommandSpec {
            program,
            args: ["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]
                .map(OsString::from)
                .to_vec(),
            env: vec![
                ("SystemRoot".into(), root),
                ("AUTOPILOT_NOTIFY_TITLE".into(), title.into()),
                ("AUTOPILOT_NOTIFY_TEXT".into(), text.into()),
            ],
            cwd: None,
        })
    } else if cfg!(target_os = "macos") {
        Some(CommandSpec {
            program: PathBuf::from("/usr/bin/osascript"),
            args: vec!["-e".into(), MACOS_SCRIPT.into(), title.into(), text.into()],
            env: Vec::new(),
            cwd: None,
        })
    } else {
        Some(CommandSpec {
            program: PathBuf::from("/usr/bin/notify-send"),
            args: vec!["--".into(), title.into(), text.into()],
            env: Vec::new(),
            cwd: None,
        })
    }
}

/// Show a notification.
///
/// # Errors
/// Returns a sentence if the system tool is missing, cannot start or fails.
pub fn send(guardian: &Path, title: &str, text: &str) -> Result<(), String> {
    let spec = command(title, text).ok_or("this system has no notification tool")?;
    if !spec.program.exists() {
        return Err(format!("{} is not installed", spec.program.display()));
    }
    let limits = Limits {
        timeout: Duration::from_secs(15),
        bytes_per_stream: 4096,
    };
    let output = Runner::new(guardian.to_path_buf())
        .run(&spec, &[], limits, &AtomicBool::new(false))
        .map_err(|error| error.to_string())?;
    match output.outcome {
        autopilot_process::Outcome::Exited(status) if status.success() => Ok(()),
        other => Err(format!("the notification tool ended with {other:?}")),
    }
}

/// Open a terminal window in a folder, for the owner to look at a worktree by
/// hand. It lives here because `ui` may not start programs and `notify` already
/// is the window's way to the desktop.
///
/// # Errors
/// Returns a sentence if the folder is not a folder or no terminal could start.
pub fn open_terminal(folder: &Path) -> Result<(), String> {
    autopilot_process::terminal::open_terminal(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: untrusted text is never part of a script or command string.
    // It travels as an argument or an environment value, control characters
    // are removed and the length is capped.
    #[test]
    fn untrusted_text_stays_out_of_the_script() {
        let nasty = "x'; Remove-Item -Recurse C:\\ #\n\"$(calc)";
        let spec = command(nasty, &"a".repeat(1000)).unwrap();
        // Every value the command carries: arguments and environment.
        let values: Vec<String> = spec
            .args
            .iter()
            .chain(spec.env.iter().map(|(_, value)| value))
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        let mut carried = false;
        for value in &values {
            if value == WINDOWS_SCRIPT || value == MACOS_SCRIPT {
                // The fixed script never holds the text.
                assert!(!value.contains("Remove-Item"), "{value}");
            } else {
                // Anything else is data: one line and no longer than the cap.
                assert!(!value.contains('\n'), "{value}");
                assert!(value.chars().count() <= MAX_TEXT, "{value}");
                carried |= value.contains("Remove-Item");
            }
        }
        assert!(carried, "the text must still reach the notification");
    }
}
