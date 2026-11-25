//! Open a terminal window in a folder, for the owner to look at a worktree by
//! hand. The window crate may not start programs itself, so this is its way.

use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

/// One way to start a terminal. The folder is an argument or the working
/// directory and never part of a command string: a folder named `a & b` must
/// stay one folder, so nothing here goes through a shell.
#[derive(Debug, Eq, PartialEq)]
struct Launch {
    program: PathBuf,
    args: Vec<PathBuf>,
    /// Start the program in the folder, for terminals that open in their own
    /// working directory.
    cwd: Option<PathBuf>,
}

/// Open a terminal in `folder`. The program is started on its own, outside the
/// supervised job a step runs in, so closing the window never touches a run.
///
/// # Errors
/// Returns a sentence if `folder` is not a folder or no terminal could start.
pub fn open_terminal(folder: &Path) -> Result<(), String> {
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let mut last = None;
    for launch in launches(folder) {
        match start(&launch) {
            Ok(()) => return Ok(()),
            // A missing terminal is expected on some systems: try the next.
            Err(error) => last = Some(error),
        }
    }
    Err(match last {
        Some(error) => format!("could not open a terminal in {}: {error}", folder.display()),
        None => format!(
            "no terminal is known for this system ({})",
            folder.display()
        ),
    })
}

fn launches(folder: &Path) -> Vec<Launch> {
    if cfg!(windows) {
        let root = std::env::var_os("SYSTEMROOT")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        vec![Launch {
            program: root.join("System32").join("cmd.exe"),
            args: Vec::new(),
            cwd: Some(folder.to_path_buf()),
        }]
    } else if cfg!(target_os = "macos") {
        vec![Launch {
            program: PathBuf::from("/usr/bin/open"),
            args: vec![
                PathBuf::from("-a"),
                PathBuf::from("Terminal"),
                folder.to_path_buf(),
            ],
            cwd: None,
        }]
    } else {
        ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"]
            .into_iter()
            .map(|name| Launch {
                program: PathBuf::from(name),
                args: Vec::new(),
                cwd: Some(folder.to_path_buf()),
            })
            .collect()
    }
}

fn start(launch: &Launch) -> io::Result<()> {
    let mut command = Command::new(&launch.program);
    command.args(&launch.args);
    if let Some(cwd) = &launch.cwd {
        command.current_dir(cwd);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without its own console, cmd.exe would share ours, which the window
        // does not have.
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    command.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invariant: only a real folder is opened. A change that breaks it: dropping
    // the check, so a path from a deleted worktree starts a terminal in the
    // wrong place (the system falls back to the home folder).
    #[test]
    fn a_missing_folder_or_a_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("autopilot-terminal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        let file = dir.join("a-file");
        std::fs::write(&file, "x").expect("file");
        let missing = dir.join("gone");
        assert!(
            open_terminal(&missing)
                .expect_err("missing")
                .contains("not a folder")
        );
        assert!(
            open_terminal(&file)
                .expect_err("file")
                .contains("not a folder")
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    // Invariant: the folder reaches the terminal as one argument or as its
    // working directory, whatever characters it holds. A change that breaks it:
    // building one command string for a shell, where `&` or a quote in a
    // folder name starts a second command.
    #[test]
    fn the_folder_is_never_part_of_a_command_string() {
        let folder = Path::new("some dir/a & b \"c\"");
        let found = launches(folder);
        assert_ne!(found.len(), 0, "no launch for this system");
        for launch in found {
            let whole = launch.cwd.as_deref() == Some(folder);
            let one_arg = launch.args.iter().any(|arg| arg == folder);
            assert!(
                whole || one_arg,
                "{launch:?} does not carry the folder whole"
            );
            assert!(
                launch
                    .args
                    .iter()
                    .all(|arg| arg == folder || !arg.to_string_lossy().contains('&')),
                "{launch:?}"
            );
        }
    }
}
