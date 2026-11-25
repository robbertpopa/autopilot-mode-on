//! A stand-in for the `gh` CLI in tests. It keeps its world in the JSON file
//! named by `FAKE_GH_STATE` and appends every call to `calls`, so a test can
//! check what was asked and what environment `gh` was given.
//!
//! State shape: `issues` (by number), `heads` (branch to commit), `prs`, `runs`
//! (check runs by commit), `create` (`"ok"`, `"fail"`, or `"fail_after"`, which
//! makes the pull request and then exits non-zero), `logs` (job log text by job
//! id), and `calls`.
//!
//! A pull request may also carry the fields real `gh` reports: `state` (default
//! `OPEN`), `mergeable` (`MERGEABLE`, `CONFLICTING` or `UNKNOWN`),
//! `reviewDecision`, and `threads`, each `{isResolved, comments: [{author,
//! authorAssociation, body}]}`. A pull request may carry `title` and `diff`
//! (what `pr diff` prints), and `headAfterDiff`, the commit its branch moves to
//! once the diff has been read. Check runs are passed through as written, so a
//! run may hold `output.text` as its log tail. Another person pushing to the
//! branch is a change to `heads` in the state file; `pr view` reads it live.

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

/// Looks a pull request up by number or by branch name, like `gh pr view` does.
fn pr_by_selector(state: &Value, selector: &str) -> Result<Option<Value>, &'static str> {
    let prs = state["prs"].as_array().ok_or("prs")?;
    Ok(prs
        .iter()
        .find(|pr| pr["number"] == selector.parse::<u64>().unwrap_or(0) || pr["branch"] == selector)
        .cloned())
}

/// The review threads the way the GraphQL API nests them.
fn thread_nodes(pr: &Value) -> Vec<Value> {
    let mut nodes = Vec::new();
    for thread in pr["threads"].as_array().into_iter().flatten() {
        let mut comments = Vec::new();
        for c in thread["comments"].as_array().into_iter().flatten() {
            comments.push(json!({"author": {"login": c["author"]},
                                 "authorAssociation": c["authorAssociation"],
                                 "body": c["body"]}));
        }
        nodes.push(json!({"isResolved": thread["isResolved"],
                          "comments": {"nodes": comments}}));
    }
    nodes
}

fn pr_view(state: &Value, pr: &Value) -> Value {
    let branch = pr["branch"].as_str().unwrap_or_default();
    // Read live: someone else pushing moves the branch without touching the record.
    let head = state["heads"].get(branch).unwrap_or(&pr["headRefOid"]);
    json!({
        "number": pr["number"], "url": pr["url"], "headRefOid": head,
        "isDraft": pr["isDraft"], "body": pr["body"], "title": pr["title"],
        "headRefName": pr["branch"], "baseRefName": pr["base"],
        "state": pr.get("state").unwrap_or(&json!("OPEN")),
        "mergeable": pr.get("mergeable").unwrap_or(&json!("MERGEABLE")),
        "reviewDecision": pr.get("reviewDecision").unwrap_or(&json!("")),
    })
}

type Fail = Box<dyn std::error::Error>;

fn pr_view_command(state: &Value, args: &[String], out: &mut impl Write) -> Result<ExitCode, Fail> {
    let Some(pr) = pr_by_selector(state, args.get(2).ok_or("pull request")?)? else {
        eprintln!("no pull request found");
        return Ok(ExitCode::from(1));
    };
    let view = pr_view(state, &pr);
    let shown = match flag(args, "--json") {
        Some(fields) => fields
            .split(',')
            .map(|field| (field.to_owned(), view[field].clone()))
            .collect(),
        None => view,
    };
    writeln!(out, "{shown}")?;
    Ok(ExitCode::SUCCESS)
}

/// Prints the pull request's `diff` text. A pull request that carries
/// `headAfterDiff` moves its branch to that commit once the diff has been
/// read, the way another person pushing during a review would.
fn pr_diff_command(
    state: &mut Value,
    args: &[String],
    out: &mut impl Write,
) -> Result<ExitCode, Fail> {
    let Some(pr) = pr_by_selector(state, args.get(2).ok_or("pull request")?)? else {
        eprintln!("no pull request found");
        return Ok(ExitCode::from(1));
    };
    write!(out, "{}", pr["diff"].as_str().unwrap_or_default())?;
    if let (Some(branch), Some(next)) = (pr["branch"].as_str(), pr.get("headAfterDiff")) {
        state["heads"][branch] = next.clone();
    }
    Ok(ExitCode::SUCCESS)
}

fn graphql_command(state: &Value, args: &[String], out: &mut impl Write) -> Result<ExitCode, Fail> {
    let number = args
        .iter()
        .find_map(|arg| arg.strip_prefix("number="))
        .ok_or("number=")?;
    let pr = pr_by_selector(state, number)?.ok_or("no such pull request")?;
    let nodes = thread_nodes(&pr);
    writeln!(
        out,
        "{}",
        json!({"data": {"repository": {"pullRequest": {"reviewThreads": {"nodes": nodes}}}}})
    )?;
    Ok(ExitCode::SUCCESS)
}

fn log_command(state: &Value, route: &str, out: &mut impl Write) -> Result<ExitCode, Fail> {
    let id = route
        .strip_suffix("/logs")
        .and_then(|r| r.rsplit('/').next())
        .ok_or("logs route")?;
    let Some(text) = state["logs"].get(id).and_then(Value::as_str) else {
        eprintln!("no log for job {id}");
        return Ok(ExitCode::from(1));
    };
    write!(out, "{text}")?;
    Ok(ExitCode::SUCCESS)
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
        (Some("pr"), Some("view")) => pr_view_command(&state, &args, &mut out)?,
        (Some("pr"), Some("comment")) => pr_comment_command(&mut state, &args)?,
        (Some("pr"), Some("diff")) => pr_diff_command(&mut state, &args, &mut out)?,
        (Some("api"), Some("graphql")) => graphql_command(&state, &args, &mut out)?,
        (Some("api"), Some(route)) if route.ends_with("/logs") => {
            log_command(&state, route, &mut out)?
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

/// Records the comment text from stdin under `comments`; nothing is posted
/// anywhere, so a test can read exactly what would have been.
fn pr_comment_command(state: &mut Value, args: &[String]) -> Result<ExitCode, Fail> {
    let mut body = String::new();
    io::stdin().read_to_string(&mut body)?;
    let number = args.get(2).ok_or("pull request")?;
    let number = number.parse::<u64>().unwrap_or(0);
    if state.get("comments").is_none() {
        state["comments"] = json!([]);
    }
    state["comments"]
        .as_array_mut()
        .ok_or("comments")?
        .push(json!({"number": number, "body": body}));
    Ok(ExitCode::SUCCESS)
}
