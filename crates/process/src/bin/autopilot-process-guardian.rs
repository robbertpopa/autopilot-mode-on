#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use std::io::Write;

    match autopilot_process::linux::supervise() {
        Ok(code) => std::process::ExitCode::from(code),
        Err(error) => {
            let _ = writeln!(std::io::stderr().lock(), "guardian: {error}");
            std::process::ExitCode::from(autopilot_process::linux::GUARDIAN_FAILURE)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    std::process::ExitCode::FAILURE
}
