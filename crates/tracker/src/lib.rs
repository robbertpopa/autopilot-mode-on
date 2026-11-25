//! GitHub issues and pull requests through the `gh` CLI, on the host side.
//!
//! Nothing here is reachable from an agent. The agent never gets `gh`, a token
//! or a push, so everything that leaves the machine goes through the approval
//! checked in [`Gh::publish_draft`].
//!
//! `gh` runs with an allowlisted environment. Its sign-in comes from its own
//! config folder, not from a token variable that happens to be exported.

mod body;
mod names;
mod state;

use std::{ffi::OsString, path::PathBuf, sync::atomic::AtomicBool, time::Duration};

use autopilot_core::{Ticket, TicketId};
use autopilot_process::{CommandSpec, Limits, Outcome, Runner};
use serde::{Deserialize, Serialize};

pub use body::{CheckLine, description, redact};
pub use names::{Repo, valid_branch};
pub use state::{
    Decision, FailedCheck, Feedback, Mergeable, PrState, PullState, ci_block, feedback_block,
};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    bytes_per_stream: 1024 * 1024,
};

const ENV_ALLOWED: &[&str] = &[
    "APPDATA",
    "COMSPEC",
    "GH_CONFIG_DIR",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "LANG",
    "LOCALAPPDATA",
    "NO_PROXY",
    "PATH",
    "PATHEXT",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "SYSTEMDRIVE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "TMPDIR",
    "USERNAME",
    "USERPROFILE",
    "WINDIR",
    "XDG_CONFIG_HOME",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not a usable name")]
    InvalidName(String),
    #[error("the gh program must be an absolute path: {0}")]
    RelativeProgram(PathBuf),
    #[error("could not run gh: {0}")]
    Launch(#[from] std::io::Error),
    #[error("gh failed (exit code {code:?}): {message}")]
    Failed { code: Option<i32>, message: String },
    /// gh did not finish. Whether it did its work is not known.
    #[error("gh did not finish: {0}")]
    Uncertain(&'static str),
    #[error("could not read gh output: {0}")]
    Parse(String),
    #[error("the approval is for {approved}, not {actual}")]
    WrongTarget { approved: String, actual: String },
    #[error("the head changed after approval: approved {approved}, now {current}")]
    HeadChanged { approved: String, current: String },
    #[error("more than one pull request has the head branch {0}")]
    Ambiguous(String),
}

/// What the owner approved: this repository, this base, and exactly this head
/// commit on this branch. Any difference at publish time voids it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Approval {
    pub repo: String,
    pub base: String,
    pub branch: String,
    pub head: String,
}

impl Approval {
    /// # Errors
    /// Returns an error if the base, branch or head is not a plain name or object id.
    pub fn new(repo: &Repo, base: &str, branch: &str, head: &str) -> Result<Self, Error> {
        if !valid_branch(base) {
            return Err(Error::InvalidName(base.into()));
        }
        if !valid_branch(branch) {
            return Err(Error::InvalidName(branch.into()));
        }
        if head.len() < 40 || head.len() > 64 || !head.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::InvalidName(head.into()));
        }
        Ok(Self {
            repo: repo.to_string(),
            base: base.into(),
            branch: branch.into(),
            head: head.to_ascii_lowercase(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
}

impl Issue {
    /// The issue as a ticket. Its text is data from outside, and stays that
    /// way: the agent prompts frame it as untrusted.
    #[must_use]
    pub fn to_ticket(&self) -> Ticket {
        Ticket {
            id: TicketId::from(format!("gh-{}", self.number)),
            title: self.title.clone(),
            description: self.body.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub head: String,
    pub draft: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Published {
    /// This call made the pull request.
    Created(PullRequest),
    /// A pull request for the approved head already existed.
    Existing(PullRequest),
    /// The create call failed or timed out, but the pull request is there.
    Reconciled(PullRequest),
}

impl Published {
    #[must_use]
    pub fn pull_request(&self) -> &PullRequest {
        match self {
            Self::Created(pr) | Self::Existing(pr) | Self::Reconciled(pr) => pr,
        }
    }
}

/// Where CI stands for one commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ci {
    /// No check run exists for the commit yet.
    NoChecks,
    Pending,
    Passed,
    Failed(Vec<String>),
}

pub struct Gh {
    program: PathBuf,
    runner: Runner,
    repo: Repo,
    env: Vec<(OsString, OsString)>,
}

impl Gh {
    /// # Errors
    /// Returns an error if `program` is not an absolute path.
    pub fn new(
        program: PathBuf,
        guardian: PathBuf,
        repo: Repo,
        parent_env: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, Error> {
        if !program.is_absolute() {
            return Err(Error::RelativeProgram(program));
        }
        let env = parent_env
            .into_iter()
            .filter(|(name, _)| {
                ENV_ALLOWED.iter().any(|allowed| {
                    name.to_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(allowed))
                })
            })
            .collect();
        Ok(Self {
            program,
            runner: Runner::new(guardian),
            repo,
            env,
        })
    }

    /// Add environment beyond the allowlist, for a test double.
    #[must_use]
    pub fn with_extra_env(mut self, extra: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        self.env.extend(extra);
        self
    }

    fn run(&self, args: &[&str], input: &[u8]) -> Result<String, Error> {
        let spec = CommandSpec {
            program: self.program.clone(),
            args: args.iter().map(OsString::from).collect(),
            env: self.env.clone(),
            cwd: None,
        };
        let cancel = AtomicBool::new(false);
        let output = self.runner.run(&spec, input, LIMITS, &cancel)?;
        match output.outcome {
            Outcome::Exited(status) if status.success() => {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }
            Outcome::Exited(status) => Err(Error::Failed {
                code: status.code(),
                message: redact(&tail(&String::from_utf8_lossy(&output.stderr))),
            }),
            Outcome::TimedOut => Err(Error::Uncertain("it timed out")),
            Outcome::Cancelled => Err(Error::Uncertain("it was cancelled")),
            Outcome::OutputLimit => Err(Error::Uncertain("its output was too large")),
        }
    }

    /// Read one issue.
    ///
    /// # Errors
    /// Returns an error if `gh` fails or its answer cannot be read.
    pub fn issue(&self, number: u64) -> Result<Issue, Error> {
        let number_text = number.to_string();
        let repo = self.repo.to_string();
        let text = self.run(
            &[
                "issue",
                "view",
                &number_text,
                "--repo",
                &repo,
                "--json",
                "number,title,body,labels",
            ],
            &[],
        )?;
        let raw: RawIssue = parse(&text)?;
        Ok(Issue {
            number: raw.number,
            title: raw.title,
            body: raw.body.unwrap_or_default(),
            labels: raw.labels.into_iter().map(|label| label.name).collect(),
        })
    }

    /// The pull request whose head branch is `branch`, open or not.
    ///
    /// # Errors
    /// Returns an error if `gh` fails, or if more than one pull request matches.
    pub fn pull_request_for(&self, branch: &str) -> Result<Option<PullRequest>, Error> {
        if !valid_branch(branch) {
            return Err(Error::InvalidName(branch.into()));
        }
        let repo = self.repo.to_string();
        let text = self.run(
            &[
                "pr",
                "list",
                "--repo",
                &repo,
                "--head",
                branch,
                "--state",
                "all",
                "--json",
                "number,url,headRefOid,isDraft",
            ],
            &[],
        )?;
        let mut found: Vec<RawPull> = parse(&text)?;
        match found.len() {
            0 => Ok(None),
            1 => Ok(found.pop().map(RawPull::into_pull)),
            _ => Err(Error::Ambiguous(branch.into())),
        }
    }

    /// Open a draft pull request for the approved head, once.
    ///
    /// The approval must name this repository and the commit the caller is
    /// about to publish. The branch must already be on the remote. If a pull
    /// request for the branch exists, it is returned when its head is the
    /// approved one and refused otherwise, so a retry after a lost answer
    /// never makes a second one and a moved head never slips through.
    ///
    /// # Errors
    /// Returns an error if the approval does not match, the head moved, or
    /// `gh` fails and the pull request cannot be found afterwards.
    pub fn publish_draft(
        &self,
        approval: &Approval,
        current_head: &str,
        title: &str,
        body: &str,
    ) -> Result<Published, Error> {
        let repo = self.repo.to_string();
        if approval.repo != repo {
            return Err(Error::WrongTarget {
                approved: approval.repo.clone(),
                actual: repo,
            });
        }
        if !current_head.eq_ignore_ascii_case(&approval.head) {
            return Err(Error::HeadChanged {
                approved: approval.head.clone(),
                current: current_head.into(),
            });
        }
        if let Some(existing) = self.pull_request_for(&approval.branch)? {
            return same_head(approval, existing).map(Published::Existing);
        }
        let created = self.run(
            &[
                "pr",
                "create",
                "--repo",
                &repo,
                "--draft",
                "--base",
                &approval.base,
                "--head",
                &approval.branch,
                "--title",
                title,
                "--body-file",
                "-",
            ],
            redact(body).as_bytes(),
        );
        match created {
            Ok(_) => {
                let pr = self
                    .pull_request_for(&approval.branch)?
                    .ok_or_else(|| Error::Parse("the new pull request was not found".into()))?;
                same_head(approval, pr).map(Published::Created)
            }
            Err(error @ (Error::Failed { .. } | Error::Uncertain(_))) => {
                // The answer was lost or an error came after the work. Look.
                match self.pull_request_for(&approval.branch) {
                    Ok(Some(pr)) => same_head(approval, pr).map(Published::Reconciled),
                    _ => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// The facts about one pull request that decide what to do next. Comments
    /// from people outside the repository are dropped here.
    ///
    /// # Errors
    /// Returns an error if `gh` fails or an answer cannot be read.
    pub fn pr_state(&self, number: u64) -> Result<PrState, Error> {
        let number_text = number.to_string();
        let repo = self.repo.to_string();
        let text = self.run(
            &[
                "pr",
                "view",
                &number_text,
                "--repo",
                &repo,
                "--json",
                "number,headRefOid,isDraft,state,mergeable,reviewDecision",
            ],
            &[],
        )?;
        let view = state::decode_view(&text)?;
        let (owner, name) = repo
            .split_once('/')
            .ok_or_else(|| Error::InvalidName(repo.clone()))?;
        let owner_arg = format!("owner={owner}");
        let name_arg = format!("name={name}");
        let number_arg = format!("number={number}");
        let query = format!(
            "query=query($owner:String!,$name:String!,$number:Int!){{repository(owner:$owner,name:$name)\
             {{pullRequest(number:$number){{reviewThreads(first:{}){{nodes{{isResolved \
             comments(first:{}){{nodes{{author{{login}} authorAssociation body}}}}}}}}}}}}}}}}",
            state::MAX_THREADS,
            state::MAX_THREAD_COMMENTS
        );
        let text = self.run(
            &[
                "api",
                "graphql",
                "-F",
                &owner_arg,
                "-F",
                &name_arg,
                "-F",
                &number_arg,
                "-f",
                &query,
            ],
            &[],
        )?;
        let threads = state::decode_threads(&text)?;
        let ci = self.ci(&view.head)?;
        Ok(PrState {
            number: view.number,
            head: view.head,
            draft: view.draft,
            state: view.state,
            mergeable: view.mergeable,
            decision: view.decision,
            unresolved_threads: threads.unresolved,
            feedback: threads.feedback,
            ci,
        })
    }

    /// The failing checks of one exact commit, each with the end of its log.
    /// A log is attacker-controlled text: it is bounded and cleaned here and
    /// goes to an agent only through [`ci_block`].
    ///
    /// # Errors
    /// Returns an error if `gh` fails or the list of check runs cannot be read.
    pub fn failed_checks(&self, head: &str) -> Result<Vec<FailedCheck>, Error> {
        let runs = self.check_runs(head)?;
        let mut found = Vec::new();
        for run in runs
            .iter()
            .filter(|run| is_failed(run))
            .take(state::MAX_FAILED)
        {
            // A job log is only there for GitHub Actions jobs and can be
            // larger than the read limit; the run's own output text is the
            // next best thing and may be all a third-party check has.
            let log = run
                .id
                .and_then(|id| {
                    let path = format!("repos/{}/actions/jobs/{id}/logs", self.repo);
                    self.run(&["api", &path], &[]).ok()
                })
                .or_else(|| run.output.as_ref().map(RawOutput::text))
                .unwrap_or_default();
            found.push(FailedCheck {
                name: state::clean_name(&run.name),
                log: state::log_tail(&log),
            });
        }
        Ok(found)
    }

    fn check_runs(&self, head: &str) -> Result<Vec<RawRun>, Error> {
        if head.len() < 40 || !head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::InvalidName(head.into()));
        }
        let path = format!("repos/{}/commits/{head}/check-runs", self.repo);
        let text = self.run(&["api", &path], &[])?;
        let runs: RawRuns = parse(&text)?;
        Ok(runs.check_runs)
    }

    /// CI for one exact commit, not for whatever the branch points at now.
    ///
    /// # Errors
    /// Returns an error if `gh` fails or its answer cannot be read.
    pub fn ci(&self, head: &str) -> Result<Ci, Error> {
        Ok(summarize(&self.check_runs(head)?))
    }
}

fn same_head(approval: &Approval, pr: PullRequest) -> Result<PullRequest, Error> {
    if pr.head.eq_ignore_ascii_case(&approval.head) {
        Ok(pr)
    } else {
        Err(Error::HeadChanged {
            approved: approval.head.clone(),
            current: pr.head,
        })
    }
}

fn is_failed(run: &RawRun) -> bool {
    run.status == "completed"
        && !matches!(
            run.conclusion.as_deref(),
            Some("success" | "neutral" | "skipped")
        )
}

fn summarize(runs: &[RawRun]) -> Ci {
    if runs.is_empty() {
        return Ci::NoChecks;
    }
    let failed: Vec<String> = runs
        .iter()
        .filter(|run| is_failed(run))
        .map(|run| run.name.clone())
        .collect();
    if !failed.is_empty() {
        return Ci::Failed(failed);
    }
    if runs.iter().all(|run| run.status == "completed") {
        Ci::Passed
    } else {
        Ci::Pending
    }
}

fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(|error| Error::Parse(error.to_string()))
}

/// The end of a long message, where tools put the reason.
fn tail(text: &str) -> String {
    const KEEP: usize = 600;
    let text = text.trim();
    if text.len() <= KEEP {
        return text.to_owned();
    }
    let mut start = text.len() - KEEP;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("...{}", &text[start..])
}

#[derive(Deserialize)]
struct RawIssue {
    number: u64,
    title: String,
    body: Option<String>,
    #[serde(default)]
    labels: Vec<RawLabel>,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

#[derive(Deserialize)]
struct RawPull {
    number: u64,
    url: String,
    #[serde(rename = "headRefOid")]
    head: String,
    #[serde(rename = "isDraft")]
    draft: bool,
}

impl RawPull {
    fn into_pull(self) -> PullRequest {
        PullRequest {
            number: self.number,
            url: self.url,
            head: self.head,
            draft: self.draft,
        }
    }
}

#[derive(Deserialize)]
struct RawRuns {
    check_runs: Vec<RawRun>,
}

#[derive(Deserialize)]
struct RawRun {
    name: String,
    status: String,
    conclusion: Option<String>,
    id: Option<u64>,
    output: Option<RawOutput>,
}

#[derive(Deserialize)]
struct RawOutput {
    title: Option<String>,
    summary: Option<String>,
    text: Option<String>,
}

impl RawOutput {
    fn text(&self) -> String {
        [&self.title, &self.summary, &self.text]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, status: &str, conclusion: Option<&str>) -> RawRun {
        RawRun {
            name: name.into(),
            status: status.into(),
            conclusion: conclusion.map(Into::into),
            id: None,
            output: None,
        }
    }

    // Invariant: CI is only "passed" when every run finished well. A run still
    // going, or no run at all, is never read as green.
    #[test]
    fn ci_is_passed_only_when_every_run_finished_well() {
        assert_eq!(summarize(&[]), Ci::NoChecks);
        assert_eq!(
            summarize(&[
                run("a", "completed", Some("success")),
                run("b", "in_progress", None)
            ]),
            Ci::Pending
        );
        assert_eq!(
            summarize(&[
                run("a", "completed", Some("success")),
                run("b", "completed", Some("skipped"))
            ]),
            Ci::Passed
        );
        assert_eq!(
            summarize(&[
                run("a", "completed", Some("success")),
                run("b", "completed", Some("failure")),
                run("c", "queued", None)
            ]),
            Ci::Failed(vec!["b".into()])
        );
    }

    // Invariant: an approval needs a full object id, so a short or odd string
    // cannot stand in for the checked commit.
    #[test]
    fn an_approval_needs_plain_names_and_a_full_object_id() {
        let repo = Repo::parse("owner/name").unwrap();
        let head = "a".repeat(40);
        assert!(Approval::new(&repo, "main", "ticket/1", &head).is_ok());
        assert!(Approval::new(&repo, "main", "ticket/1", "abc123").is_err());
        assert!(Approval::new(&repo, "--help", "ticket/1", &head).is_err());
        assert!(Approval::new(&repo, "main", "a b", &head).is_err());
        assert!(Approval::new(&repo, "main", "ticket/1", &"g".repeat(40)).is_err());
    }
}
