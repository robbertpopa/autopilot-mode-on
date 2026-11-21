//! A stand-in for the `gh` CLI in tests. It keeps its world in the JSON file
//! named by `FAKE_GH_STATE` and appends every call to `calls`, so a test can
//! check what was asked and what environment `gh` was given.
//!
//! State shape: `issues` (by number), `heads` (branch to commit), `prs`, `runs`
//! (check runs by commit), `create` (`"ok"`, `"fail"`, or `"fail_after"`, which
//! makes the pull request and then exits non-zero), and `calls`.

use std::{
    env, fs,
    io::{self, Read, Write},
    process::ExitCode,
};

use serde_json::{Value, json};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("fake gh: {error}");
            ExitCode::from(2)
        }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    // The state file is named directly, or sits in gh's config folder, which
    // is the one variable the tracker passes on to gh.
    let path = env::var("FAKE_GH_STATE").or_else(|_| {
        env::var("GH_CONFIG_DIR").map(|directory| format!("{directory}/fake-gh-state.json"))
    })?;
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    let leaked: Vec<String> = env::vars()
        .map(|(name, _)| name)
        .filter(|name| name.to_ascii_uppercase().contains("TOKEN"))
        .collect();
    state["calls"]
        .as_array_mut()
        .ok_or("state needs a calls array")?
        .push(json!({"args": args, "token_vars": leaked}));
    let mut out = io::stdout().lock();
    let code = match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("issue"), Some("view")) => {
            let number = args.get(2).ok_or("issue number")?;
            if let Some(issue) = state["issues"].get(number.as_str()) {
                writeln!(out, "{issue}")?;
                ExitCode::SUCCESS
            } else {
                eprintln!("no issue {number}");
                ExitCode::from(1)
            }
        }
        (Some("pr"), Some("list")) => {
            let head = flag(&args, "--head").ok_or("--head")?;
            let found: Vec<&Value> = state["prs"]
                .as_array()
                .ok_or("prs")?
                .iter()
                .filter(|pr| pr["branch"] == head)
                .collect();
            let shown: Vec<Value> = found
                .into_iter()
                .map(|pr| {
                    json!({"number": pr["number"], "url": pr["url"],
                           "headRefOid": pr["headRefOid"], "isDraft": pr["isDraft"]})
                })
                .collect();
            writeln!(out, "{}", Value::Array(shown))?;
            ExitCode::SUCCESS
        }
        (Some("pr"), Some("create")) => {
            let mut body = String::new();
            io::stdin().read_to_string(&mut body)?;
            let branch = flag(&args, "--head").ok_or("--head")?.to_owned();
            let mode = state["create"].as_str().unwrap_or("ok").to_owned();
            if mode == "fail" {
                eprintln!("could not create the pull request");
                ExitCode::from(1)
            } else {
                let number = state["prs"].as_array().ok_or("prs")?.len() + 1;
                let url = format!("https://example.invalid/pull/{number}");
                let oid = state["heads"][branch.as_str()].clone();
                state["prs"].as_array_mut().ok_or("prs")?.push(json!({
                    "number": number, "url": url, "headRefOid": oid, "branch": branch,
                    "isDraft": args.iter().any(|a| a == "--draft"), "body": body,
                    "base": flag(&args, "--base"),
                }));
                if mode == "fail_after" {
                    eprintln!("connection reset after the pull request was made");
                    ExitCode::from(1)
                } else {
                    writeln!(out, "{url}")?;
                    ExitCode::SUCCESS
                }
            }
        }
        (Some("api"), Some(route)) => {
            let sha = route
                .strip_suffix("/check-runs")
                .and_then(|r| r.rsplit('/').next())
                .ok_or("check-runs route")?;
            let runs = state["runs"].get(sha).cloned().unwrap_or_else(|| json!([]));
            writeln!(out, "{}", json!({"check_runs": runs}))?;
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("fake gh does not know {args:?}");
            ExitCode::from(2)
        }
    };
    fs::write(&path, serde_json::to_string(&state)?)?;
    Ok(code)
}
