//! A flow that stopped on a conflict: what is unresolved, and how to get out.

use anyhow::Result;
use std::path::Path;

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
    // git reports conflicted paths relative to the repository, and lg never
    // changes the process working directory — so reading them relative to it
    // finds nothing, and a file full of markers would read as resolved.
    let root = crate::git::repo_root().unwrap_or_default();
    let root = Path::new(&root);
    let mut staged = Vec::new();
    for path in conflicted_files()? {
        let text = std::fs::read_to_string(root.join(&path)).unwrap_or_default();
        if has_conflict_markers(&text) {
            continue;
        }
        stage(&path)?;
        staged.push(path);
    }
    Ok(staged)
}

/// Whether the file still holds a conflict git wrote into it.
///
/// A start marker and an end marker together are what a conflict looks like;
/// either alone is somebody's document. Matching on `=======` anywhere in the
/// text — as this once did — leaves a resolved file that happens to contain a
/// row of equals signs permanently unresolvable, with no way to finish the flow
/// but to edit content lg has no business touching.
fn has_conflict_markers(text: &str) -> bool {
    let mut opened = false;
    for line in text.lines() {
        if is_marker(line, '<') {
            opened = true;
        } else if opened && is_marker(line, '>') {
            return true;
        }
    }
    false
}

/// Whether `line` is one of git's seven-character conflict markers. Git writes
/// exactly seven, followed by end of line or a space and the side's label, so a
/// longer run of the same character is a heading rule rather than a marker.
fn is_marker(line: &str, marker: char) -> bool {
    let Some(rest) = line.strip_prefix(&marker.to_string().repeat(CONFLICT_MARKER_WIDTH)) else {
        return false;
    };
    rest.is_empty() || rest.starts_with(' ')
}

/// How many characters git repeats in a conflict marker.
const CONFLICT_MARKER_WIDTH: usize = 7;

/// What a flow still owes once its conflict has been settled.
///
/// A conflict stops a flow part-way, and continuing means doing the rest of it
/// — not a fixed pair of steps. Which steps are left depends on where it
/// stopped, so each one is derived from the repository rather than assumed:
/// nothing here happens twice if it has already happened.
#[derive(Debug, Clone, Copy, Default)]
pub struct Followup<'a> {
    /// A branch whose remote head still has to be merged into `push_branch`
    /// before it is pushed. This is the half of a release the conflict
    /// interrupted; it is skipped when it is already in.
    pub merge_branch: Option<&'a str>,
    /// The branch to push once the flow's work is complete.
    pub push_branch: Option<&'a str>,
    /// The branch to leave the checkout on.
    pub return_branch: Option<&'a str>,
    /// The safety backup to drop, as (label, branch).
    pub safety_cleanup: Option<(&'a str, &'a str)>,
}

impl<'a> Followup<'a> {
    /// A flow whose only remaining work is to push and come back.
    pub fn new(push_branch: Option<&'a str>, return_branch: Option<&'a str>) -> Self {
        Self {
            push_branch,
            return_branch,
            ..Self::default()
        }
    }
}

pub fn validate_conflict_resolution(followup: Followup<'_>) -> Result<String> {
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

    if let Some(note) = merge_outstanding_branch(followup.merge_branch, followup.push_branch)? {
        out.push_str("\n\nMerge:\n");
        out.push_str(note.trim());
    }

    if let Some(branch) = followup.push_branch {
        let push = push_followup_branch(branch)?;
        out.push_str("\n\nPush:\n");
        out.push_str(push.trim());
    }

    if let Some(branch) = followup.return_branch {
        if head_branch()
            .map(|current| current != branch)
            .unwrap_or(true)
        {
            let checkout = run_combined(&["checkout", branch])?;
            out.push_str("\n\nCheckout:\n");
            out.push_str(checkout.trim());
        }
    }

    if let Some((label, branch)) = followup.safety_cleanup
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

/// Merge `origin/<merge_branch>` into `push_branch`, when the flow got no
/// further than the merge before it.
///
/// A release merges twice — origin/main into the deploy branch, then the
/// feature into it — and stops at whichever conflicts first. If that was the
/// first, the feature has not been merged at all, and continuing by pushing
/// would release nothing while reporting success.
///
/// What is left is measured against the deploy branch rather than HEAD, because
/// by the time this runs the checkout may be anywhere: a session that committed
/// the resolution and went back to the feature branch is the ordinary way to
/// arrive here, not an odd one. So it checks the branch out when it has to, and
/// the return leg puts the checkout back afterwards. Nothing happens at all when
/// the merge is already in, which is the case where the conflict *was* the
/// feature merge.
fn merge_outstanding_branch(
    merge_branch: Option<&str>,
    push_branch: Option<&str>,
) -> Result<Option<String>> {
    let (Some(merge_branch), Some(push_branch)) = (merge_branch, push_branch) else {
        return Ok(None);
    };
    let remote_ref = format!("{DEFAULT_PUSH_REMOTE}/{merge_branch}");
    if !ref_exists(&remote_ref)
        || !ref_exists(push_branch)
        || commits_missing_from(push_branch, &remote_ref)? == 0
    {
        return Ok(None);
    }

    let mut out = String::new();
    if head_branch()
        .map(|head| head != push_branch)
        .unwrap_or(true)
    {
        out.push_str(run_combined(&["checkout", push_branch])?.trim());
        out.push('\n');
    }
    out.push_str(run_combined(&["merge", "--no-edit", &remote_ref])?.trim());
    Ok(Some(out))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What git actually writes into a file it could not merge.
    const CONFLICTED: &str = "one\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> origin/main\ntwo\n";

    #[test]
    fn a_file_git_left_conflicted_is_not_treated_as_resolved() {
        assert!(has_conflict_markers(CONFLICTED));
    }

    #[test]
    fn a_diff3_conflict_is_recognised_by_its_outer_markers() {
        let text = "<<<<<<< ours\na\n||||||| base\nb\n=======\nc\n>>>>>>> theirs\n";
        assert!(has_conflict_markers(text));
    }

    /// The reason `v` could get stuck: a resolved file whose content happens to
    /// look like half a marker. A row of equals signs is a heading rule in half
    /// the documentation ever written, and it used to make a file permanently
    /// unresolvable.
    #[test]
    fn ordinary_text_that_looks_like_half_a_marker_reads_as_resolved() {
        for text in [
            "Heading\n=======\nbody\n",
            "Section\n=========================\nbody\n",
            "let width = a<<<<<<<b;\n",
            "printf('>>>>>>> done');\n",
            "banner\n<<<<<<<<<<<<<<<\n",
        ] {
            assert!(
                !has_conflict_markers(text),
                "this is somebody's file, not a conflict: {text:?}"
            );
        }
    }

    /// An opening marker with nothing closing it is not a conflict either —
    /// git writes both or neither.
    #[test]
    fn an_unpaired_marker_reads_as_resolved() {
        assert!(!has_conflict_markers("<<<<<<< HEAD\nours\n"));
        assert!(!has_conflict_markers("theirs\n>>>>>>> origin/main\n"));
    }

    #[test]
    fn the_resolved_version_of_a_conflicted_file_reads_as_resolved() {
        assert!(!has_conflict_markers("one\nresolved\ntwo\n"));
    }
}
