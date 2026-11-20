use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use autopilot_process::{CommandSpec, Limits, Outcome, Runner};

const GUARDIAN: &str = env!("CARGO_BIN_EXE_autopilot-process-guardian");
const BASE_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(15),
    bytes_per_stream: 8192,
};

fn runner() -> Runner {
    Runner::new(PathBuf::from(GUARDIAN))
}

fn fixture(mode: &str, data: &Path) -> io::Result<CommandSpec> {
    let mut env = vec![
        ("AUTOPILOT_TEST_MODE".into(), mode.into()),
        ("AUTOPILOT_TEST_DATA".into(), data.as_os_str().into()),
        ("AUTOPILOT_TEST_GUARDIAN".into(), GUARDIAN.into()),
    ];
    if let Some(root) = env::var_os("SystemRoot") {
        env.push(("SystemRoot".into(), root));
    }
    Ok(CommandSpec {
        program: env::current_exe()?,
        args: ["--ignored", "--exact", "fake_child", "--nocapture"]
            .map(Into::into)
            .to_vec(),
        env,
    })
}

fn pids(data: &Path) -> io::Result<Vec<u32>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(marker) = fs::read_to_string(data.join("ready")) {
            return marker
                .lines()
                .map(|pid| pid.parse().map_err(io::Error::other))
                .collect();
        }
        assert!(
            Instant::now() < deadline,
            "synthetic process tree never started"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_dead(pids: &[u32]) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while pids.iter().copied().any(alive) {
        assert!(
            Instant::now() < deadline,
            "synthetic process still running: {pids:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(") ")
                .and_then(|(_, tail)| tail.as_bytes().first().copied())
        })
        .is_some_and(|state| !matches!(state, b'Z' | b'X'))
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_probe {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::{
            Console::GetConsoleWindow,
            Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        },
    };

    pub fn alive(pid: u32) -> bool {
        // SAFETY: An owned synchronization handle is closed after a zero-duration wait.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return false;
            }
            let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
            CloseHandle(handle);
            running
        }
    }

    pub fn no_console() -> bool {
        // SAFETY: This query takes no pointers and does not modify the console.
        unsafe { GetConsoleWindow().is_null() }
    }
}

#[cfg(windows)]
use windows_probe::alive;

#[test]
fn normal_exit_closes_stdin_drains_both_streams_and_cleans_up_descendants() {
    let data = tempfile::tempdir().unwrap();
    let input = vec![b'x'; 64 * 1024];
    let output = runner()
        .run(
            &fixture("finish", data.path()).unwrap(),
            &input,
            BASE_LIMITS,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(matches!(output.outcome, Outcome::Exited(status) if status.success()));
    assert!(String::from_utf8_lossy(&output.stdout).contains("stdin bytes: 65536"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("stderr reached"));
    assert_dead(&pids(data.path()).unwrap());
}

#[test]
fn cancellation_and_timeout_end_the_worker_and_its_grandchild() {
    for timeout in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let spec = fixture("hold", data.path()).unwrap();
        let limits = Limits {
            timeout: if timeout {
                Duration::from_secs(2)
            } else {
                BASE_LIMITS.timeout
            },
            ..BASE_LIMITS
        };
        let (pids, output) = thread::scope(|scope| {
            let task = scope.spawn(|| runner().run(&spec, b"ignored", limits, &cancel).unwrap());
            let pids = pids(data.path()).unwrap();
            assert!(
                pids.iter().copied().all(alive),
                "worker tree must be running before stopping it"
            );
            if !timeout {
                cancel.store(true, Ordering::Release);
            }
            (pids, task.join().unwrap())
        });
        assert!(matches!(
            (timeout, output.outcome),
            (true, Outcome::TimedOut) | (false, Outcome::Cancelled)
        ));
        assert_dead(&pids);
    }
}

#[test]
fn abrupt_supervisor_exit_cleans_up_the_process_tree() {
    let data = tempfile::tempdir().unwrap();
    let spec = fixture("supervisor", data.path()).unwrap();
    let mut supervisor = synthetic_command(spec.program)
        .args(spec.args)
        .envs(spec.env)
        .spawn()
        .unwrap();
    let pids = pids(data.path()).unwrap();
    assert!(pids.iter().copied().all(alive));
    supervisor.kill().unwrap();
    assert!(!supervisor.wait().unwrap().success());
    assert_dead(&pids);
}

#[test]
fn flooding_both_pipes_hits_the_limit_instead_of_blocking() {
    let data = tempfile::tempdir().unwrap();
    let output = runner()
        .run(
            &fixture("flood", data.path()).unwrap(),
            b"",
            BASE_LIMITS,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(matches!(output.outcome, Outcome::OutputLimit));
    assert!(output.stdout.len() <= BASE_LIMITS.bytes_per_stream);
    assert!(output.stderr.len() <= BASE_LIMITS.bytes_per_stream);
}

#[test]
fn a_missing_worker_is_reported_as_a_launch_error() {
    let data = tempfile::tempdir().unwrap();
    let mut spec = fixture("hold", data.path()).unwrap();
    spec.program = data.path().join("missing-executable.exe");
    assert!(
        runner()
            .run(&spec, b"", BASE_LIMITS, &AtomicBool::new(false))
            .is_err()
    );
    assert!(!data.path().join("ready").exists());
}

#[test]
#[ignore = "invoked by lifecycle tests"]
fn fake_child() {
    let Some(mode) = env::var_os("AUTOPILOT_TEST_MODE") else {
        return;
    };
    let path = PathBuf::from(env::var_os("AUTOPILOT_TEST_DATA").unwrap());
    match mode.to_str().unwrap() {
        "supervisor" => {
            let guardian = PathBuf::from(env::var_os("AUTOPILOT_TEST_GUARDIAN").unwrap());
            let spec = fixture("hold", &path).unwrap();
            Runner::new(guardian)
                .run(&spec, b"", BASE_LIMITS, &AtomicBool::new(false))
                .unwrap();
        }
        "finish" | "hold" => {
            let spec = fixture("grandchild", &path).unwrap();
            #[allow(
                clippy::zombie_processes,
                reason = "The test supervisor must clean up the synthetic grandchild after its parent exits."
            )]
            let grandchild = synthetic_command(spec.program)
                .args(spec.args)
                .envs(spec.env)
                .spawn()
                .unwrap();
            fs::write(
                path.join("partial"),
                format!("{}\n{}\n", process::id(), grandchild.id()),
            )
            .unwrap();
            fs::rename(path.join("partial"), path.join("ready")).unwrap();
            if mode == "finish" {
                #[cfg(windows)]
                assert!(windows_probe::no_console(), "child acquired a console");
                let mut input = Vec::new();
                io::stdin().read_to_end(&mut input).unwrap();
                writeln!(io::stdout(), "stdin bytes: {}", input.len()).unwrap();
                writeln!(io::stderr(), "stderr reached").unwrap();
            } else {
                thread::sleep(Duration::from_secs(20));
            }
        }
        "grandchild" => thread::sleep(Duration::from_secs(20)),
        "flood" => {
            let chunk = [b'x'; 4096];
            loop {
                if io::stdout().write_all(&chunk).is_err()
                    || io::stderr().write_all(&chunk).is_err()
                {
                    break;
                }
            }
        }
        other => panic!("unknown synthetic mode {other}"),
    }
}

fn synthetic_command(program: PathBuf) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}
