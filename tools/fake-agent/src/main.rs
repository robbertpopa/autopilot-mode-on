use std::{
    env,
    ffi::OsStr,
    io::{self, Read, Write},
    process::ExitCode,
    thread,
    time::Duration,
};

use autopilot_core::Ticket;

const TRANSCRIPT: &[u8] = include_bytes!("../../../fixtures/agent/understand.jsonl");
const CLAUDE_SUCCESS: &[u8] = include_bytes!("../../../fixtures/claude/synthetic-understand.jsonl");

fn main() -> ExitCode {
    // Real Claude Code starts with `-p` or `auth`; the replay mode keeps its own word.
    let first = env::args_os().nth(1);
    let result = match first.as_deref().and_then(OsStr::to_str) {
        Some("-p") => claude_print(),
        Some("auth") => claude_status(),
        _ => replay(),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("fake agent: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Pretend to be `claude auth status --json` for a subscription sign-in.
fn claude_status() -> Result<ExitCode, Box<dyn std::error::Error>> {
    writeln!(
        io::stdout(),
        "{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}"
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Pretend to be `claude -p`. The ticket description picks the outcome so a
/// test can drive each result shape without a real model.
fn claude_print() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut prompt = String::new();
    io::stdin().lock().read_to_string(&mut prompt)?;
    let mut output = io::stdout().lock();
    let init = "{\"type\":\"system\",\"subtype\":\"init\"}\n";
    if prompt.contains("scenario:max-turns") {
        writeln!(
            output,
            "{init}{{\"type\":\"result\",\"subtype\":\"error_max_turns\",\"is_error\":true,\"total_cost_usd\":0.002}}"
        )?;
        return Ok(ExitCode::FAILURE);
    }
    if prompt.contains("scenario:rate-limited") {
        write!(
            output,
            "{init}{{\"type\":\"system\",\"subtype\":\"api_retry\",\"attempt\":1,\"error_status\":429}}\n\
             {{\"type\":\"assistant\",\"error\":\"rate_limit\"}}\n\
             {{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true}}\n"
        )?;
        return Ok(ExitCode::FAILURE);
    }
    output.write_all(CLAUDE_SUCCESS)?;
    Ok(ExitCode::SUCCESS)
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
