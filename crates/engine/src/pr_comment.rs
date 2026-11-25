//! Posting what the reviewers found on a pull request, after the owner has
//! approved the exact text.
//!
//! A comment on someone else's pull request is public and cannot be taken back,
//! so it is always a forced approval: no checkbox or allowance skips it. The
//! offer (text and commit) is written to disk before the owner is asked, and the
//! comment that is posted is that stored text, never a second rendering.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use autopilot_core::{RequestId, TicketId};
use autopilot_pipeline::inbox::{Decision, Inbox, Reply, Request, RequestBody, check_reply};
use autopilot_tracker::{Gh, Repo};
use serde::{Deserialize, Serialize};

use crate::{Error, pr_review::PrReview};

const OFFERS: &str = "pr-review-offers";
const MAX_FINDINGS: usize = 20;
const MAX_ISSUE_CHARS: usize = 600;

#[derive(Debug, Deserialize, Serialize)]
struct Offer {
    repo: String,
    number: u64,
    head: String,
    body: String,
}

fn offer_path(data: &Path, id: &RequestId) -> PathBuf {
    data.join(OFFERS).join(format!("{}.json", id.as_str()))
}

/// Model text that came from someone else's diff, made safe to publish: no
/// control characters, no HTML, no `@` that would notify a stranger.
fn publishable(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().take(MAX_ISSUE_CHARS) {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '@' => out.push_str("@\u{200b}"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn comment_body(review: &PrReview) -> String {
    let head = review.head.get(..8).unwrap_or(&review.head);
    let mut body = format!(
        "Automated review of commit {head} by {} reviewers. It is not an approval and may be wrong.\n\n",
        review.reviewers
    );
    if review.findings.is_empty() {
        body.push_str("Nothing was found with enough confidence to report.\n");
    }
    for finding in review.findings.iter().take(MAX_FINDINGS) {
        let _ = writeln!(
            body,
            "- `{}:{}` ({}, {}%): {}",
            publishable(&finding.path).replace('`', "'"),
            finding.line,
            crate::build::review::kind_name(finding.kind),
            finding.confidence,
            publishable(&finding.issue)
        );
    }
    if review.findings.len() > MAX_FINDINGS {
        let _ = writeln!(
            body,
            "\n{} more findings are not listed.",
            review.findings.len() - MAX_FINDINGS
        );
    }
    if review.diff_left_out > 0 {
        let _ = writeln!(
            body,
            "\nThe last {} bytes of the diff were not read.",
            review.diff_left_out
        );
    }
    body
}

/// Write the offer and ask the owner. Nothing is posted.
///
/// # Errors
/// Returns an error if the offer or the request cannot be written.
pub fn prepare_pr_comment(data: &Path, repo: &Repo, review: &PrReview) -> Result<RequestId, Error> {
    let offer = Offer {
        repo: repo.to_string(),
        number: review.number,
        head: review.head.clone(),
        body: comment_body(review),
    };
    let ticket = TicketId::from(format!("review-pr-{}", review.number));
    let key = TicketId::from(format!("{}#{}@{}", offer.repo, offer.number, offer.head));
    let id = crate::scoped(&key, "pr_comment");
    let path = offer_path(data, &id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::ReviewRefused(error.to_string()))?;
    }
    let json =
        serde_json::to_vec(&offer).map_err(|error| Error::ReviewRefused(error.to_string()))?;
    fs::write(&path, json).map_err(|error| Error::ReviewRefused(error.to_string()))?;
    let action = format!(
        "post this comment on {}#{} at commit {}:\n\n{}",
        offer.repo, offer.number, offer.head, offer.body
    );
    Inbox::in_data_dir(data).write_request(&Request {
        id: id.clone(),
        ticket,
        body: RequestBody::Approval {
            action,
            forced: true,
        },
    })?;
    Ok(id)
}

/// After the owner approved: check the pull request is still at the commit that
/// was reviewed, then post the stored text. The offer is removed before the
/// post, so a comment that may have been posted is never posted twice; a failed
/// post means running the review again.
///
/// # Errors
/// Returns an error if there is no offer, no approval, a decision other than
/// approve, a different repository or a moved head, or `gh` fails.
pub fn post_pr_comment(data: &Path, gh: &Gh, repo: &Repo, id: &RequestId) -> Result<(), Error> {
    let inbox = Inbox::in_data_dir(data);
    let path = offer_path(data, id);
    let text = fs::read_to_string(&path)
        .map_err(|_| Error::NotReady("there is no such comment waiting".into()))?;
    let offer: Offer =
        serde_json::from_str(&text).map_err(|error| Error::ReviewRefused(error.to_string()))?;
    let request = inbox
        .read_request(id)?
        .ok_or_else(|| Error::NotReady("the request is gone; run the review again".into()))?;
    let reply = inbox
        .read_answer(id)?
        .ok_or_else(|| Error::NotReady("the comment has not been approved".into()))?;
    check_reply(&request, &reply)?;
    let approved = matches!(
        reply,
        Reply::Approval {
            decision: Decision::Approve,
            ..
        }
    );
    if !approved {
        discard(&inbox, id, &path)?;
        return Err(Error::ReviewRefused(
            "the comment was not approved as written, so nothing was posted".into(),
        ));
    }
    if offer.repo != repo.to_string() {
        return Err(Error::ReviewRefused(format!(
            "the approval was for {}, not {repo}",
            offer.repo
        )));
    }
    let now = gh.pr_subject(offer.number)?;
    if now.head != offer.head {
        discard(&inbox, id, &path)?;
        return Err(Error::ReviewRefused(format!(
            "the approval was for {} but the pull request is now at {}; run the review again",
            offer.head, now.head
        )));
    }
    discard(&inbox, id, &path)?;
    gh.pr_comment(offer.number, &offer.body)?;
    Ok(())
}

fn discard(inbox: &Inbox, id: &RequestId, path: &Path) -> Result<(), Error> {
    inbox.clean_up(id)?;
    fs::remove_file(path).map_err(|error| Error::ReviewRefused(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::publishable;

    // Invariant: text that came from a stranger's diff cannot notify people,
    // inject HTML or carry control characters into a public comment. Breaks if
    // the escaping in `publishable` is dropped; the CLI tests use a fixed
    // finding with none of these characters.
    #[test]
    fn model_text_is_made_safe_to_publish() {
        let out = publishable("@owner <img src=x>\u{1b}[2J\nnext");
        assert!(!out.contains("@owner"));
        assert!(!out.contains('<'));
        assert!(!out.contains('\u{1b}'));
        assert!(!out.contains('\n'));
        assert!(out.contains("next"));
    }
}
