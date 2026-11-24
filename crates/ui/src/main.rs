// A release build is a window app: without this Windows opens a console beside
// it. Debug builds keep the console so `cargo run` still shows messages.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! `autopilot-ui [--data-dir <dir>] [--profile <file>] [--demo] [--view <screen>] [--ticket <id>]`

use std::{env, path::PathBuf, process::ExitCode};

use autopilot_ui::{Options, demo};

const USAGE: &str = "usage: autopilot-ui [--data-dir <dir>] [--profile <file>] [--demo]\n\
  --data-dir  where tickets are kept (default: ./autopilot-data)\n\
  --profile   the project profile (default: ./profile.toml)\n\
  --demo      add sample tickets for every state so every screen has content. Without --data-dir it
              uses ./autopilot-demo-data, never the default folder\n\
  --view      open on today, new, plan, board, projects, pipelines, health, review, settings, design, checks, look, cost, accounts, run or step (plan, run and step use --ticket, default the newest)";

fn main() -> ExitCode {
    let mut data_dir = PathBuf::from("autopilot-data");
    let mut profile = PathBuf::from("profile.toml");
    let mut sample = false;
    let mut chosen_dir = false;
    let mut view = None;
    let mut ticket = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                chosen_dir = true;
                data_dir = args.next().map(PathBuf::from).unwrap_or(data_dir);
            }
            "--profile" => profile = args.next().map(PathBuf::from).unwrap_or(profile),
            "--demo" => sample = true,
            "--view" => view = args.next(),
            "--ticket" => ticket = args.next(),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    // Sample tickets never go into the real data folder by accident.
    if sample && !chosen_dir {
        data_dir = PathBuf::from("autopilot-demo-data");
    }
    if sample && let Err(error) = demo::seed(&data_dir, autopilot_ui::now_ms()) {
        eprintln!("autopilot-ui: could not write the sample tickets: {error}");
        return ExitCode::FAILURE;
    }
    if sample && let Err(error) = demo::seed_build_states(&data_dir, autopilot_ui::now_ms()) {
        eprintln!("autopilot-ui: could not write the sample tickets: {error}");
        return ExitCode::FAILURE;
    }
    match autopilot_ui::run(Options {
        data_dir,
        profile,
        view,
        ticket,
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("autopilot-ui: {error}");
            ExitCode::FAILURE
        }
    }
}
