use std::{
    env,
    io::{self, Write},
    process::ExitCode,
};

use autopilot_core::Ticket;

fn main() -> ExitCode {
    match replay() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fake agent: {error}");
            ExitCode::FAILURE
        }
    }
}

fn replay() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("replay")) || args.next().is_some() {
        return Err("expected the replay command".into());
    }
    let _: Ticket = serde_json::from_reader(io::stdin().lock())?;
    io::stdout()
        .lock()
        .write_all(include_bytes!("../../../fixtures/agent/understand.jsonl"))?;
    Ok(())
}
