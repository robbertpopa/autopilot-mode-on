use std::{
    env,
    ffi::OsStr,
    io::{self, Write},
    process::ExitCode,
    thread,
    time::Duration,
};

use autopilot_core::Ticket;

const TRANSCRIPT: &[u8] = include_bytes!("../../../fixtures/agent/understand.jsonl");

fn main() -> ExitCode {
    match replay() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("fake agent: {error}");
            ExitCode::FAILURE
        }
    }
}

fn replay() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new("replay")) {
        return Err("expected replay [truncated|partial-utf8|oversized|hang|crash]".into());
    }
    let fault = args.next();
    if args.next().is_some() {
        return Err("expected at most one replay fault".into());
    }

    let _: Ticket = serde_json::from_reader(io::stdin().lock())?;
    let mut output = io::stdout().lock();
    let activity = TRANSCRIPT
        .split_inclusive(|&byte| byte == b'\n')
        .next()
        .ok_or("the fixture has no activity frame")?;
    match fault.as_deref() {
        None => output.write_all(TRANSCRIPT)?,
        Some(name) => match name.to_str() {
            Some("truncated") => {
                output.write_all(&TRANSCRIPT[..TRANSCRIPT.len().saturating_sub(2)])?;
            }
            Some("partial-utf8") => {
                output.write_all(activity)?;
                output.write_all(b"{\"type\":\"activity\",\"summary\":\"")?;
                output.write_all(&[0xe2, 0x82])?;
            }
            Some("oversized") => {
                let chunk = [b'x'; 4096];
                for _ in 0..32 {
                    output.write_all(&chunk)?;
                }
            }
            Some("hang") => {
                output.write_all(activity)?;
                output.flush()?;
                thread::sleep(Duration::from_secs(30));
            }
            Some("crash") => {
                output.write_all(activity)?;
                output.flush()?;
                return Ok(ExitCode::from(23));
            }
            _ => return Err("unknown replay fault".into()),
        },
    }
    Ok(ExitCode::SUCCESS)
}
