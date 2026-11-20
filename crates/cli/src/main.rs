use std::{
    env,
    error::Error,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use autopilot_agent_fixture::FixtureAgent;
use autopilot_core::{Ticket, TicketId};
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
            agent: Agent::Fake,
        } => {
            let ticket = read_ticket(&ticket)?;
            let agent = FixtureAgent::beside(&env::current_exe()?)?;
            let result = autopilot_engine::run_understand(&cli.data_dir, &ticket, &agent)?;
            serde_json::to_writer(
                &mut out,
                &serde_json::json!({"ticket": ticket.id, "result": result}),
            )?;
            writeln!(out)?;
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

fn read_ticket(path: &Path) -> Result<Ticket, Box<dyn Error>> {
    const LIMIT: u64 = 1024 * 1024;
    let mut input = Vec::new();
    File::open(path)?.take(LIMIT + 1).read_to_end(&mut input)?;
    if input.len() as u64 > LIMIT {
        return Err("ticket file exceeds 1 MiB".into());
    }
    Ok(serde_json::from_slice(&input)?)
}
