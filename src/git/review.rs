//! Reviewing this branch against main: what changed, where to start, what to check.

use anyhow::Result;

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE};

use super::{head_branch, preferred_commit_ref, run};

mod category;
mod collect;
mod entry;
mod report;
mod source;
mod tree;

use collect::{
    branch_review_commits, worktree_review_diff, worktree_review_files, worktree_review_stat,
};
use entry::review_entry_points;
use report::render_assisted_review;
use tree::build_review_nodes;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewFile {
    status: String,
    path: String,
    old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewEntryPoint {
    path: String,
    line: Option<usize>,
    symbol: String,
    description: String,
    hunk: String,
    patch: Vec<String>,
    context: Vec<String>,
    added: usize,
    removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistedReview {
    pub report: String,
    pub nodes: Vec<ReviewNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNode {
    pub id: String,
    pub parent: Option<String>,
    pub depth: u16,
    pub title: String,
    pub body: Vec<String>,
    pub context: Vec<String>,
}

pub const REVIEW_PR_TEXT_NODE_ID: &str = "checklist:pr-text";

struct ReviewRender<'a> {
    branch: &'a str,
    base_ref: &'a str,
    merge_base: &'a str,
    commits: &'a [String],
    files: &'a [ReviewFile],
    stat: &'a str,
    entries: &'a [ReviewEntryPoint],
    diff: &'a str,
}

pub fn assisted_review_against_main() -> Result<String> {
    Ok(build_assisted_review_against_main()?.report)
}

pub fn build_assisted_review_against_main() -> Result<AssistedReview> {
    let base_ref =
        preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"), BRANCH_MAIN)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not find {DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN} or {BRANCH_MAIN}"
                )
            })?;
    let branch = head_branch().unwrap_or_else(|_| "HEAD".to_string());

    let merge_base = run(&["merge-base", &base_ref, "HEAD"])
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    let diff_base = if merge_base.is_empty() {
        base_ref.as_str()
    } else {
        merge_base.as_str()
    };
    let commits = branch_review_commits(&base_ref)?;
    let files = worktree_review_files(diff_base)?;
    let stat = worktree_review_stat(diff_base).unwrap_or_default();
    let diff = worktree_review_diff(diff_base)?;
    let entries = review_entry_points(&diff);

    let render = ReviewRender {
        branch: &branch,
        base_ref: &base_ref,
        merge_base: &merge_base,
        commits: &commits,
        files: &files,
        stat: &stat,
        entries: &entries,
        diff: &diff,
    };
    let report = render_assisted_review(&render);
    let nodes = build_review_nodes(&render);

    Ok(AssistedReview { report, nodes })
}

fn truncate_review_text(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn short_oid(oid: &str) -> &str {
    oid.get(..12).unwrap_or(oid)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_review_text_appends_ellipsis_when_cut() {
        assert_eq!(truncate_review_text("short", 10), "short");
        assert_eq!(truncate_review_text("0123456789xyz", 10), "0123456789...");
    }
}
