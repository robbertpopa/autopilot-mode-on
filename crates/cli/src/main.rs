use std::{
    env,
    error::Error,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use autopilot_accounts::{Accounts, Status};
use autopilot_agent_claude::{ClaudeAgent, ClaudeConfig};
use autopilot_agent_fixture::FixtureAgent;
use autopilot_core::{Ticket, TicketId};
use autopilot_profile::Auth;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(about = "Run and replay a local coding-agent fixture")]
struct Cli {
    #[arg(long, global = true, default_value = ".autopilot")]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one synthetic Understand step.
    RunStep {
        #[arg(long)]
        step: Step,
        #[arg(long)]
        ticket: PathBuf,
        #[arg(long)]
        agent: Agent,
        /// How the claude agent is billed. Ignored by the fake agent.
        #[arg(long, default_value = "subscription")]
        auth: AuthArg,
        #[arg(long, default_value = "haiku")]
        model: String,
        /// Structured output needs a turn after the answer, so keep this above 1.
        #[arg(long, default_value_t = 4)]
        max_turns: u32,
        /// Dollar cap per call. Applies to --auth api only.
        #[arg(long, default_value = "0.05")]
        max_budget_usd: String,
        /// Absolute path to a native claude executable. Defaults to a PATH search.
        #[arg(long)]
        claude_path: Option<PathBuf>,
        /// Wall-clock limit for the claude run, in seconds.
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
    },
    /// Show which Claude, GitHub and Azure accounts this machine is signed in to.
    Accounts {
        /// Print machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Replay committed events from the ticket's local history.
    Watch {
        ticket: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Step {
    Understand,
}

#[derive(Clone, Copy, ValueEnum)]
enum Agent {
    Fake,
    Claude,
}

#[derive(Clone, Copy, ValueEnum)]
enum AuthArg {
    Subscription,
    Api,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("autopilot: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let mut out = io::stdout().lock();
    match cli.command {
        Command::RunStep {
            step: Step::Understand,
            ticket,
            agent,
            auth,
            model,
            max_turns,
            max_budget_usd,
            claude_path,
            timeout_seconds,
        } => {
            let ticket = read_ticket(&ticket)?;
            let exe = env::current_exe()?;
            let result = match agent {
                Agent::Fake => {
                    let agent = FixtureAgent::beside(&exe)?;
                    autopilot_engine::run_understand(&cli.data_dir, &ticket, &agent)?
                }
                Agent::Claude => {
                    let guardian = exe.with_file_name(format!(
                        "autopilot-process-guardian{}",
                        env::consts::EXE_SUFFIX
                    ));
                    let agent = ClaudeAgent::new(
                        ClaudeConfig {
                            auth: match auth {
                                AuthArg::Subscription => Auth::Subscription,
                                AuthArg::Api => Auth::Api,
                            },
                            model,
                            effort: None,
                            max_turns,
                            max_budget_usd: Some(max_budget_usd),
                            executable: claude_path,
                            timeout: Duration::from_secs(timeout_seconds),
                        },
                        guardian,
                    )?;
                    agent
                        .preflight()
                        .map_err(|error| -> Box<dyn Error> { error })?;
                    autopilot_engine::run_understand(&cli.data_dir, &ticket, &agent)?
                }
            };
            serde_json::to_writer(
                &mut out,
                &serde_json::json!({"ticket": ticket.id, "result": result}),
            )?;
            writeln!(out)?;
        }
        Command::Accounts { json } => {
            let exe = env::current_exe()?;
            let guardian = exe.with_file_name(format!(
                "autopilot-process-guardian{}",
                env::consts::EXE_SUFFIX
            ));
            let accounts = autopilot_accounts::collect(guardian);
            if json {
                serde_json::to_writer_pretty(&mut out, &accounts)?;
                writeln!(out)?;
            } else {
                write_accounts(&mut out, &accounts)?;
            }
        }
        Command::Watch { ticket, after } => {
            for record in autopilot_engine::watch(&cli.data_dir, &TicketId::from(ticket), after)? {
                serde_json::to_writer(&mut out, &record)?;
                writeln!(out)?;
            }
        }
    }
    Ok(())
}

fn write_accounts(out: &mut impl Write, accounts: &Accounts) -> io::Result<()> {
    match &accounts.claude {
        Status::SignedIn(account) => {
            let kind = if account.method == "claude.ai" {
                "subscription"
            } else {
                "API key"
            };
            let mut line = format!("Claude  signed in with {kind}");
            for part in [&account.email, &account.organization, &account.plan]
                .into_iter()
                .flatten()
            {
                line.push_str(" | ");
                line.push_str(part);
            }
            writeln!(out, "{line}")?;
        }
        other => writeln!(out, "Claude  {}", unavailable(other))?,
    }
    match &accounts.github {
        Status::SignedIn(list) => {
            for (index, account) in list.iter().enumerate() {
                let label = if index == 0 { "GitHub " } else { "       " };
                let mut flags = Vec::new();
                if account.active {
                    flags.push("active");
                }
                if !account.valid {
                    flags.push("token rejected");
                }
                let flags = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", flags.join(", "))
                };
                writeln!(out, "{label} {}{flags} on {}", account.login, account.host)?;
            }
        }
        other => writeln!(out, "GitHub  {}", unavailable(other))?,
    }
    match &accounts.azure {
        Status::SignedIn(account) => {
            writeln!(
                out,
                "Azure   {} | subscription {}",
                account.user, account.subscription
            )?;
        }
        other => writeln!(out, "Azure   {}", unavailable(other))?,
    }
    Ok(())
}

fn unavailable<T>(status: &Status<T>) -> String {
    match status {
        Status::SignedIn(_) => String::new(),
        Status::SignedOut => "not signed in".into(),
        Status::NotInstalled => "not installed".into(),
        Status::Failed(reason) => format!("could not be checked: {reason}"),
    }
}

fn read_ticket(path: &Path) -> Result<Ticket, Box<dyn Error>> {
    const LIMIT: u64 = 1024 * 1024;
    let mut input = Vec::new();
    File::open(path)?.take(LIMIT + 1).read_to_end(&mut input)?;
    if input.len() as u64 > LIMIT {
        return Err("ticket file exceeds 1 MiB".into());
    }
    Ok(serde_json::from_slice(&input)?)
}
