//! The work a guided review hands to other threads: reading the change, the
//! model's commentary and answers, and Claude's fixes. Each answers on a
//! channel of its own, which [`super::poll`] reads.

use std::sync::mpsc::Receiver;

use super::{Loaded, Source};
use crate::state::GenMsg;

/// Read a walk over the checkout itself: the branch against main, or the
/// uncommitted changes.
pub(super) fn load_local(source: &Source) -> Receiver<Result<Loaded, String>> {
    let changes = matches!(source, Source::Changes { .. });
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let diff = if changes {
            crate::git::uncommitted_diff()
        } else {
            crate::git::branch_diff_against_main()
        };
        let result = diff
            .map(|diff| {
                let steps = crate::git::guided::parse_hunks(&diff.diff);
                let (title, source) = if changes {
                    (
                        format!("Uncommitted changes on {}", diff.branch),
                        Source::Changes {
                            branch: diff.branch,
                        },
                    )
                } else {
                    (
                        format!("Branch {} against {}", diff.branch, diff.base_ref),
                        Source::Branch {
                            branch: diff.branch,
                            base_ref: diff.base_ref,
                        },
                    )
                };
                let overview = super::overview_text(&title, "", &diff.commits, &steps);
                Loaded {
                    source,
                    steps,
                    overview,
                }
            })
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

pub(super) fn load_pull_request(
    pr: crate::github::PullRequest,
    local: bool,
) -> Receiver<Result<Loaded, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = (|| -> anyhow::Result<Loaded> {
            let diff = crate::github::pr_diff(pr.number)?;
            let commit = crate::github::pr_head(pr.number)?;
            let steps = crate::git::guided::parse_hunks(&diff);
            let overview = super::overview_text(
                &format!(
                    "Pull request #{} {} ({} into {})",
                    pr.number, pr.title, pr.head, pr.base
                ),
                &pr.body,
                &[],
                &steps,
            );
            Ok(Loaded {
                source: Source::PullRequest {
                    number: pr.number,
                    title: pr.title,
                    head: pr.head,
                    base: pr.base,
                    commit,
                    local,
                },
                steps,
                overview,
            })
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

/// The model's opening read of the whole change.
pub(super) fn ask_overview(context: String) -> Receiver<GenMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || crate::llm::stream_guided_overview(context, tx));
    rx
}

/// The model's commentary on one step.
pub(super) fn ask_step(context: String) -> Receiver<GenMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || crate::llm::stream_guided_step(context, tx));
    rx
}

/// The model's answer to a question about a step.
pub(super) fn ask_question(
    context: String,
    commentary: String,
    question: String,
) -> Receiver<GenMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        crate::llm::stream_guided_question(context, commentary, question, tx)
    });
    rx
}

/// Claude Code making the change `prompt` describes in the checkout at
/// `root`: what it said it did, or why it could not.
pub(super) fn run_fix(root: String, prompt: String) -> Receiver<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = crate::llm::run_claude_fix(std::path::Path::new(&root), &prompt)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}
