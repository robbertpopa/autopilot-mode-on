//! `autopilot-mcp --data-dir <dir> --read-root <dir>... --write-root <dir>...`
//!
//! Speaks MCP on standard input and output. It exits with an error unless it
//! has a data directory and at least one read and one write root.

use std::{io::BufReader, path::PathBuf, process::ExitCode};

use autopilot_mcp::Server;

fn parse() -> Result<(PathBuf, Vec<PathBuf>, Vec<PathBuf>), String> {
    let mut data = None;
    let (mut reads, mut writes) = (Vec::new(), Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--data-dir" => data = Some(PathBuf::from(value)),
            "--read-root" => reads.push(PathBuf::from(value)),
            "--write-root" => writes.push(PathBuf::from(value)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let data = data.ok_or("--data-dir is required")?;
    Ok((data, reads, writes))
}

fn main() -> ExitCode {
    let outcome = parse().and_then(|(data, reads, writes)| {
        let server = Server::new(data, &reads, &writes).map_err(|error| error.to_string())?;
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
