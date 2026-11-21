mod claude;

use std::{
    io::{self, Write},
    process::{Command, Output, Stdio},
};

fn run_fault(fault: &str) -> io::Result<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_autopilot-fake-agent"));
    command
        .args(["replay", fault])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing child stdin"))?
        .write_all(include_bytes!(
            "../../../../fixtures/tickets/understand.json"
        ))?;
    child.wait_with_output()
}

#[test]
fn malformed_faults_leave_the_first_frame_complete_and_the_tail_invalid() {
    for fault in ["truncated", "partial-utf8"] {
        let output = run_fault(fault).unwrap();
        assert!(output.status.success());
        let first = output
            .stdout
            .iter()
            .position(|&byte| byte == b'\n')
            .unwrap();
        let activity: serde_json::Value = serde_json::from_slice(&output.stdout[..first]).unwrap();
        assert_eq!(activity["type"], "activity");
        if fault == "partial-utf8" {
            assert!(std::str::from_utf8(&output.stdout[first + 1..]).is_err());
        } else {
            let final_frame = output
                .stdout
                .split(|&byte| byte == b'\n')
                .next_back()
                .unwrap();
            let error = serde_json::from_slice::<serde_json::Value>(final_frame).unwrap_err();
            assert!(error.is_eof());
        }
    }
}

#[test]
fn an_oversized_fault_can_cross_the_fixture_adapter_output_limit() {
    let output = run_fault("oversized").unwrap();
    assert!(output.status.success());
    assert!(output.stdout.len() > 64 * 1024);
}

#[test]
fn an_abnormal_exit_keeps_its_prefix_and_exits_unsuccessfully() {
    let output = run_fault("crash").unwrap();
    assert_eq!(output.status.code(), Some(23));
    let activity: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(activity["type"], "activity");
}
