//! `autopilot-mcp --data-dir <dir> --read-root <dir>... --write-root <dir>...`
//!
//! Speaks MCP on standard input and output. It exits with an error unless it
//! has a data directory and at least one read and one write root. With
//! `--ticket <id>`, the tool `approve` asks the owner about that ticket;
//! without it `approve` denies every call.

use std::{io::BufReader, path::PathBuf, process::ExitCode, time::Duration};

use autopilot_core::TicketId;
use autopilot_mcp::Server;

/// Longer than an owner is likely to leave a tool call unanswered; after it the
/// call is denied.
const PROMPT_WAIT: Duration = Duration::from_secs(600);

type Options = (PathBuf, Vec<PathBuf>, Vec<PathBuf>, Option<TicketId>);

fn parse() -> Result<Options, String> {
    let mut data = None;
    let mut ticket = None;
    let (mut reads, mut writes) = (Vec::new(), Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--data-dir" => data = Some(PathBuf::from(value)),
            "--read-root" => reads.push(PathBuf::from(value)),
            "--write-root" => writes.push(PathBuf::from(value)),
            "--ticket" => ticket = Some(TicketId::from(value)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let data = data.ok_or("--data-dir is required")?;
    Ok((data, reads, writes, ticket))
}

fn main() -> ExitCode {
    let outcome = parse().and_then(|(data, reads, writes, ticket)| {
        let mut server = Server::new(data, &reads, &writes).map_err(|error| error.to_string())?;
        if let Some(ticket) = ticket {
            server = server.with_permission_prompt(ticket, PROMPT_WAIT);
        }
        let stdin = std::io::stdin();
        server
            .serve(BufReader::new(stdin.lock()), std::io::stdout().lock())
            .map_err(|error| error.to_string())
    });
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("autopilot-mcp: {message}");
            ExitCode::FAILURE
        }
    }
}
