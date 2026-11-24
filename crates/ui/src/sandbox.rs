//! Settings, Sandbox: why Build is refused, row by row, and the commands that
//! fix it. The probe is the one Build runs (`build_actions::probe_sandbox`), so
//! this page can never say "ready" when Build would refuse.

use eframe::egui::{self, RichText};

use crate::{app::App, backend::Message, theme};

/// Where the probe stands.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum Probe {
    #[default]
    Unknown,
    Running,
    Ready,
    Refused(String),
}

/// The refusal text split into what the owner can act on.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Report {
    /// Tools discovery did not find.
    pub(crate) missing: Vec<String>,
    /// Probe rows that did not behave as expected.
    pub(crate) failed: Vec<String>,
    /// Anything else the machine said, kept whole.
    pub(crate) other: Option<String>,
}

const MISSING: &str = " is missing: ";
const FAILED: &str = " check(s): ";

/// Read the words `Wsl::discover` and `isolation::validate` refuse with. Text
/// that matches neither is kept as it came, never dropped.
pub(crate) fn explain(refusal: &str) -> Report {
    let mut report = Report::default();
    if let Some((_, tools)) = refusal.split_once(MISSING) {
        report.missing = tools
            .split(',')
            .map(|tool| tool.trim().to_owned())
            .filter(|tool| !tool.is_empty())
            .collect();
    } else if let Some((_, rows)) = refusal.split_once(FAILED) {
        report.failed = rows
            .split("; ")
            .map(|row| row.trim().to_owned())
            .filter(|row| !row.is_empty())
            .collect();
    }
    if report.missing.is_empty() && report.failed.is_empty() {
        report.other = Some(refusal.to_owned());
    }
    report
}

/// The commands that fix what is missing, in the order to run them. `windows`
/// picks the `wsl` form; otherwise they run on this Linux machine.
pub(crate) fn fixes(report: &Report, distro: &str, windows: bool) -> Vec<String> {
    let run = |user: Option<&str>, script: &str| match (windows, user) {
        (true, Some(user)) => format!("wsl -d {distro} -u {user} -- bash tools/wsl/{script}"),
        (true, None) => format!("wsl -d {distro} -- bash tools/wsl/{script}"),
        (false, Some("root")) => format!("sudo bash tools/wsl/{script}"),
        (false, _) => format!("bash tools/wsl/{script}"),
    };
    let packages = ["bwrap", "bubblewrap", "socat", "rg", "ripgrep", "python3"];
    let mut commands = Vec::new();
    if report
        .missing
        .iter()
        .any(|tool| packages.iter().any(|name| tool.contains(name)))
    {
        commands.push(run(Some("root"), "setup-root.sh"));
    }
    let hook = |tool: &String| tool.contains("autopilot-hook");
    if report
        .missing
        .iter()
        .any(|tool| !hook(tool) && !packages.iter().any(|name| tool.contains(name)))
    {
        commands.push(run(None, "setup-user.sh"));
    }
    if report.missing.iter().any(hook) || !report.failed.is_empty() {
        commands.push(run(None, "install-hook.sh"));
    }
    commands
}

impl App {
    pub(crate) fn sandbox_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(RichText::new("Sandbox").size(18.0).strong());
        ui.label(theme::muted(
            "Build runs the agent and the checks only inside a Linux boundary that passed this probe. The probe takes about five seconds.",
        ));
        ui.add_space(10.0);
        if !cfg!(any(windows, target_os = "linux")) {
            ui.label(
                "This computer is not a Linux or Windows host, so Build with Claude stays off here. The practice agent still works.",
            );
            return;
        }
        let running = self.sandbox == Probe::Running;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!running, theme::primary("Run the probe"))
                .clicked()
            {
                self.sandbox = Probe::Running;
                let options = self.options.clone();
                let sender = self.sender.clone();
                let context = self.context.clone();
                std::thread::spawn(move || {
                    let outcome = crate::build_actions::probe_sandbox(&options).map(|_| ());
                    let _ = sender.send(Message::Sandbox(outcome));
                    context.request_repaint();
                });
            }
            if running {
                ui.add(egui::Spinner::new());
            }
        });
        ui.add_space(8.0);
        match self.sandbox.clone() {
            Probe::Unknown => {
                ui.label(theme::muted("Not run in this window yet."));
            }
            Probe::Running => {
                ui.label("Running the probe...");
            }
            Probe::Ready => {
                ui.label(
                    RichText::new("✔ Every row held. Build can use the sandbox.")
                        .color(theme::GREEN()),
                );
            }
            Probe::Refused(refusal) => {
                let report = explain(&refusal);
                ui.label(
                    RichText::new("Build is refused")
                        .color(theme::RED())
                        .strong(),
                );
                for tool in &report.missing {
                    ui.label(format!("× missing: {tool}"));
                }
                for row in &report.failed {
                    ui.label(format!("× {row}"));
                }
                if let Some(other) = &report.other {
                    ui.label(format!("× {other}"));
                }
                let distro =
                    std::env::var("AUTOPILOT_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu".to_owned());
                let commands = fixes(&report, &distro, cfg!(windows));
                if !commands.is_empty() {
                    ui.add_space(8.0);
                    ui.label("Run these from the project folder, then probe again:");
                    for command in commands {
                        ui.add(
                            egui::Label::new(RichText::new(command).monospace()).selectable(true),
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: the refusal Discovery gives for absent tools becomes one row
    // per tool and the command that installs them. A change that showed the
    // sentence whole would leave the owner to guess which script to run.
    #[test]
    fn missing_tools_become_rows_and_the_right_commands() {
        let report = explain(
            "the WSL distro Ubuntu is missing: bwrap, socat, autopilot-hook (run tools/wsl/install-hook.sh)",
        );
        assert_eq!(report.missing.len(), 3, "{report:?}");
        assert!(report.failed.is_empty() && report.other.is_none());
        let commands = fixes(&report, "Ubuntu", true);
        assert_eq!(
            commands,
            [
                "wsl -d Ubuntu -u root -- bash tools/wsl/setup-root.sh",
                "wsl -d Ubuntu -- bash tools/wsl/install-hook.sh"
            ]
        );
        assert_eq!(
            fixes(&report, "Ubuntu", false)[0],
            "sudo bash tools/wsl/setup-root.sh"
        );
    }

    // Invariant: each failed probe row is listed by itself, and text this page
    // does not understand is kept, never replaced by "ready".
    #[test]
    fn failed_rows_are_listed_and_unknown_text_is_kept() {
        let report =
            explain("the WSL boundary failed 2 check(s): agent: write-outside; check: net-closed");
        assert_eq!(report.failed, ["agent: write-outside", "check: net-closed"]);
        let odd = explain("something else broke");
        assert_eq!(odd.other.as_deref(), Some("something else broke"));
        assert_eq!(fixes(&odd, "Ubuntu", true), Vec::<String>::new());
    }
}
