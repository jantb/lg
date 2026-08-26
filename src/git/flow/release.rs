//! Releasing the current branch to an environment.

use anyhow::{Context, Result};

use crate::config::{BRANCH_MAIN, BRANCH_TEST, DEFAULT_PUSH_REMOTE, DEV_BRANCH_NAMES};

use super::super::{head_branch, run, run_combined};
use super::*;

pub fn flow_release_current(current_branch: &str, target_branch: &str) -> Result<String> {
    flow_release_current_with_progress(current_branch, target_branch, &mut || {})
}

pub fn flow_release_current_with_progress(
    current_branch: &str,
    target_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    ensure_no_conflict_in_progress()?;
    ensure_feature_branch(current_branch)?;
    progress();
    let stashed = stash_uncommitted_changes(AUTO_STASH_RELEASE)?;
    progress();
    let safety_ref = create_safety_ref("release-current")?;
    if let Err(err) = release_current_branch(current_branch, target_branch, progress) {
        return Err(restore_auto_stash_after_failure(
            err,
            stashed,
            AUTO_STASH_RELEASE,
            current_branch,
        ));
    }
    progress();
    pop_stash_if_needed(stashed)?;
    delete_safety_ref(&safety_ref)?;
    Ok(format!(
        "released {current_branch} to {target_branch} -> {}",
        release_environment(target_branch)
    ))
}

/// Refuse to reset the deploy branch over commits only this checkout has.
///
/// A release starts the deploy branch from its remote, which is the right
/// starting point — and silent data loss when the local branch is ahead. That
/// is what a resolved-but-unpushed release conflict looks like: the merge
/// commit is sitting on the deploy branch, and resetting to the remote drops it
/// and walks straight back into the conflict it resolved.
fn ensure_target_matches_remote(target_branch: &str) -> Result<()> {
    let remote_ref = format!("{DEFAULT_PUSH_REMOTE}/{target_branch}");
    if !ref_exists(target_branch) || !ref_exists(&remote_ref) {
        return Ok(());
    }
    let ahead = commits_missing_from(&remote_ref, target_branch)?;
    if ahead == 0 {
        return Ok(());
    }
    let plural = if ahead == 1 { "commit" } else { "commits" };
    // The remedy has to work from a cold start: the followup a conflict left
    // behind lives only as long as lg does, so this says what to do with git
    // rather than which key would have continued the flow.
    anyhow::bail!(
        "{target_branch} has {ahead} {plural} that {remote_ref} does not, and releasing resets it to the remote — that would lose them.\nPush {target_branch} to keep them (they are most likely a resolved merge from a release that stopped), then release again."
    )
}

fn release_environment(target_branch: &str) -> &str {
    if DEV_BRANCH_NAMES.contains(&target_branch) {
        "dev"
    } else if target_branch == BRANCH_TEST {
        "test"
    } else {
        target_branch
    }
}

fn release_current_branch(
    current_branch: &str,
    target_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<()> {
    progress();
    run(&["push", DEFAULT_PUSH_REMOTE, current_branch])?;
    if target_branch != current_branch {
        progress();
        run(&["fetch"])?;
        progress();
        ensure_target_matches_remote(target_branch)?;
        run(&[
            "branch",
            "-f",
            target_branch,
            &format!("{DEFAULT_PUSH_REMOTE}/{target_branch}"),
        ])?;
        run(&[
            "branch",
            "--set-upstream-to",
            &format!("{DEFAULT_PUSH_REMOTE}/{target_branch}"),
            target_branch,
        ])?;
    } else {
        progress();
        run(&["fetch"])?;
        progress();
        run(&["pull", "--rebase"])?;
    }
    progress();
    run(&["checkout", target_branch])?;
    progress();
    merge_remote_main_into_current_release_branch(target_branch)?;
    progress();
    run(&["merge", &format!("{DEFAULT_PUSH_REMOTE}/{current_branch}")])?;
    progress();
    run(&[
        "push",
        DEFAULT_PUSH_REMOTE,
        &format!("HEAD:refs/heads/{target_branch}"),
    ])?;
    progress();
    run(&["checkout", current_branch])?;
    Ok(())
}

pub(crate) fn update_release_branch_from_main_before_commit() -> Result<Option<String>> {
    let current_branch = head_branch()?;
    if !is_release_branch(&current_branch) {
        return Ok(None);
    }

    let stashed =
        stash_uncommitted_changes("lg: auto-stash before updating release branch from main")?;
    let update = merge_remote_main_into_current_release_branch(&current_branch);
    match update {
        Ok(message) => {
            pop_stash_with_index_if_needed(stashed)?;
            Ok(message)
        }
        Err(err) => {
            if !git_path_exists("MERGE_HEAD").unwrap_or(false) {
                restore_stash_after_failed_checkout(stashed)?;
            }
            Err(err)
        }
    }
}

fn merge_remote_main_into_current_release_branch(branch: &str) -> Result<Option<String>> {
    ensure_release_branch(branch)?;
    run(&["fetch"])?;
    let remote_main = format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    if !ref_exists(&remote_main) {
        anyhow::bail!("cannot update {branch}: missing {remote_main}");
    }
    let out = run(&["rev-list", "--count", &remote_main, "--not", "HEAD"])?;
    let count = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .context("parsing release branch behind-main count")?;
    if count == 0 {
        return Ok(None);
    }

    let out = run_combined(&["merge", &remote_main])?;
    Ok(Some(
        out.lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or("updated release branch from origin/main")
            .to_string(),
    ))
}
