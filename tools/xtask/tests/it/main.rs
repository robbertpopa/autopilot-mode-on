use std::{fs, io, path::Path, process::Command};

fn package(root: &Path, name: &str, dependencies: &str, source: &str) -> io::Result<()> {
    let src = root.join(name).join("src");
    fs::create_dir_all(&src)?;
    fs::write(
        root.join(name).join("Cargo.toml"),
        format!(
            "[package]\nname = \"autopilot-{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n{dependencies}"
        ),
    )?;
    fs::write(src.join("lib.rs"), source)
}

#[test]
fn production_dependencies_must_point_down() {
    // Dev dependencies may cross layers, but a normal dependency must not.
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"core\", \"engine\"]",
    )
    .unwrap();
    package(root, "engine", "", "").unwrap();
    package(
        root,
        "core",
        "[dev-dependencies]\nautopilot-engine = { path = \"../engine\" }",
        "",
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("layers")
            .current_dir(root)
            .output()
            .unwrap()
    };
    assert!(run().status.success());

    package(
        root,
        "core",
        "[dependencies]\nautopilot-engine = { path = \"../engine\" }",
        "",
    )
    .unwrap();
    let output = run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("autopilot-core -> autopilot-engine"));
}

#[test]
fn git_uses_the_process_crate_to_spawn() {
    // A grouped alias still grants direct access to the OS launch API.
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"git\"]",
    )
    .unwrap();
    package(
        root,
        "git",
        "",
        "use std::{path::Path, process::Command as Runner};",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("layers")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("direct subprocess"));

    package(root, "git", "[lib]\npath = \"worker.rs\"", "").unwrap();
    fs::write(root.join("git/worker.rs"), "use std::process::Command;").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("layers")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("worker.rs: direct subprocess"));
}
