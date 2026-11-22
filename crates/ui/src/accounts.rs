//! Who is signed in to GitHub, Claude, Codex and Azure, read from the tools
//! themselves. The window never sees a token: only names and how each tool
//! is signed in.

use autopilot_accounts::{Accounts, Status};
use eframe::egui::{self, Color32, RichText};

use crate::app::App;
use crate::backend;
use crate::theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Good,
    /// Installed but not signed in, or signed in with something rejected.
    Attention,
    Missing,
}

/// One service as the window shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub name: &'static str,
    pub level: Level,
    /// One line each: who, how, where.
    pub lines: Vec<String>,
    /// What to type to fix it, when there is something to fix.
    pub hint: Option<&'static str>,
}

fn problem<T>(status: &Status<T>) -> (Level, String) {
    match status {
        Status::SignedIn(_) => (Level::Good, String::new()),
        Status::SignedOut => (Level::Attention, "not signed in".into()),
        Status::NotInstalled => (Level::Missing, "not installed".into()),
        Status::Failed(reason) => (Level::Attention, format!("could not be checked: {reason}")),
    }
}

/// The four rows, in the order the window lists them.
#[must_use]
pub fn rows(accounts: &Accounts) -> Vec<Row> {
    let mut rows = Vec::new();

    let (level, text) = problem(&accounts.github);
    let mut lines = vec![];
    let mut level = level;
    if let Status::SignedIn(list) = &accounts.github {
        for account in list {
            let mut line = format!("{} on {}", account.login, account.host);
            if account.active {
                line.push_str(" (active)");
            }
            if !account.valid {
                line.push_str(" (token rejected)");
            }
            lines.push(line);
        }
        if list.iter().all(|account| !account.valid) {
            level = Level::Attention;
        }
    } else {
        lines.push(text);
    }
    rows.push(Row {
        name: "GitHub",
        level,
        lines,
        hint: (level != Level::Good).then_some("gh auth login"),
    });

    let (level, text) = problem(&accounts.claude);
    let lines = match &accounts.claude {
        Status::SignedIn(account) => {
            let how = if account.method == "claude.ai" {
                "subscription"
            } else {
                "API key"
            };
            let mut lines = vec![format!("signed in with {how}")];
            lines.extend(
                [&account.email, &account.organization, &account.plan]
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            lines
        }
        _ => vec![text],
    };
    rows.push(Row {
        name: "Claude",
        level,
        lines,
        hint: (level != Level::Good).then_some("claude auth login"),
    });

    let (level, text) = problem(&accounts.codex);
    let lines = match &accounts.codex {
        Status::SignedIn(account) => vec![format!("signed in with {}", account.method)],
        _ => vec![text],
    };
    rows.push(Row {
        name: "Codex",
        level,
        lines,
        hint: (level != Level::Good).then_some("codex login"),
    });

    let (level, text) = problem(&accounts.azure);
    let lines = match &accounts.azure {
        Status::SignedIn(account) => vec![
            account.user.clone(),
            format!("subscription {}", account.subscription),
        ],
        _ => vec![text],
    };
    rows.push(Row {
        name: "Azure",
        level,
        lines,
        hint: (level != Level::Good).then_some("az login"),
    });
    rows
}

fn color(level: Level) -> Color32 {
    match level {
        Level::Good => theme::GREEN(),
        Level::Attention => theme::AMBER(),
        Level::Missing => theme::TEXT_MUTED(),
    }
}

impl App {
    pub(crate) fn accounts_screen(&mut self, ui: &mut egui::Ui) {
        self.mode_tabs(ui);
        ui.add_space(14.0);
        ui.label(RichText::new("Who is signed in?").size(30.0).strong());
        ui.label(theme::muted(
            "Read from the tools on this computer. No password or token is shown or kept.",
        ));
        ui.add_space(10.0);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Agent");
            let before = self.agent;
            theme::segmented(
                ui,
                &mut self.agent,
                &[
                    (crate::backend::AgentChoice::Claude, "Claude"),
                    (crate::backend::AgentChoice::Practice, "Offline demo"),
                ],
            );
            if self.agent != before {
                self.agent.save(&self.options.data_dir);
            }
        });
        ui.label(theme::muted(
            "Claude uses your Claude Code sign-in. The offline demo gives fixed sample answers, spends nothing and needs no sign-in; it is for trying the app.",
        ));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Orchestrator");
            let before = self.orchestrator;
            theme::segmented(
                ui,
                &mut self.orchestrator,
                &[
                    (crate::backend::Orchestrator::Off, "Off"),
                    (crate::backend::Orchestrator::Sonnet, "Sonnet"),
                    (crate::backend::Orchestrator::Opus, "Opus"),
                ],
            );
            if self.orchestrator != before {
                self.orchestrator.save(&self.options.data_dir);
            }
        });
        ui.label(theme::muted(
            "After Understand, one call by this model judges how complex the ticket is against what each step costs, and picks the cheapest model and effort for Design and Plan. You can change any of its choices.",
        ));
        ui.add_space(10.0);
        let Some(accounts) = self.accounts.clone() else {
            ui.label(theme::muted("Checking GitHub, Claude, Codex and Azure..."));
            return;
        };
        for row in rows(&accounts) {
            theme::card(theme::CARD(), theme::BORDER()).show(ui, |ui| {
                ui.set_width(ui.available_width() - 38.0);
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 6.0, color(row.level));
                    ui.label(RichText::new(row.name).size(18.0).strong());
                });
                for line in &row.lines {
                    ui.label(line);
                }
                if let Some(hint) = row.hint {
                    ui.horizontal(|ui| {
                        ui.label(theme::muted("To fix, run in a terminal:"));
                        ui.label(theme::mono(hint));
                    });
                }
            });
            ui.add_space(6.0);
        }
        if ui.add(theme::primary("Check again")).clicked() {
            self.accounts = None;
            backend::fetch_accounts(self.context.clone(), self.sender.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use autopilot_accounts::{
        AzureAccount, ClaudeAccount, CodexAccount, GithubAccount, parse_azure, parse_claude,
        parse_codex, parse_github,
    };

    use super::*;

    fn signed_in() -> Accounts {
        Accounts {
            claude: Status::SignedIn(ClaudeAccount {
                method: "claude.ai".into(),
                email: Some("a@example.test".into()),
                organization: None,
                plan: Some("pro".into()),
            }),
            codex: Status::SignedIn(CodexAccount {
                method: "ChatGPT subscription".into(),
            }),
            github: Status::SignedIn(vec![
                GithubAccount {
                    host: "github.com".into(),
                    login: "one".into(),
                    active: true,
                    valid: true,
                },
                GithubAccount {
                    host: "github.com".into(),
                    login: "two".into(),
                    active: false,
                    valid: false,
                },
            ]),
            azure: Status::SignedIn(AzureAccount {
                user: "me@example.test".into(),
                subscription: "dev".into(),
            }),
        }
    }

    // Invariant: a signed-in service is good and says who and how; every
    // account GitHub lists is shown, with the active one marked.
    #[test]
    fn signed_in_services_say_who_and_how() {
        let rows = rows(&signed_in());
        assert_eq!(
            rows.iter().map(|row| row.name).collect::<Vec<_>>(),
            ["GitHub", "Claude", "Codex", "Azure"]
        );
        assert!(rows.iter().all(|row| row.level == Level::Good));
        assert_eq!(rows[0].lines[0], "one on github.com (active)");
        assert_eq!(rows[0].lines[1], "two on github.com (token rejected)");
        assert_eq!(rows[1].lines[0], "signed in with subscription");
        assert_eq!(rows[2].lines, ["signed in with ChatGPT subscription"]);
        assert!(rows.iter().all(|row| row.hint.is_none()));
    }

    // Invariant: a missing or signed-out service is flagged and tells the
    // owner the command that fixes it.
    #[test]
    fn a_signed_out_service_gets_the_command_that_fixes_it() {
        let mut accounts = signed_in();
        accounts.codex = Status::SignedOut;
        accounts.azure = Status::NotInstalled;
        let rows = rows(&accounts);
        assert_eq!(rows[2].level, Level::Attention);
        assert_eq!(rows[2].hint, Some("codex login"));
        assert_eq!(rows[3].level, Level::Missing);
        assert_eq!(rows[3].lines, ["not installed"]);
    }

    // Invariant: the rows are built from what the parsers read from the real
    // tools' output, not from anything typed into the window.
    #[test]
    fn rows_follow_the_parsers() {
        let accounts = Accounts {
            claude: parse_claude(r#"{"loggedIn":true,"authMethod":"apiKey"}"#),
            codex: parse_codex("Logged in using an API key"),
            github: parse_github(
                r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com","login":"me"}]}}"#,
            ),
            azure: parse_azure("{}"),
        };
        let rows = rows(&accounts);
        assert_eq!(rows[0].level, Level::Good);
        assert_eq!(rows[1].lines[0], "signed in with API key");
        assert_eq!(rows[2].lines, ["signed in with API key"]);
        assert_ne!(rows[3].level, Level::Good);
    }
}
