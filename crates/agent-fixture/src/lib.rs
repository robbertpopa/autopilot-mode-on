//! Synthetic Understand protocol used by the first CLI slice.

use std::{
    env, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use autopilot_agent::{Agent, AgentContext, AgentError, AgentRun, StepRequest};
use autopilot_core::{Step, Understanding};
use autopilot_process::{CommandSpec, Limits, Outcome, Runner};
use serde::Deserialize;

pub struct FixtureAgent {
    program: PathBuf,
    guardian: PathBuf,
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
        Ok(Self { program, guardian })
    }
}

impl Agent for FixtureAgent {
    fn start(&self, request: StepRequest<'_>) -> Result<AgentRun, AgentError> {
        if request.step != Step::Understand {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the fixture agent supports only Understand",
            )
            .into());
        }
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
            cwd: None,
        };
        let input = serde_json::to_vec(request.ticket)?;
        let runner = Runner::new(self.guardian.clone());
        Ok(AgentRun::spawn(move |context| {
            run_fixture(&runner, &spec, &input, &context)
        })?)
    }
}

fn run_fixture(
    runner: &Runner,
    spec: &CommandSpec,
    input: &[u8],
    context: &AgentContext,
) -> Result<Understanding, AgentError> {
    let decoder = Arc::new(Mutex::new(Decoder::default()));
    let stdout = Arc::clone(&decoder);
    let activity = context.clone();
    let output = runner.run_with_stdout(
        spec,
        input,
        Limits {
            timeout: Duration::from_secs(10),
            bytes_per_stream: 64 * 1024,
        },
        context.cancel_flag(),
        move |chunk| {
            if let Ok(mut decoder) = stdout.lock() {
                decoder.push(chunk, &mut |summary| activity.message(summary));
            }
        },
    )?;
    let decoder = Arc::into_inner(decoder)
        .ok_or_else(|| io::Error::other("fixture output reader did not release its decoder"))?
        .into_inner()
        .map_err(|_| io::Error::other("fixture decoder panicked"))?;
    complete_run(decoder, &output.outcome, &mut |summary| {
        context.message(summary);
    })
}

fn complete_run(
    decoder: Decoder,
    outcome: &Outcome,
    report: &mut impl FnMut(String),
) -> Result<Understanding, AgentError> {
    match outcome {
        // Only a clean exit makes an undelimited tail eligible to be a frame.
        Outcome::Exited(status) if status.success() => decoder.finish(report),
        Outcome::Exited(status) => {
            Err(io::Error::other(format!("synthetic agent exited with {status}")).into())
        }
        Outcome::TimedOut => {
            Err(io::Error::new(io::ErrorKind::TimedOut, "synthetic agent timed out").into())
        }
        Outcome::Cancelled => {
            Err(io::Error::new(io::ErrorKind::Interrupted, "synthetic agent was cancelled").into())
        }
        Outcome::OutputLimit => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "synthetic agent exceeded its output limit",
        )
        .into()),
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

#[derive(Default)]
struct Decoder {
    pending: Vec<u8>,
    result: Option<Understanding>,
    error: Option<AgentError>,
}

impl Decoder {
    fn push(&mut self, bytes: &[u8], report: &mut impl FnMut(String)) {
        let mut line = std::mem::take(&mut self.pending);
        for part in bytes.split_inclusive(|&byte| byte == b'\n') {
            line.extend_from_slice(part);
            if part.ends_with(b"\n") {
                self.read_line(&line, report);
                line.clear();
            }
        }
        self.pending = line;
    }

    fn read_line(&mut self, line: &[u8], report: &mut impl FnMut(String)) {
        if line.trim_ascii().is_empty() {
            return;
        }
        match serde_json::from_slice(line) {
            Ok(Frame::Activity { summary }) => report(summary),
            Ok(Frame::Result { result }) if self.result.is_none() => self.result = Some(result),
            Ok(Frame::Result { .. }) => {
                self.error.get_or_insert_with(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "fixture sent more than one result",
                    )
                    .into()
                });
            }
            Ok(Frame::Unknown) => {}
            Err(error) => {
                self.error.get_or_insert_with(|| error.into());
            }
        }
    }

    fn finish(mut self, report: &mut impl FnMut(String)) -> Result<Understanding, AgentError> {
        let tail = std::mem::take(&mut self.pending);
        self.read_line(&tail, report);
        if let Some(error) = self.error {
            return Err(error);
        }
        self.result.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "fixture ended without a result",
            )
            .into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../../fixtures/agent/understand.jsonl");

    fn decode(input: &[u8]) -> (Vec<String>, Result<Understanding, AgentError>) {
        let mut decoder = Decoder::default();
        let mut activity = Vec::new();
        let mut report = |summary| activity.push(summary);
        decoder.push(input, &mut report);
        let result = decoder.finish(&mut report);
        (activity, result)
    }

    #[test]
    fn a_fixture_requires_one_complete_result() {
        let (activity, result) = decode(FIXTURE);
        assert_eq!(activity.len(), 1);
        assert!(result.is_ok());
        let end = FIXTURE.iter().rposition(|&byte| byte == b'}').unwrap();
        let (prefix, result) = decode(&FIXTURE[..end]);
        assert_eq!(prefix, activity);
        assert!(result.is_err());
        assert!(decode(&[FIXTURE, FIXTURE].concat()).1.is_err());
    }

    #[test]
    fn a_split_character_is_buffered_and_complete_activity_is_delivered_immediately() {
        let mut decoder = Decoder::default();
        let mut activity = Vec::new();
        decoder.push(b"{\"type\":\"activity\",\"summary\":\"\xe2", &mut |_| {
            panic!("the frame is incomplete")
        });
        decoder.push(b"\x82\xac\"}\n", &mut |summary| activity.push(summary));
        assert_eq!(activity, ["€"]);
        assert!(decoder.finish(&mut |_| {}).is_err());
    }

    #[test]
    fn a_corrupt_frame_does_not_hide_complete_activity_on_either_side() {
        let activity = b"{\"type\":\"activity\",\"summary\":\"still readable\"}\n";
        let mut corrupt = activity.to_vec();
        corrupt[0] ^= 0x80;
        let (messages, result) =
            decode(&[activity.as_slice(), &corrupt, activity, FIXTURE].concat());
        assert_eq!(&messages[..2], ["still readable", "still readable"]);
        assert!(result.is_err());
    }

    #[test]
    fn the_output_limit_does_not_turn_an_undelimited_prefix_into_a_frame() {
        let mut decoder = Decoder::default();
        let mut activity = Vec::new();
        let mut report = |summary| activity.push(summary);
        decoder.push(
            b"{\"type\":\"activity\",\"summary\":\"complete\"}\n",
            &mut report,
        );
        decoder.push(
            b"{\"type\":\"activity\",\"summary\":\"could be truncated\"}",
            &mut report,
        );
        assert!(complete_run(decoder, &Outcome::OutputLimit, &mut report).is_err());
        assert_eq!(activity, ["complete"]);
    }
}
