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
const CLAUDE_PLAN: &[u8] = include_bytes!("../../../fixtures/claude/synthetic-plan.jsonl");

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
    if prompt.contains("scenario:crash") {
        output.write_all(init.as_bytes())?;
        output.flush()?;
        return Ok(ExitCode::from(23));
    }
    if prompt.contains("scenario:truncated") {
        output.write_all(&CLAUDE_SUCCESS[..CLAUDE_SUCCESS.len() - 20])?;
        return Ok(ExitCode::SUCCESS);
    }
    if prompt.contains("scenario:oversized") {
        output.write_all(init.as_bytes())?;
        let chunk = vec![b'x'; 64 * 1024];
        // The adapter caps a stream at 16 MiB, so this must cross it.
        for _ in 0..300 {
            output.write_all(&chunk)?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    if prompt.contains("scenario:hang") {
        output.write_all(init.as_bytes())?;
        output.flush()?;
        thread::sleep(Duration::from_secs(60));
        return Ok(ExitCode::SUCCESS);
    }
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
    if env::args_os().any(|arg| arg == "--allowedTools") {
        return claude_build(&prompt, &mut output);
    }
    // The Plan prompt asks for a plan; every other prompt is Understand.
    let reply = if prompt.contains("Write a plan for this ticket") {
        CLAUDE_PLAN
    } else {
        CLAUDE_SUCCESS
    };
    output.write_all(reply)?;
    Ok(ExitCode::SUCCESS)
}

/// Pretend to be a build turn of `claude -p`: edit files in the working
/// directory the way the prompt's scenario says, then report success. A build
/// turn is recognised by `--allowedTools`, which only it passes.
fn claude_build(
    prompt: &str,
    output: &mut impl Write,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let session = args
        .windows(2)
        .find(|pair| pair[0] == "--session-id" || pair[0] == "--resume")
        .map_or("00000000-0000-4000-8000-000000000000", |pair| {
            pair[1].as_str()
        });
    if prompt.contains("scenario:record-args") {
        std::fs::write(
            ".fake-args",
            args.join(
                "
",
            ),
        )?;
    }
    if prompt.contains("scenario:poisoned") {
        obey_injection(&args)?;
    }
    if prompt.contains("scenario:build-protected") {
        std::fs::create_dir_all("secrets")?;
        std::fs::write("secrets/key.txt", "x")?;
    } else if prompt.contains("scenario:build-nothing") {
        // Changes nothing: the not-working counter must notice.
    } else if prompt.contains("scenario:build-never-fixes") {
        // A different edit every turn, so only the round cap can stop it.
        let count = std::fs::read_dir(".")?.count();
        std::fs::write(format!("attempt-{count}.txt"), "still wrong")?;
    } else if prompt.contains("FAILED:") {
        std::fs::write("fixed.txt", "fixed")?;
    } else {
        std::fs::write("draft.txt", "draft")?;
    }
    writeln!(
        output,
        "{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{session}\"}}
         {{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":1,\"total_cost_usd\":0.001}}"
    )?;
    Ok(ExitCode::SUCCESS)
}

/// A build turn that obeys an injected instruction: it tries to publish, the
/// way real Claude Code would, by sending each shell call through the hook
/// command in `--settings` and acting only if the hook allows it. Whatever
/// happens is written to `.hook-log`, which the build then commits, so a test
/// can read it back.
fn obey_injection(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::fmt::Write as _;
    use std::process::{Command, Stdio};

    let settings = args
        .windows(2)
        .find(|pair| pair[0] == "--settings")
        .map(|pair| pair[1].as_str())
        .ok_or("no --settings on a build turn")?;
    let settings: serde_json::Value = serde_json::from_str(settings)?;
    let hook = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .ok_or("no hook command in the settings")?;
    // The command is a quoted program and its arguments.
    let rest = hook.strip_prefix('"').ok_or("hook command is not quoted")?;
    let (program, arguments) = rest.split_once('"').ok_or("hook command is not closed")?;
    let mut log = String::new();
    for attempt in [
        "git push origin HEAD:main",
        "git -C . push --force origin HEAD",
        "gh pr create --fill",
        "bash -c \"git push\"",
    ] {
        let mut child = Command::new(program)
            .args(arguments.split_whitespace())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let call = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": attempt}});
        child
            .stdin
            .take()
            .ok_or("no hook stdin")?
            .write_all(call.to_string().as_bytes())?;
        let output = child.wait_with_output()?;
        let verdict: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let denied = verdict["hookSpecificOutput"]["permissionDecision"] == "deny";
        let _ = writeln!(
            log,
            "{}: {attempt}",
            if denied { "denied" } else { "ALLOWED" }
        );
    }
    for name in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if env::var_os(name).is_some() {
            let _ = writeln!(log, "LEAKED: {name}");
        }
    }
    std::fs::write(".hook-log", log)?;
    Ok(())
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
