//! Running git, and the reads and writes lg builds on it.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Output;

mod attrs;
mod branches;
mod commits;
mod config;
mod context;
mod diff;
mod flow;
mod index;
mod nested;
mod release;
mod remote;
mod review;
mod status;
mod worktree;

pub use attrs::{
    FileAttrs, SUPPRESSED_DIFF_MARKER, file_attrs, is_suppressed_diff_body, suppress_generated_diff,
};
pub use branches::{
    Branch, RemoteBranch, list_branches, list_remote_branches, nested_repo_branches,
    nested_repo_branches_at, nested_repo_remote_branches, nested_repo_remote_branches_at,
};
use commits::preferred_commit_ref;
pub use commits::{
    Commit, counts_ahead_behind, list_commits, list_commits_for_ref, recent_commit_messages,
    unpushed_shas,
};
pub use config::{
    AuthorConfig, IdeOpenCommand, add_to_gitignore, author_config, clear_local_author,
    clear_subtree_author, ide_open_command, open_file_in_ide, open_project_in_ide,
    open_project_path_in_ide, project_open_command, set_local_author, set_subtree_author,
    subtree_author_rule_exists,
};
pub use context::{active_repo, set_active_repo, spawn_pinned, with_repo};
use context::{git_command, git_command_in_dir};
pub use diff::{
    all_diffs, branch_log, fetch_updates, file_diff, folder_diff, repo_root, show_commit,
    staged_diff,
};
pub use flow::{
    ConflictHunk, ConflictSideCommit, ConflictSides, ConflictedFile, Followup,
    abort_in_progress_operation, abort_in_progress_operation_with_cleanup,
    abort_in_progress_operation_with_return, checkout_branch, checkout_remote_branch,
    conflict_sides, conflicted_files, delete_local_branch, delete_remote_branch,
    flow_clean_orphan_branches, flow_create_feature_branch, flow_discard_checkout_from_remote,
    flow_discard_checkout_from_remote_with_progress, flow_merge_main_into_all_local_branches,
    flow_merge_main_into_current, flow_merge_main_into_current_with_progress, flow_release_current,
    flow_release_current_with_progress, flow_reset_branch_from_main,
    flow_reset_branch_from_main_with_progress, flow_transfer_diff_to_feature_branch,
    flow_transfer_diff_to_feature_branch_with_progress, holds_conflict_marker, marker_line,
    stage_resolved_conflicts, validate_conflict_resolution,
};
pub use index::{
    commit, delete_worktree_path, rollback_worktree_path, stage, stage_all, unstage, unstage_all,
};
pub use nested::{
    NestedRepo, checkout_nested_branch, checkout_nested_branch_at, checkout_nested_remote_branch,
    checkout_nested_remote_branch_at, nested_repositories, nested_repositories_at,
};
pub use release::{
    BranchReleaseStatus, ReleaseBranches, ReleaseEnv, ReleaseTargetStatus, branch_release_status,
    release_branches,
};
pub use remote::{merge_upstream, pull, push, remote_url, set_branch_upstream};
pub use review::{
    AssistedReview, REVIEW_PR_TEXT_NODE_ID, ReviewNode, assisted_review_against_main,
    build_assisted_review_against_main,
};
pub use status::{
    FileEntry, parse_porcelain, parse_porcelain_xy, status_entries, status_porcelain,
};
pub use worktree::{
    Worktree, common_git_dir, default_worktree_path, main_worktree, parse_worktree_list,
    preferred_base_ref, same_dir, worktree_add, worktree_bring_home, worktree_land,
    worktree_land_with_progress, worktree_prune, worktree_remove, worktree_slug,
    worktree_sync_main, worktree_sync_main_with_progress, worktrees,
};

/// The lock file git refused over, when that is why a command failed.
///
/// Git says so across four lines and puts the part worth acting on last, so a
/// status bar with room for the first line says only that some file exists.
/// A lock left behind by a git process that died — lg crashing takes its own
/// git children with it — then makes every command that writes the index fail
/// while `git status` carries on as if nothing were wrong.
fn refused_lock_path(text: &str) -> Option<&str> {
    let (_, rest) = text.split_once("Unable to create '")?;
    let (path, _) = rest.split_once("': File exists")?;
    Some(path)
}

/// What a failed git command should say, in one line that survives being cut
/// to the width of a status bar.
fn failure_message(command: &str, text: &str) -> String {
    match refused_lock_path(text) {
        Some(path) => format!(
            "{command} failed: {path} exists, so another git process may be running here; delete it if none is"
        ),
        None => format!("{command} failed: {}", text.trim()),
    }
}

/// Same, keeping git's own output for the callers that show all of it.
fn combined_failure_message(command: &str, text: &str) -> String {
    match refused_lock_path(text) {
        Some(_) => failure_message(command, text),
        None => format!("{command} failed:\n{text}"),
    }
}

fn run(args: &[&str]) -> Result<Output> {
    let out = git_command(args)
        .output()
        .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
    if out.status.success() {
        Ok(out)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(anyhow::anyhow!(
            "{}",
            failure_message(&format!("git {}", args.join(" ")), &stderr)
        ))
    }
}

fn run_in_dir(dir: &Path, args: &[&str]) -> Result<Output> {
    let out = git_command_in_dir(dir, args).output().with_context(|| {
        format!(
            "failed to spawn git -C {} {}",
            dir.display(),
            args.join(" ")
        )
    })?;
    if out.status.success() {
        Ok(out)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(anyhow::anyhow!(
            "{}",
            failure_message(
                &format!("git -C {} {}", dir.display(), args.join(" ")),
                &stderr
            )
        ))
    }
}

fn run_combined(args: &[&str]) -> Result<String> {
    let out = git_command(args)
        .output()
        .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(anyhow::anyhow!(
            "{}",
            combined_failure_message(&format!("git {}", args.join(" ")), &text)
        ))
    }
}

fn run_combined_in_dir(dir: &Path, args: &[&str]) -> Result<String> {
    let out = git_command_in_dir(dir, args).output().with_context(|| {
        format!(
            "failed to spawn git -C {} {}",
            dir.display(),
            args.join(" ")
        )
    })?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(anyhow::anyhow!(
            "{}",
            combined_failure_message(
                &format!("git -C {} {}", dir.display(), args.join(" ")),
                &text
            )
        ))
    }
}

pub fn is_repo() -> bool {
    git_command(&["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn head_branch() -> Result<String> {
    let out = run(&["branch", "--show-current"])?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !branch.is_empty() {
        return Ok(branch);
    }

    let out = run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if branch == "HEAD" {
        anyhow::bail!("detached HEAD");
    }
    Ok(branch)
}
