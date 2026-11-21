use std::{
    env,
    error::Error,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use autopilot_accounts::{Accounts, Status};
use autopilot_agent_claude::{BuildConfig, ClaudeAgent, ClaudeConfig};
use autopilot_agent_fixture::FixtureAgent;
use autopilot_core::{
    RequestId, SettingsScope, Step as CoreStep, StepChange, StepOverride, Ticket, TicketId,
};
use autopilot_pipeline::{
    StepState, TicketState,
    clarify::Choice,
    fold,
    inbox::{Decision, Inbox, Reply, RequestBody, check_reply},
    summary,
};
use autopilot_profile::Auth;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(about = "Run and replay a local coding-agent fixture")]
struct Cli {
    #[arg(long, global = true, default_value = ".autopilot")]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one synthetic Understand step.
    RunStep {
        #[arg(long)]
        step: Step,
        #[arg(long)]
        ticket: PathBuf,
        #[arg(long)]
        agent: Agent,
        /// How the claude agent is billed. Ignored by the fake agent.
        #[arg(long, default_value = "subscription")]
        auth: AuthArg,
        #[arg(long, default_value = "haiku")]
        model: String,
        /// Structured output needs a turn after the answer, so keep this above 1.
        #[arg(long, default_value_t = 4)]
        max_turns: u32,
        /// Reasoning effort: low, medium, high, xhigh or max. Omit for the model default.
        #[arg(long)]
        effort: Option<String>,
        /// Dollar cap per call. Applies to --auth api only.
        #[arg(long, default_value = "0.05")]
        max_budget_usd: String,
        /// Absolute path to a native claude executable. Defaults to a PATH search.
        #[arg(long)]
        claude_path: Option<PathBuf>,
        /// Wall-clock limit for the claude run, in seconds.
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
        /// Wait in the inbox for approve, edit or reject before the step passes.
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        ask_after: Option<bool>,
    },
    /// Run the Build step for an approved plan in a ticket worktree.
    Build {
        #[arg(long)]
        ticket: PathBuf,
        /// The repository the ticket changes.
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        base: String,
        /// Where ticket checkouts live. Keep it outside the repository.
        #[arg(long)]
        worktrees: PathBuf,
        /// The project profile (TOML) with the check commands.
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        agent: Agent,
    },
    /// Show which Claude, GitHub and Azure accounts this machine is signed in to.
    Accounts {
        /// Print machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Skip Clarify when nothing is open, or ask the open questions through the inbox.
    Clarify { ticket: String },
    /// Look at and answer the requests waiting for you.
    Inbox {
        #[command(subcommand)]
        action: InboxAction,
    },
    /// Mark an attempt left running by a dead process as interrupted.
    Recover { ticket: String },
    /// Change step settings. A change applies when the next step starts.
    Override {
        ticket: String,
        #[arg(long)]
        scope: ScopeArg,
        #[arg(long)]
        step: Step,
        /// Whether to wait for approval after the step.
        #[arg(long)]
        ask_after: Option<bool>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        effort: Option<String>,
    },
    /// Show where each step of a ticket stands, folded from its history.
    Status {
        ticket: String,
        /// Print machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// The agent's tool-call hook: reads one call on standard input and
    /// prints whether it may run. Claude Code starts this, not people.
    #[command(hide = true)]
    Hook,
    /// Replay committed events from the ticket's local history.
    Watch {
        ticket: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
}

#[derive(Subcommand)]
enum InboxAction {
    /// List waiting requests.
    List,
    /// Answer a question request.
    Answer {
        request: String,
        /// Take the recommended option for every question.
        #[arg(long, conflicts_with = "choice")]
        recommended: bool,
        /// One per question, in order: `recommended`, an option number from 0, or `text:<answer>`.
        #[arg(long)]
        choice: Vec<String>,
    },
    /// Answer an approval request.
    Approve {
        request: String,
        #[arg(long, conflicts_with = "edit")]
        reject: bool,
        /// Approve with this replacement text.
        #[arg(long)]
        edit: Option<String>,
        /// Do not ask about this optional confirmation again in the project.
        #[arg(long)]
        allow_for_project: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Step {
    Understand,
    Plan,
}

#[derive(Clone, Copy, ValueEnum)]
enum ScopeArg {
    Ticket,
    Project,
}

#[derive(Clone, Copy, ValueEnum)]
enum Agent {
    Fake,
    Claude,
}

#[derive(Clone, Copy, ValueEnum)]
enum AuthArg {
    Subscription,
    Api,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("autopilot: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let mut out = io::stdout().lock();
    match cli.command {
        Command::RunStep {
            step,
            ticket,
            agent,
            auth,
            model,
            max_turns,
            effort,
            max_budget_usd,
            claude_path,
            timeout_seconds,
            ask_after,
        } => {
            let ticket = read_ticket(&ticket)?;
            let exe = env::current_exe()?;
            let result = match agent {
                Agent::Fake => {
                    let agent = FixtureAgent::beside(&exe)?;
                    run_step(&cli.data_dir, step, &ticket, &agent, ask_after)?
                }
                Agent::Claude => {
                    let config = ClaudeConfig {
                        auth: match auth {
                            AuthArg::Subscription => Auth::Subscription,
                            AuthArg::Api => Auth::Api,
                        },
                        model,
                        effort,
                        max_turns,
                        max_budget_usd: Some(max_budget_usd),
                        executable: claude_path,
                        timeout: Duration::from_secs(timeout_seconds),
                    };
                    let agent = claude_agent(&exe, config)?;
                    run_step(&cli.data_dir, step, &ticket, &agent, ask_after)?
                }
            };
            serde_json::to_writer(
                &mut out,
                &serde_json::json!({"ticket": ticket.id, "result": result}),
            )?;
            writeln!(out)?;
        }
        Command::Build {
            ticket,
            repo,
            base,
            worktrees,
            profile,
            agent,
        } => {
            let places = BuildPlaces {
                repo: &repo,
                base: &base,
                worktrees: &worktrees,
            };
            let outcome = build_command(&cli.data_dir, &ticket, &profile, agent, &places)?;
            serde_json::to_writer(&mut out, &outcome)?;
            writeln!(out)?;
        }
        Command::Hook => run_hook(&mut out)?,
        Command::Accounts { json } => write_accounts_command(&mut out, json)?,
        Command::Clarify { ticket } => run_clarify(&mut out, &cli.data_dir, ticket)?,
        Command::Recover { ticket } => {
            let steps = autopilot_engine::recover(&cli.data_dir, &TicketId::from(ticket))?;
            write_recovered(&mut out, &steps)?;
        }
        Command::Override {
            ticket,
            scope,
            step,
            ask_after,
            model,
            effort,
        } => {
            let change = StepOverride {
                ask_after,
                model,
                effort,
            };
            run_override(&mut out, &cli.data_dir, &ticket, scope, step, change)?;
        }
        Command::Inbox { action } => run_inbox(&mut out, &cli.data_dir, action)?,
        Command::Status { ticket, json } => {
            let events = autopilot_engine::watch(&cli.data_dir, &TicketId::from(ticket), 0)?;
            let state = fold(&events)?;
            write_status(&mut out, &state, &summary::from_history(&events), json)?;
        }
        Command::Watch { ticket, after } => {
            for record in autopilot_engine::watch(&cli.data_dir, &TicketId::from(ticket), after)? {
                serde_json::to_writer(&mut out, &record)?;
                writeln!(out)?;
            }
        }
    }
    Ok(())
}

fn run_override(
    out: &mut impl Write,
    data_dir: &Path,
    ticket: &str,
    scope: ScopeArg,
    step: Step,
    change: StepOverride,
) -> Result<(), Box<dyn Error>> {
    let scope = match scope {
        ScopeArg::Ticket => SettingsScope::Ticket,
        ScopeArg::Project => SettingsScope::Project,
    };
    let step = match step {
        Step::Understand => CoreStep::Understand,
        Step::Plan => CoreStep::Plan,
    };
    autopilot_engine::override_settings(
        data_dir,
        &TicketId::from(ticket),
        scope,
        &[StepChange { step, change }],
    )?;
    writeln!(out, "{}", serde_json::json!({"recorded": true}))?;
    Ok(())
}

fn run_step(
    data_dir: &Path,
    step: Step,
    ticket: &Ticket,
    agent: &impl autopilot_agent::Agent,
    ask_after: Option<bool>,
) -> Result<serde_json::Value, Box<dyn Error>> {
    Ok(match step {
        Step::Understand => serde_json::to_value(autopilot_engine::run_understand_with(
            data_dir, ticket, agent, ask_after,
        )?)?,
        Step::Plan => serde_json::to_value(autopilot_engine::run_plan(data_dir, ticket, agent)?)?,
    })
}

struct BuildPlaces<'a> {
    repo: &'a Path,
    base: &'a str,
    worktrees: &'a Path,
}

fn run_hook(out: &mut impl Write) -> Result<(), Box<dyn Error>> {
    // A bounded read: the hook must answer even for a hostile call.
    let mut input = Vec::new();
    io::stdin().lock().take(1 << 20).read_to_end(&mut input)?;
    let verdict = autopilot_policy::hook::decide(&input);
    writeln!(out, "{}", autopilot_policy::hook::response(&verdict))?;
    Ok(())
}

fn build_command(
    data_dir: &Path,
    ticket: &Path,
    profile: &Path,
    agent: Agent,
    places: &BuildPlaces<'_>,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let ticket = read_ticket(ticket)?;
    let source = std::fs::read_to_string(profile)?;
    let profile = autopilot_profile::Profile::parse(profile, &source)?;
    let exe = env::current_exe()?;
    run_build(data_dir, &ticket, agent, &exe, places, &profile)
}

/// The fake agent stands in for Claude Code here and is the only agent a
/// build may use: a real one would run tools in a real repository, and S6 has
/// not shown how to contain that.
fn run_build(
    data_dir: &Path,
    ticket: &Ticket,
    agent: Agent,
    exe: &Path,
    places: &BuildPlaces<'_>,
    profile: &autopilot_profile::Profile,
) -> Result<serde_json::Value, Box<dyn Error>> {
    if matches!(agent, Agent::Claude) {
        return Err(
            "a real agent cannot run the Build step until the containment spike S6 \
                    is settled (docs/spikes/S6-containment.md); use --agent fake"
                .into(),
        );
    }
    let guardian = exe.with_file_name(format!(
        "autopilot-process-guardian{}",
        env::consts::EXE_SUFFIX
    ));
    let fake = exe.with_file_name(format!("autopilot-fake-agent{}", env::consts::EXE_SUFFIX));
    let config = ClaudeConfig {
        auth: Auth::Subscription,
        model: "fake".into(),
        effort: None,
        max_turns: 8,
        max_budget_usd: None,
        executable: Some(fake),
        timeout: Duration::from_secs(120),
    };
    let agent = ClaudeAgent::new(config, guardian.clone())?.with_build(BuildConfig {
        hook_command: format!("\"{}\" hook", exe.display()),
        data_dir: data_dir.to_path_buf(),
    });
    let setup = autopilot_engine::BuildSetup {
        repository: places.repo,
        base_branch: places.base,
        worktrees: places.worktrees,
        git: PathBuf::from("git"),
        guardian,
        profile,
    };
    let outcome = autopilot_engine::run_build(
        data_dir,
        ticket,
        &agent,
        &setup,
        autopilot_engine::Isolation::Fixture,
    )?;
    Ok(match outcome {
        autopilot_engine::BuildOutcome::Ready => serde_json::json!({"build": "ready"}),
        autopilot_engine::BuildOutcome::NeedsYou { request, reason } => {
            serde_json::json!({"build": "needs_you", "request": request, "reason": reason})
        }
    })
}

fn claude_agent(exe: &Path, config: ClaudeConfig) -> Result<ClaudeAgent, Box<dyn Error>> {
    let guardian = exe.with_file_name(format!(
        "autopilot-process-guardian{}",
        env::consts::EXE_SUFFIX
    ));
    let agent = ClaudeAgent::new(config, guardian)?;
    agent
        .preflight()
        .map_err(|error| -> Box<dyn Error> { error })?;
    Ok(agent)
}

fn write_accounts_command(out: &mut impl Write, json: bool) -> Result<(), Box<dyn Error>> {
    let exe = env::current_exe()?;
    let guardian = exe.with_file_name(format!(
        "autopilot-process-guardian{}",
        env::consts::EXE_SUFFIX
    ));
    let accounts = autopilot_accounts::collect(guardian);
    if json {
        serde_json::to_writer_pretty(&mut *out, &accounts)?;
        writeln!(out)?;
    } else {
        write_accounts(out, &accounts)?;
    }
    Ok(())
}

fn write_recovered(out: &mut impl Write, steps: &[CoreStep]) -> io::Result<()> {
    let names: Vec<_> = steps.iter().map(|step| step.as_str()).collect();
    writeln!(out, "{}", serde_json::json!({"interrupted": names}))
}

fn run_clarify(
    out: &mut impl Write,
    data_dir: &Path,
    ticket: String,
) -> Result<(), Box<dyn Error>> {
    let value = match autopilot_engine::start_clarify(data_dir, &TicketId::from(ticket))? {
        autopilot_engine::ClarifyStart::Skipped(reason) => {
            serde_json::json!({"clarify": "skipped", "reason": reason})
        }
        autopilot_engine::ClarifyStart::Asking { request, questions } => {
            serde_json::json!({"clarify": "asking", "request": request, "questions": questions})
        }
    };
    serde_json::to_writer(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn run_inbox(
    out: &mut impl Write,
    data_dir: &Path,
    action: InboxAction,
) -> Result<(), Box<dyn Error>> {
    let inbox = Inbox::in_data_dir(data_dir);
    match action {
        InboxAction::List => {
            for request in inbox.requests()? {
                serde_json::to_writer(&mut *out, &request)?;
                writeln!(out)?;
            }
        }
        InboxAction::Answer {
            request,
            recommended,
            choice,
        } => {
            let id = RequestId::from(request);
            let waiting = inbox.read_request(&id)?.ok_or("no such waiting request")?;
            let RequestBody::Question { questions } = &waiting.body else {
                return Err("that request is an approval; use `inbox approve`".into());
            };
            let choices = if recommended {
                vec![Choice::Recommended; questions.len()]
            } else {
                choice
                    .iter()
                    .map(|spec| parse_choice(spec))
                    .collect::<Result<_, _>>()?
            };
            let reply = Reply::Question { choices };
            check_reply(&waiting, &reply)?;
            inbox.write_answer(&id, &reply)?;
            let applied = autopilot_engine::sync_inbox(data_dir, &waiting.ticket)?;
            serde_json::to_writer(&mut *out, &serde_json::json!({"applied": applied}))?;
            writeln!(out)?;
        }
        InboxAction::Approve {
            request,
            reject,
            edit,
            allow_for_project,
        } => {
            let id = RequestId::from(request);
            let waiting = inbox.read_request(&id)?.ok_or("no such waiting request")?;
            let decision = match (reject, edit) {
                (true, _) => Decision::Reject,
                (false, Some(text)) => Decision::Edit(text),
                (false, None) => Decision::Approve,
            };
            let reply = Reply::Approval {
                decision,
                allow_for_project,
            };
            check_reply(&waiting, &reply)?;
            inbox.write_answer(&id, &reply)?;
            let applied = autopilot_engine::sync_inbox(data_dir, &waiting.ticket)?;
            serde_json::to_writer(&mut *out, &serde_json::json!({"applied": applied}))?;
            writeln!(out)?;
        }
    }
    Ok(())
}

fn parse_choice(spec: &str) -> Result<Choice, Box<dyn Error>> {
    if spec == "recommended" {
        return Ok(Choice::Recommended);
    }
    if let Some(text) = spec.strip_prefix("text:") {
        return Ok(Choice::Text(text.to_owned()));
    }
    let index: usize = spec.parse().map_err(|_| {
        format!("choice {spec:?} must be `recommended`, a number or `text:<answer>`")
    })?;
    Ok(Choice::Option(index))
}

fn state_name(state: &StepState) -> (&'static str, Option<String>) {
    match state {
        StepState::Idle => ("idle", None),
        StepState::Running => ("running", None),
        StepState::Asking(request) => ("asking", Some(request.as_str().to_owned())),
        StepState::Paused => ("paused", None),
        StepState::Skipped(reason) => ("skipped", Some(reason.clone())),
        StepState::Passed => ("passed", None),
        StepState::Failed(reason) => ("failed", Some(reason.clone())),
        StepState::Interrupted(reason) => ("interrupted", Some(reason.clone())),
    }
}

fn write_status(
    out: &mut impl Write,
    state: &TicketState,
    in_short: &[(CoreStep, String)],
    json: bool,
) -> Result<(), Box<dyn Error>> {
    let (cost, estimated) = state.cost();
    if json {
        let steps: Vec<_> = CoreStep::ALL
            .iter()
            .map(|step| {
                let (name, detail) = state_name(state.step(*step));
                serde_json::json!({"step": step.as_str(), "state": name, "detail": detail})
            })
            .collect();
        let value = serde_json::json!({
            "steps": steps,
            "in_short": in_short.iter().map(|(step, line)| serde_json::json!({"step": step.as_str(), "line": line})).collect::<Vec<_>>(),
            "done": state.is_done(),
            "cost_microusd": cost.as_microusd(),
            "cost_estimated": estimated,
        });
        serde_json::to_writer_pretty(&mut *out, &value)?;
        writeln!(out)?;
        return Ok(());
    }
    for step in CoreStep::ALL {
        match state_name(state.step(step)) {
            (name, Some(detail)) => writeln!(out, "{}: {name} ({detail})", step.as_str())?,
            (name, None) => writeln!(out, "{}: {name}", step.as_str())?,
        }
    }
    for (step, line) in in_short {
        writeln!(out, "in short, {}: {line}", step.as_str())?;
    }
    let label = if estimated { "estimated" } else { "billed" };
    writeln!(out, "cost: {} microUSD ({label})", cost.as_microusd())?;
    Ok(())
}

fn write_accounts(out: &mut impl Write, accounts: &Accounts) -> io::Result<()> {
    match &accounts.claude {
        Status::SignedIn(account) => {
            let kind = if account.method == "claude.ai" {
                "subscription"
            } else {
                "API key"
            };
            let mut line = format!("Claude  signed in with {kind}");
            for part in [&account.email, &account.organization, &account.plan]
                .into_iter()
                .flatten()
            {
                line.push_str(" | ");
                line.push_str(part);
            }
            writeln!(out, "{line}")?;
        }
        other => writeln!(out, "Claude  {}", unavailable(other))?,
    }
    match &accounts.github {
        Status::SignedIn(list) => {
            for (index, account) in list.iter().enumerate() {
                let label = if index == 0 { "GitHub " } else { "       " };
                let mut flags = Vec::new();
                if account.active {
                    flags.push("active");
                }
                if !account.valid {
                    flags.push("token rejected");
                }
                let flags = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", flags.join(", "))
                };
                writeln!(out, "{label} {}{flags} on {}", account.login, account.host)?;
            }
        }
        other => writeln!(out, "GitHub  {}", unavailable(other))?,
    }
    match &accounts.azure {
        Status::SignedIn(account) => {
            writeln!(
                out,
                "Azure   {} | subscription {}",
                account.user, account.subscription
            )?;
        }
        other => writeln!(out, "Azure   {}", unavailable(other))?,
    }
    Ok(())
}

fn unavailable<T>(status: &Status<T>) -> String {
    match status {
        Status::SignedIn(_) => String::new(),
        Status::SignedOut => "not signed in".into(),
        Status::NotInstalled => "not installed".into(),
        Status::Failed(reason) => format!("could not be checked: {reason}"),
    }
}

fn read_ticket(path: &Path) -> Result<Ticket, Box<dyn Error>> {
    const LIMIT: u64 = 1024 * 1024;
    let mut input = Vec::new();
    File::open(path)?.take(LIMIT + 1).read_to_end(&mut input)?;
    if input.len() as u64 > LIMIT {
        return Err("ticket file exceeds 1 MiB".into());
    }
    Ok(serde_json::from_slice(&input)?)
}
