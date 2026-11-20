mod layers;

use std::{
    env,
    error::Error,
    fs, io,
    process::{Command, ExitCode},
};

type TaskResult = Result<(), Box<dyn Error>>;

fn main() -> ExitCode {
    let result = match env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [task] if task == "layers" => layers::check(),
        [task] if task == "ci" => ci(),
        [task] if task == "secrets" => {
            run("gitleaks", &["git", "--staged", "--redact", "--no-banner"])
        }
        _ => Err("usage: cargo xtask <ci|layers|secrets>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn ci() -> TaskResult {
    if !fs::metadata("AGENTS.md")?.is_file()
        || fs::read_to_string("CLAUDE.md")?.trim_end() != "@AGENTS.md"
    {
        return Err("CLAUDE.md must import AGENTS.md".into());
    }
    layers::check()?;
    for args in [
        &["fmt", "--all", "--", "--check"][..],
        &[
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        &["nextest", "run", "--locked", "--workspace"],
        &["deny", "--locked", "check"],
    ] {
        run("cargo", args)?;
    }
    // Cargo's inherited package variables make machete interpret the subcommand as a path.
    run("cargo-machete", &["--skip-target-dir", "."])
}

fn run(program: &str, args: &[&str]) -> TaskResult {
    let status = Command::new(program).args(args).status().map_err(|error| {
        io::Error::new(error.kind(), format!("could not start {program}: {error}"))
    })?;
    if !status.success() {
        return Err(format!("{program} {} exited with {status}", args.join(" ")).into());
    }
    Ok(())
}
