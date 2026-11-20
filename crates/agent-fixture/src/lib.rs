//! Synthetic Understand protocol used by the first CLI slice.

use std::{
    env, io,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

use autopilot_agent::{Agent, AgentError, UnderstandRun};
use autopilot_core::{Ticket, Understanding};
use autopilot_process::{CommandSpec, Limits, Outcome, Runner};
use serde::Deserialize;

pub struct FixtureAgent {
    program: PathBuf,
    runner: Runner,
}

impl FixtureAgent {
    /// Locate the fixture tools beside the CLI binary.
    ///
    /// # Errors
    /// Returns an error if a required sibling binary is missing.
    pub fn beside(cli: &Path) -> io::Result<Self> {
        let program =
            cli.with_file_name(format!("autopilot-fake-agent{}", env::consts::EXE_SUFFIX));
        let guardian = cli.with_file_name(format!(
            "autopilot-process-guardian{}",
            env::consts::EXE_SUFFIX
        ));
        if !program.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("fixture agent missing at {}", program.display()),
            ));
        }
        if cfg!(target_os = "linux") && !guardian.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("process guardian missing at {}", guardian.display()),
            ));
        }
        Ok(Self {
            program,
            runner: Runner::new(guardian),
        })
    }
}

impl Agent for FixtureAgent {
    fn understand(&self, ticket: &Ticket) -> Result<UnderstandRun, AgentError> {
        let mut child_env = Vec::new();
        if cfg!(windows)
            && let Some(root) = env::var_os("SystemRoot")
        {
            child_env.push(("SystemRoot".into(), root));
        }
        let spec = CommandSpec {
            program: self.program.clone(),
            args: vec!["replay".into()],
            env: child_env,
        };
        let output = self.runner.run(
            &spec,
            &serde_json::to_vec(ticket)?,
            Limits {
                timeout: Duration::from_secs(10),
                bytes_per_stream: 64 * 1024,
            },
            &AtomicBool::new(false),
        )?;
        match output.outcome {
            Outcome::Exited(status) if status.success() => decode(&output.stdout),
            Outcome::Exited(status) => {
                Err(io::Error::other(format!("synthetic agent exited with {status}")).into())
            }
            Outcome::TimedOut => {
                Err(io::Error::new(io::ErrorKind::TimedOut, "synthetic agent timed out").into())
            }
            Outcome::Cancelled => Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "synthetic agent was cancelled",
            )
            .into()),
            Outcome::OutputLimit => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "synthetic agent exceeded its output limit",
            )
            .into()),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Frame {
    Activity {
        summary: String,
    },
    Result {
        result: Understanding,
    },
    #[serde(other)]
    Unknown,
}

fn decode(bytes: &[u8]) -> Result<UnderstandRun, AgentError> {
    let mut activity = Vec::new();
    let mut result = None;
    for line in bytes.split(|&byte| byte == b'\n') {
        if line.trim_ascii().is_empty() {
            continue;
        }
        match serde_json::from_slice(line)? {
            Frame::Activity { summary } => activity.push(summary),
            Frame::Result { result: value } => {
                if result.replace(value).is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "fixture sent more than one result",
                    )
                    .into());
                }
            }
            Frame::Unknown => {}
        }
    }
    let result = result.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "fixture ended without a result",
        )
    })?;
    Ok(UnderstandRun { activity, result })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixture_requires_one_complete_result() {
        let fixture = include_bytes!("../../../fixtures/agent/understand.jsonl");
        let run = decode(fixture).unwrap();
        assert_eq!(run.activity.len(), 1);
        let end = fixture.iter().rposition(|&byte| byte == b'}').unwrap();
        assert!(decode(&fixture[..end]).is_err());
        assert!(decode(&[fixture.as_slice(), fixture.as_slice()].concat()).is_err());
    }
}
