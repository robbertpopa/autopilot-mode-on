//! The pull request step: ask the owner to approve one exact commit, then push
//! it and open a draft pull request.
//!
//! Two calls, because the owner answers in between. [`prepare_publish`] works
//! out what would be published and asks. [`run_publish`] runs after the owner
//! approves, checks that nothing moved, and only then pushes and publishes.
//! What the owner was shown is written into the log, so a restart rebuilds the
//! same request and the approval can only ever cover that commit.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use autopilot_checks::{Additions, resolve_executable, signals};
use autopilot_core::{
    AttemptId, Event, Money, RecordedEvent, RequestId, Step, Ticket, TicketId, Understanding,
};
use autopilot_git::{ObjectId, Repository, WorktreeManager};
use autopilot_pipeline::{
    StepState, fold,
    inbox::{Inbox, Request, RequestBody},
};
use autopilot_policy::{Facts, Policy, Risk};
use autopilot_profile::Profile;
use autopilot_store::{SqliteStore, Store};
use autopilot_tracker::{Approval, CheckLine, Ci, Gh, Published, Repo, description};
use serde::{Deserialize, Serialize};

use crate::{DATABASE, Error, event, lock_run, timestamp};

const PR: Option<Step> = Some(Step::PullRequest);
pub(crate) const PUBLISH_REQUEST: &str = "pull_request-publish";
const OFFER_PREFIX: &str = "publish offer: ";

pub struct PublishSetup<'a> {
    pub repository: &'a Path,
    pub base_branch: &'a str,
    pub worktrees: &'a Path,
    pub git: PathBuf,
    pub guardian: PathBuf,
    pub profile: &'a Profile,
    /// The name of the Git remote to push to, such as `origin`.
    pub remote: &'a str,
    pub repo: &'a Repo,
}

/// What the owner is asked to approve.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Offer {
    pub(crate) approval: Approval,
    /// Why this asks even with no checkbox: new dependencies, workflow files, and so on.
    reasons: Vec<String>,
}

/// What exists right now: the branch, its commit, and the repository handles.
struct Target {
    repository: Repository,
    manager: WorktreeManager,
    branch: String,
    base: ObjectId,
    head: ObjectId,
}

fn target(setup: &PublishSetup<'_>, ticket: &TicketId) -> Result<Target, Error> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let git = resolve_executable(&setup.git.to_string_lossy(), &path)
        .ok_or_else(|| Error::NotReady(format!("git was not found: {}", setup.git.display())))?;
    let open = || Repository::new(git.clone(), setup.guardian.clone(), setup.repository.into());
    let manager = WorktreeManager::new(open()?, setup.worktrees)?;
    let owned = manager.create(ticket, setup.base_branch)?;
    let repository = open()?;
    let branch = WorktreeManager::ticket_branch(ticket)?;
    let head = repository.resolve_local_branch(&branch)?;
    // The remote is checked here, not at push time only, so the owner is never
    // asked about a repository the push would not reach.
    let url = repository.remote_url(setup.remote)?;
    if !setup.repo.matches_remote(&url) {
        return Err(Error::NotReady(format!(
            "remote {} points at {url}, not {}",
            setup.remote, setup.repo
        )));
    }
    Ok(Target {
        repository,
        manager,
        branch,
        base: owned.base,
        head,
    })
}

fn ready_to_publish(history: &[RecordedEvent]) -> Result<autopilot_pipeline::TicketState, Error> {
    let state = fold(history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::BuildCheck) != &StepState::Passed {
        return Err(Error::NotReady(
            "the build has not passed its checks".into(),
        ));
    }
    // A custom step the ticket's pipeline runs is part of getting to the
    // pull request, so it settles first.
    for step in crate::pipelines::chosen_custom(history) {
        if !matches!(state.step(step), StepState::Passed | StepState::Skipped(_)) {
            return Err(Error::NotReady(format!(
                "{} has not finished",
                step.as_str()
            )));
        }
    }
    Ok(state)
}

pub(crate) fn last_offer(history: &[RecordedEvent]) -> Option<Offer> {
    history
        .iter()
        .rev()
        .find_map(|record| match &record.event.kind {
            Event::AgentActivity { summary } if record.event.step == PR => summary
                .strip_prefix(OFFER_PREFIX)
                .and_then(|json| serde_json::from_str(json).ok()),
            _ => None,
        })
}

/// The request for a prepared publish, rebuilt from the log. Always forced:
/// publishing leaves the machine, so no checkbox or standing allowance skips it.
pub(crate) fn request(ticket: &TicketId, history: &[RecordedEvent]) -> Result<Request, Error> {
    let offer = last_offer(history).ok_or_else(|| Error::History("no publish offer".into()))?;
    let approval = &offer.approval;
    let mut action = format!(
        "publish a draft pull request: push {} at {} to {}, base {}",
        approval.branch,
        approval.head.get(..8).unwrap_or(&approval.head),
        approval.repo,
        approval.base
    );
    if !offer.reasons.is_empty() {
        let _ = write!(action, ". Asks anyway: {}", offer.reasons.join(", "));
    }
    Ok(Request {
        id: crate::scoped(ticket, PUBLISH_REQUEST),
        ticket: ticket.clone(),
        body: RequestBody::Approval {
            action,
            forced: true,
        },
    })
}

fn push_event(
    store: &mut SqliteStore,
    ticket: &TicketId,
    last: &mut u64,
    kind: Event,
) -> Result<(), Error> {
    let committed = store.append(ticket, *last, &[event(timestamp()?, PR, kind)])?;
    *last = committed.last().map_or(*last, |record| record.seq);
    Ok(())
}

/// Work out what would be published and ask the owner. Nothing leaves the machine.
///
/// # Errors
/// Returns an error if the build has not passed, the remote is not the
/// approved repository, the branch may not be pushed, or the log cannot be written.
pub fn prepare_publish(
    directory: &Path,
    ticket: &Ticket,
    setup: &PublishSetup<'_>,
) -> Result<RequestId, Error> {
    let _owner = lock_run(directory, &ticket.id)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(&ticket.id, 0)?;
    let mut last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = ready_to_publish(&history)?;
    if !matches!(
        state.step(Step::PullRequest),
        StepState::Idle | StepState::Failed(_) | StepState::Interrupted(_)
    ) {
        return Err(Error::NotReady(
            "the pull request step is already under way".into(),
        ));
    }
    let policy = Policy::new(setup.profile).map_err(|error| Error::NotReady(error.to_string()))?;
    let found = target(setup, &ticket.id)?;
    policy.check_push(&found.branch)?;

    let changes = found.manager.committed_changes(&ticket.id)?;
    let touched: Vec<String> = changes
        .files
        .iter()
        .map(|file| file.path.to_string_lossy().replace('\\', "/"))
        .collect();
    let added: Vec<Additions> = found
        .repository
        .added_lines(&found.base, &found.head)?
        .into_iter()
        .map(|file| Additions {
            path: file.path,
            lines: file.lines,
        })
        .collect();
    let decision = policy.evaluate(
        false,
        &Facts {
            touched: &touched,
            risk: Risk::Standard,
            labels: &[],
            ticket_spent: Money::default(),
            day_spent: Money::default(),
            signals: signals(&added),
        },
    );
    let offer = Offer {
        approval: Approval::new(
            setup.repo,
            setup.base_branch,
            &found.branch,
            found.head.as_str(),
        )?,
        reasons: decision
            .forced
            .iter()
            .map(autopilot_policy::Reason::describe)
            .collect(),
    };
    let attempts = history
        .iter()
        .filter(|r| r.event.step == PR && matches!(r.event.kind, Event::StepStarted { .. }))
        .count();
    push_event(
        &mut store,
        &ticket.id,
        &mut last,
        Event::StepStarted {
            attempt: AttemptId::from(format!("pr-{}", attempts + 1)),
            session: None,
        },
    )?;
    let json = serde_json::to_string(&offer).map_err(|error| Error::History(error.to_string()))?;
    push_event(
        &mut store,
        &ticket.id,
        &mut last,
        Event::AgentActivity {
            summary: format!("{OFFER_PREFIX}{json}"),
        },
    )?;
    let id = crate::scoped(&ticket.id, PUBLISH_REQUEST);
    push_event(
        &mut store,
        &ticket.id,
        &mut last,
        Event::Asked {
            request: id.clone(),
        },
    )?;
    let history = store.events(&ticket.id, 0)?;
    Inbox::in_data_dir(directory).write_request(&request(&ticket.id, &history)?)?;
    Ok(id)
}

/// After the owner approved: check that nothing moved, push the approved
/// commit to the ticket branch, and open the draft pull request.
///
/// A failure after the approval leaves the step running, so calling again
/// retries. Pushing the same commit twice and publishing twice are both safe.
/// If the branch or commit no longer matches the approval, the step is
/// interrupted instead, because the owner approved something else.
///
/// # Errors
/// Returns an error if the approval does not cover what is there now, or the
/// push or `gh` fails.
pub fn run_publish(
    directory: &Path,
    ticket: &Ticket,
    setup: &PublishSetup<'_>,
    gh: &Gh,
) -> Result<Published, Error> {
    let _owner = lock_run(directory, &ticket.id)?;
    let mut store = SqliteStore::open(directory.join(DATABASE))?;
    let history = store.events(&ticket.id, 0)?;
    let mut last = history.last().ok_or(Error::UnknownTicket)?.seq;
    let state = ready_to_publish(&history)?;
    let answered = history.last().is_some_and(|record| {
        record.event.step == PR && matches!(record.event.kind, Event::Answered { .. })
    });
    if state.step(Step::PullRequest) != &StepState::Running || !answered {
        return Err(Error::NotReady("the publish has not been approved".into()));
    }
    let offer = last_offer(&history).ok_or_else(|| Error::History("no publish offer".into()))?;
    let policy = Policy::new(setup.profile).map_err(|error| Error::NotReady(error.to_string()))?;
    let found = target(setup, &ticket.id)?;
    policy.check_push(&found.branch)?;
    let now = Approval::new(
        setup.repo,
        setup.base_branch,
        &found.branch,
        found.head.as_str(),
    )?;
    if now != offer.approval {
        let reason = format!(
            "the approval was for {} but the branch is now at {}",
            offer.approval.head, now.head
        );
        push_event(
            &mut store,
            &ticket.id,
            &mut last,
            Event::Interrupted {
                reason: reason.clone(),
            },
        )?;
        return Err(Error::Tracker(autopilot_tracker::Error::HeadChanged {
            approved: offer.approval.head,
            current: now.head,
        }));
    }
    found
        .repository
        .push_commit(setup.remote, &found.head, &found.branch)?;
    let body = description(&summary(&history), &check_lines(&history));
    let published = gh.publish_draft(&offer.approval, found.head.as_str(), &ticket.title, &body)?;
    let pr = published.pull_request();
    push_event(
        &mut store,
        &ticket.id,
        &mut last,
        Event::AgentActivity {
            summary: format!("Draft pull request #{}: {}", pr.number, pr.url),
        },
    )?;
    push_event(&mut store, &ticket.id, &mut last, Event::StepFinished)?;
    Ok(published)
}

/// Where CI stands for the commit that was published, not for whatever the
/// branch points at now. Read-only: it records nothing.
///
/// # Errors
/// Returns an error if the ticket has no published pull request or `gh` fails.
pub fn ci_status(directory: &Path, ticket: &TicketId, gh: &Gh) -> Result<Ci, Error> {
    let store = SqliteStore::open_read_only(directory.join(DATABASE))?;
    let history = store.events(ticket, 0)?;
    let state = fold(&history).map_err(|error| Error::History(error.to_string()))?;
    if state.step(Step::PullRequest) != &StepState::Passed {
        return Err(Error::NotReady("no pull request has been published".into()));
    }
    let offer = last_offer(&history).ok_or_else(|| Error::History("no publish offer".into()))?;
    Ok(gh.ci(&offer.approval.head)?)
}

fn summary(history: &[RecordedEvent]) -> String {
    crate::latest_understanding(history).map_or_else(
        || "Changes for the ticket.".to_owned(),
        |understanding: &Understanding| understanding.summary.clone(),
    )
}

/// The latest result of each check, in the order the checks first ran.
fn check_lines(history: &[RecordedEvent]) -> Vec<CheckLine> {
    let mut lines: Vec<CheckLine> = Vec::new();
    for record in history
        .iter()
        .filter(|r| r.event.step == Some(Step::BuildCheck))
    {
        if let Event::CheckResult { check, passed, .. } = &record.event.kind {
            match lines.iter_mut().find(|line| &line.name == check) {
                Some(line) => line.passed = *passed,
                None => lines.push(CheckLine {
                    name: check.clone(),
                    passed: *passed,
                }),
            }
        }
    }
    lines
}
