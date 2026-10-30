//! Acting on a pull request: checking it out, merging, drafting, closing,
//! reopening and creating one.

use anyhow::Result;
use std::path::Path;

use super::MergeMethod;
use super::gh::{flag_value, last_line, run, run_combined};

/// Check a pull request out in the checkout lg is pointed at.
pub fn checkout(number: u64) -> Result<String> {
    let n = number.to_string();
    let out = run_combined(None, &["pr", "checkout", &n])?;
    Ok(last_line(&out).unwrap_or_else(|| format!("checked out #{number}")))
}

/// Check a pull request out in a new worktree at `path`, leaving the checkout
/// lg is pointed at as it was.
///
/// The worktree starts detached and `gh` checks the pull request out inside
/// it, so a pull request from a fork gets its remote set up the same way it
/// would in place.
pub fn checkout_in_worktree(number: u64, path: &Path) -> Result<String> {
    crate::git::worktree_add_detached(path)?;
    let n = number.to_string();
    match run_combined(Some(path), &["pr", "checkout", &n]) {
        Ok(_) => Ok(format!("checked out #{number} in {}", path.display())),
        Err(err) => {
            // A worktree parked on a detached HEAD is clutter nobody asked for.
            let _ = crate::git::worktree_remove(path, true);
            Err(err)
        }
    }
}

/// Merge a pull request. `auto` queues the merge for when the branch
/// protection rules are met, which is the only way through while required
/// checks are still running.
pub fn merge(number: u64, method: MergeMethod, delete_branch: bool, auto: bool) -> Result<String> {
    let n = number.to_string();
    let mut args = vec!["pr", "merge", &n, method.flag()];
    if delete_branch {
        args.push("--delete-branch");
    }
    if auto {
        args.push("--auto");
    }
    run_combined(None, &args)?;
    Ok(if auto {
        format!("#{number} will merge once its requirements are met")
    } else {
        format!("merged #{number} ({})", method.label())
    })
}

/// Mark a draft ready for review, or turn a pull request back into a draft.
pub fn set_draft(number: u64, draft: bool) -> Result<String> {
    let n = number.to_string();
    let mut args = vec!["pr", "ready", &n];
    if draft {
        args.push("--undo");
    }
    run_combined(None, &args)?;
    Ok(if draft {
        format!("#{number} is a draft again")
    } else {
        format!("#{number} is ready for review")
    })
}

pub fn close(number: u64) -> Result<String> {
    let n = number.to_string();
    run_combined(None, &["pr", "close", &n])?;
    Ok(format!("closed #{number}"))
}

pub fn reopen(number: u64) -> Result<String> {
    let n = number.to_string();
    run_combined(None, &["pr", "reopen", &n])?;
    Ok(format!("reopened #{number}"))
}

/// What a new pull request says and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullRequest {
    pub branch: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// Push `pr.branch` to `remote` and open a pull request for it.
///
/// `gh` cannot open one for a branch the remote has not seen, and asks where
/// to push it only when a terminal is there to answer. lg answers the way it
/// pushes everywhere else: to the configured remote, which the branch then
/// tracks.
pub fn create(remote: &str, pr: &NewPullRequest) -> Result<String> {
    crate::git::push_with_upstream(remote, &pr.branch)?;
    let (head, base) = (
        flag_value("--head", &pr.branch),
        flag_value("--base", &pr.base),
    );
    let (title, body) = (
        flag_value("--title", &pr.title),
        flag_value("--body", &pr.body),
    );
    let mut args = vec!["pr", "create", &head, &base, &title, &body];
    if pr.draft {
        args.push("--draft");
    }
    let url = run(&args)?;
    Ok(match last_line(&url) {
        Some(url) => format!("opened {url}"),
        None => format!("opened a pull request for {}", pr.branch),
    })
}

/// A title and body for a new pull request, read off the commits it carries:
/// one commit speaks for itself, several are listed under the branch name.
/// The same choice `gh pr create --fill` makes.
pub fn fill_from_commits(branch: &str, commits: &[(String, String)]) -> (String, String) {
    match commits {
        [(subject, body)] => (subject.clone(), body.trim().to_string()),
        _ => {
            let body = commits
                .iter()
                .map(|(subject, _)| format!("- {subject}"))
                .collect::<Vec<_>>()
                .join("\n");
            (humanize_branch(branch), body)
        }
    }
}

/// `feat/add-login` reads as `Add login`: the last path segment, with its
/// separators turned back into spaces.
fn humanize_branch(branch: &str) -> String {
    let leaf = branch.rsplit('/').next().unwrap_or(branch);
    let words = leaf.replace(['-', '_'], " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => branch.to_string(),
    }
}
