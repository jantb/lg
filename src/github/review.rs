//! Reviewing a pull request: approving or asking for changes, commenting, and
//! submitting one review with inline comments.

use anyhow::{Context, Result};
use std::process::Stdio;

use super::Verdict;
use super::gh::{command, failure, flag_value, run_combined, spawn_failure};

pub fn review(number: u64, verdict: Verdict, body: &str) -> Result<String> {
    let n = number.to_string();
    let body_arg = flag_value("--body", body);
    let mut args = vec!["pr", "review", &n, verdict.flag()];
    if !body.trim().is_empty() {
        args.push(&body_arg);
    }
    run_combined(None, &args)?;
    Ok(match verdict {
        Verdict::Approve => format!("approved #{number}"),
        Verdict::RequestChanges => format!("requested changes on #{number}"),
    })
}

pub fn comment(number: u64, body: &str) -> Result<String> {
    let n = number.to_string();
    run_combined(None, &["pr", "comment", &n, &flag_value("--body", body)])?;
    Ok(format!("commented on #{number}"))
}

/// How a submitted review settles the pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

impl ReviewEvent {
    pub const ALL: [Self; 3] = [Self::Comment, Self::Approve, Self::RequestChanges];

    fn api(self) -> &'static str {
        match self {
            Self::Comment => "COMMENT",
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::Approve => "approve",
            Self::RequestChanges => "request changes",
        }
    }
}

/// One inline comment of a review, on one line of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineComment {
    pub path: String,
    pub line: usize,
    /// `LEFT` for a removed line, `RIGHT` for any other.
    pub side: &'static str,
    pub body: String,
}

/// Submit one review of a pull request with its inline comments, all at once,
/// so the author is notified once rather than per comment.
pub fn submit_review(
    number: u64,
    commit: &str,
    event: ReviewEvent,
    body: &str,
    comments: &[LineComment],
) -> Result<String> {
    let payload = review_payload(commit, event, body, comments);
    let path = format!("repos/{{owner}}/{{repo}}/pulls/{number}/reviews");
    let args = ["api", "--method", "POST", path.as_str(), "--input", "-"];
    let mut child = command(None, &args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_failure)?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(payload.to_string().as_bytes())
            .context("send the review to gh")?;
    }
    let out = child.wait_with_output().context("wait for gh")?;
    if !out.status.success() {
        return Err(failure(
            &args,
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ));
    }
    let noun = if comments.len() == 1 {
        "comment"
    } else {
        "comments"
    };
    Ok(format!(
        "submitted a review ({}) on #{number} with {} inline {noun}",
        event.label(),
        comments.len()
    ))
}

pub(super) fn review_payload(
    commit: &str,
    event: ReviewEvent,
    body: &str,
    comments: &[LineComment],
) -> serde_json::Value {
    // GitHub refuses a comment or change request with no body, even one
    // carrying inline comments, so one is always sent.
    let body = if body.trim().is_empty() {
        match event {
            ReviewEvent::Approve => String::new(),
            _ => format!(
                "{} inline comment{}.",
                comments.len(),
                if comments.len() == 1 { "" } else { "s" }
            ),
        }
    } else {
        body.trim().to_string()
    };
    serde_json::json!({
        "commit_id": commit,
        "event": event.api(),
        "body": body,
        "comments": comments
            .iter()
            .map(|c| serde_json::json!({
                "path": c.path,
                "line": c.line,
                "side": c.side,
                "body": c.body,
            }))
            .collect::<Vec<_>>(),
    })
}
