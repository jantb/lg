//! The branch flows: what lg does to a branch, and how it gets back out.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::{
    BRANCH_MAIN, DEFAULT_PUSH_REMOTE, deploy_branch_list, is_deploy_branch_name,
    is_protected_branch_name, protected_branch_list,
};

use super::{git_command, parse_worktree_list, run};

mod branch;
mod conflict;
mod merge_main;
mod release;
mod safety;
mod stash;

pub use branch::{
    checkout_branch, checkout_remote_branch, delete_local_branch, delete_remote_branch,
    flow_clean_orphan_branches, flow_create_feature_branch, flow_discard_checkout_from_remote,
    flow_discard_checkout_from_remote_with_progress, flow_reset_branch_from_main,
    flow_reset_branch_from_main_with_progress, flow_transfer_diff_to_feature_branch,
    flow_transfer_diff_to_feature_branch_with_progress,
};
pub use conflict::{
    abort_in_progress_operation, abort_in_progress_operation_with_cleanup,
    abort_in_progress_operation_with_return, conflicted_files, stage_resolved_conflicts,
    validate_conflict_resolution_with_cleanup, validate_conflict_resolution_with_followup,
};
pub use merge_main::{
    flow_merge_main_into_all_local_branches, flow_merge_main_into_current,
    flow_merge_main_into_current_with_progress,
};
pub(super) use release::update_release_branch_from_main_before_commit;
pub use release::{flow_release_current, flow_release_current_with_progress};

use branch::restore_stash_after_failed_checkout;
use conflict::merge_or_rebase_in_progress;
use safety::*;
use stash::*;

fn summary_with_stash_notice(mut summary: String, stashed: bool) -> String {
    if stashed {
        summary.push_str(", restored stashed changes");
    }
    summary
}

/// How many commits `ahead_of` carries that `base` does not.
fn commits_missing_from(base: &str, ahead_of: &str) -> Result<u32> {
    let out = run(&["rev-list", "--count", ahead_of, "--not", base])?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .with_context(|| format!("parsing how far {ahead_of} is ahead of {base}"))
}

/// The checkout holding `branch`, when one does. Git refuses to move a branch
/// another worktree has checked out, so this is what says whether a ref can be
/// updated from here at all.
fn branch_checkout_dir(branch: &str) -> Result<Option<PathBuf>> {
    let out = run(&["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_list(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch))
        .map(|worktree| PathBuf::from(worktree.path)))
}

fn is_protected_branch(name: &str) -> bool {
    is_protected_branch_name(name)
}

fn git_path_exists(name: &str) -> Result<bool> {
    let out = run(&["rev-parse", "--git-path", name])?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(Path::new(&path).exists())
}

fn ensure_feature_branch(branch: &str) -> Result<()> {
    if branch.is_empty() || is_protected_branch_name(branch) {
        anyhow::bail!(
            "checkout a feature branch first; protected branches: {}",
            protected_branch_list()
        );
    }
    Ok(())
}

fn ensure_merge_main_branch(branch: &str) -> Result<()> {
    if branch.is_empty() || branch == BRANCH_MAIN {
        anyhow::bail!(
            "checkout a feature branch or a deploy branch ({}) first",
            deploy_branch_list()
        );
    }
    Ok(())
}

fn ensure_release_branch(branch: &str) -> Result<()> {
    if !is_release_branch(branch) {
        anyhow::bail!(
            "expected a deploy branch ({}), got {branch}",
            deploy_branch_list()
        );
    }
    Ok(())
}

fn is_release_branch(branch: &str) -> bool {
    is_deploy_branch_name(branch)
}

fn upstream_push_target(upstream: &str) -> Option<(&str, &str)> {
    let (remote, branch) = upstream.split_once('/')?;
    (!remote.is_empty() && !branch.is_empty()).then_some((remote, branch))
}

fn ref_exists(name: &str) -> bool {
    git_command(&["rev-parse", "--verify", "--quiet", name])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn remote_exists(name: &str) -> bool {
    git_command(&["remote", "get-url", name])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn is_valid_branch_name(name: &str) -> bool {
    git_command(&["check-ref-format", "--branch", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn remote_ref_for_branch(branch: &str, upstream: Option<&str>) -> Result<String> {
    if let Some(upstream) = upstream
        && ref_exists(upstream)
    {
        return Ok(upstream.to_string());
    }

    let remote_ref = format!("{DEFAULT_PUSH_REMOTE}/{branch}");
    if ref_exists(&remote_ref) {
        return Ok(remote_ref);
    }

    anyhow::bail!(
        "no remote branch found for {branch}; set an upstream or push {DEFAULT_PUSH_REMOTE}/{branch}"
    );
}

fn branch_upstream(branch: &str) -> Result<Option<String>> {
    let upstream_spec = format!("{branch}@{{upstream}}");
    let out = git_command(&[
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        &upstream_spec,
    ])
    .output()
    .with_context(|| format!("failed to check upstream for {branch}"))?;
    if !out.status.success() {
        return Ok(None);
    }

    let upstream = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok((!upstream.is_empty()).then_some(upstream))
}

fn remote_name_from_ref(remote_ref: &str) -> Option<&str> {
    let (remote, _) = remote_ref.split_once('/')?;
    (!remote.is_empty()).then_some(remote)
}

fn has_uncommitted_changes() -> Result<bool> {
    let out = run(&["status", "--porcelain"])?;
    Ok(!out.stdout.is_empty())
}
