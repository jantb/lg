//! The work a guided review hands to other threads: reading the change, the
//! model's commentary and answers, and Claude's fixes. Each answers on a
//! channel of its own, which [`super::poll`] reads.

use std::sync::mpsc::Receiver;

use super::{Loaded, Source};
use crate::state::GenMsg;

/// Read a walk over the checkout itself: the branch against main, or what is
/// not pushed yet.
pub(super) fn load_local(source: &Source) -> Receiver<Result<Loaded, String>> {
    let unpushed = matches!(source, Source::Unpushed { .. });
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let diff = if unpushed {
            crate::git::unpushed_diff()
        } else {
            crate::git::branch_diff_against_main()
        };
        let result = diff
            .map(|diff| {
                let steps = crate::git::guided::parse_files(&diff.diff);
                let (title, source) = if unpushed {
                    (
                        format!(
                            "What is not pushed on {} (against {})",
                            diff.branch, diff.base_ref
                        ),
                        Source::Unpushed {
                            branch: diff.branch,
                            base_ref: diff.base_ref,
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
                let overview = super::overview_of(&title, "", &diff.commits, &steps);
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
            let steps = crate::git::guided::parse_files(&diff);
            let overview = super::overview_of(
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

pub(super) fn ask_step(overview: String, file: String) -> Receiver<GenMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || crate::llm::stream_guided_step(overview, file, tx));
    rx
}

/// The model's read of the whole change across files, once the walk is through them.
pub(super) fn ask_final(context: String) -> Receiver<GenMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || crate::llm::stream_guided_final(context, tx));
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

/// Claude Code reading the checkout at `root` to settle the questions
/// `prompt` carries: the findings that still matter, one a line.
pub(super) fn check_questions(root: String, prompt: String) -> Receiver<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = crate::llm::run_claude_check(std::path::Path::new(&root), &prompt)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
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
