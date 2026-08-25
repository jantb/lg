//! A flow that stopped on a conflict: what is unresolved, and how to get out.

use anyhow::Result;

use crate::config::DEFAULT_PUSH_REMOTE;

use super::super::{head_branch, run, run_combined, stage};
use super::*;

pub fn conflicted_files() -> Result<Vec<String>> {
    let out = run(&["status", "--porcelain"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut files = Vec::new();
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        let status = &line[..2];
        if matches!(status, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
            files.push(line[3..].to_string());
        }
    }
    Ok(files)
}

pub fn stage_resolved_conflicts() -> Result<Vec<String>> {
    let mut staged = Vec::new();
    for path in conflicted_files()? {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if has_conflict_markers(&text) {
            continue;
        }
        stage(&path)?;
        staged.push(path);
    }
    Ok(staged)
}

fn has_conflict_markers(text: &str) -> bool {
    text.contains("<<<<<<<") || text.contains("=======") || text.contains(">>>>>>>")
}

pub fn validate_conflict_resolution_with_followup(
    push_branch: Option<&str>,
    return_branch: Option<&str>,
) -> Result<String> {
    validate_conflict_resolution_with_cleanup(push_branch, return_branch, None)
}

pub fn validate_conflict_resolution_with_cleanup(
    push_branch: Option<&str>,
    return_branch: Option<&str>,
    safety_cleanup: Option<(&str, &str)>,
) -> Result<String> {
    let staged = stage_resolved_conflicts()?;
    let conflicts = conflicted_files()?;
    if !conflicts.is_empty() {
        anyhow::bail!(
            "unresolved conflicts remain: {}\nResolve them outside lg, then press v to validate again.",
            conflicts.join(", ")
        );
    }

    let mut out;
    if git_path_exists("rebase-merge")? || git_path_exists("rebase-apply")? {
        out = run_combined(&["rebase", "--continue"])?;
    } else if git_path_exists("CHERRY_PICK_HEAD")? {
        out = run_combined(&["cherry-pick", "--continue"])?;
    } else if git_path_exists("MERGE_HEAD")? {
        run(&["add", "-A"])?;
        out = run_combined(&["commit", "--no-edit"])?;
        if !staged.is_empty() {
            out.push_str(&format!(
                "\nauto-staged resolved conflicts: {}",
                staged.join(", ")
            ));
        }
    } else {
        out = "no merge, rebase, or cherry-pick operation is in progress; assuming the conflict was completed manually".to_string();
    }

    if let Some(branch) = push_branch {
        let push = push_followup_branch(branch)?;
        out.push_str("\n\nPush:\n");
        out.push_str(push.trim());
    }

    if let Some(branch) = return_branch {
        if head_branch()
            .map(|current| current != branch)
            .unwrap_or(true)
        {
            let checkout = run_combined(&["checkout", branch])?;
            out.push_str("\n\nCheckout:\n");
            out.push_str(checkout.trim());
        }
    }

    if let Some((label, branch)) = safety_cleanup
        && let Some(backup) = delete_latest_safety_ref(label, branch)?
    {
        out.push_str("\n\nBackup:\nremoved ");
        out.push_str(&backup);
    }

    // The conflict is settled, so the changes the flow stashed on the way in
    // can come back. Leaving them there is how a checkout ends up looking
    // empty long after the flow that emptied it finished.
    if let Some(note) = restore_auto_stash_after_conflict() {
        out.push_str("\n\nStash:\n");
        out.push_str(&note);
    }

    Ok(out)
}

fn push_followup_branch(branch: &str) -> Result<String> {
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    match run_combined(&["push", DEFAULT_PUSH_REMOTE, &refspec]) {
        Ok(out) => Ok(out),
        Err(err) => {
            if is_non_fast_forward_error(&err.to_string()) {
                let current = head_branch().ok();
                let remote_ref = format!("{DEFAULT_PUSH_REMOTE}/{branch}");
                let fetch_refspec = format!("refs/heads/{branch}:refs/remotes/{remote_ref}");
                let mut out = format!("initial push was rejected because {remote_ref} advanced\n");
                out.push_str(&run_combined(&[
                    "fetch",
                    DEFAULT_PUSH_REMOTE,
                    &fetch_refspec,
                ])?);
                if current.as_deref() != Some(branch) {
                    out.push_str(&run_combined(&["checkout", branch])?);
                }
                out.push_str(&run_combined(&["merge", &remote_ref])?);
                out.push_str(&run_combined(&["push", DEFAULT_PUSH_REMOTE, &refspec])?);
                Ok(out)
            } else {
                Err(err)
            }
        }
    }
}

fn is_non_fast_forward_error(message: &str) -> bool {
    message.contains("non-fast-forward")
        || message.contains("fetch first")
        || message
            .contains("Updates were rejected because the tip of your current branch is behind")
}

pub fn abort_in_progress_operation() -> Result<String> {
    abort_in_progress_operation_with_return(None)
}

pub fn abort_in_progress_operation_with_return(return_branch: Option<&str>) -> Result<String> {
    abort_in_progress_operation_with_cleanup(return_branch, None)
}

pub fn abort_in_progress_operation_with_cleanup(
    return_branch: Option<&str>,
    safety_cleanup: Option<(&str, &str)>,
) -> Result<String> {
    let mut out;
    if git_path_exists("rebase-merge")? || git_path_exists("rebase-apply")? {
        out = run_combined(&["rebase", "--abort"])?;
    } else if git_path_exists("CHERRY_PICK_HEAD")? {
        out = run_combined(&["cherry-pick", "--abort"])?;
    } else if git_path_exists("MERGE_HEAD")? {
        out = run_combined(&["merge", "--abort"])?;
    } else {
        out = "no merge, rebase, or cherry-pick operation is in progress".to_string();
    }

    if let Some(branch) = return_branch {
        if head_branch()
            .map(|current| current != branch)
            .unwrap_or(true)
        {
            let checkout = run_combined(&["checkout", branch])?;
            out.push_str("\n\nCheckout:\n");
            out.push_str(checkout.trim());
        }
    }

    if let Some((label, branch)) = safety_cleanup
        && let Some(backup) = delete_latest_safety_ref(label, branch)?
    {
        out.push_str("\n\nBackup:\nremoved ");
        out.push_str(&backup);
    }

    Ok(out)
}

/// Whether git is part-way through something a stash cannot be applied over.
pub(super) fn merge_or_rebase_in_progress() -> bool {
    [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ]
    .iter()
    .any(|name| git_path_exists(name).unwrap_or(false))
        || !conflicted_files().unwrap_or_default().is_empty()
}
