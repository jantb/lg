//! Creating, checking out, resetting and deleting branches.

use anyhow::{Context, Result};
use std::io::Write;
use std::process::Stdio;

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE, is_protected_branch_name};

use super::super::{git_command, head_branch, run, run_combined};
use super::*;

pub fn checkout_branch(name: &str) -> Result<String> {
    let stashed = stash_before_branch_change(name, "lg: auto-stash before checkout")?;
    let out = git_command(&["checkout", name])
        .output()
        .with_context(|| format!("failed to spawn git checkout {name}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    if out.status.success() {
        pop_stash_with_index_if_needed(stashed)?;
        Ok(checkout_output_with_stash_notice(combined, stashed))
    } else {
        restore_stash_after_failed_checkout(stashed)?;
        Err(anyhow::anyhow!("git checkout failed: {}", combined.trim()))
    }
}

pub fn checkout_remote_branch(remote_ref: &str) -> Result<String> {
    let stashed = stash_uncommitted_changes("lg: auto-stash before remote checkout")?;
    let out = git_command(&["switch", "--track", remote_ref])
        .output()
        .with_context(|| format!("failed to spawn git switch --track {remote_ref}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    if out.status.success() {
        pop_stash_with_index_if_needed(stashed)?;
        Ok(checkout_output_with_stash_notice(combined, stashed))
    } else {
        restore_stash_after_failed_checkout(stashed)?;
        Err(anyhow::anyhow!("git switch failed: {}", combined.trim()))
    }
}

pub fn flow_reset_branch_from_main(current_branch: &str, target_branch: &str) -> Result<String> {
    flow_reset_branch_from_main_with_progress(current_branch, target_branch, &mut || {})
}

pub fn flow_reset_branch_from_main_with_progress(
    current_branch: &str,
    target_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    progress();
    run(&["fetch"])?;
    if current_branch != target_branch {
        progress();
        run(&["checkout", target_branch])?;
    }
    progress();
    let safety_ref = create_safety_ref(&format!("reset-{target_branch}"))?;
    progress();
    run(&[
        "reset",
        "--hard",
        &format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"),
    ])?;
    progress();
    run(&["push", "--force"])?;
    if current_branch != target_branch {
        progress();
        run(&["checkout", current_branch])?;
    }
    progress();
    delete_safety_ref(&safety_ref)?;
    Ok(format!("reset {target_branch} from origin/{BRANCH_MAIN}"))
}

pub fn flow_discard_checkout_from_remote(current_branch: &str) -> Result<String> {
    flow_discard_checkout_from_remote_with_progress(current_branch, &mut || {})
}

pub fn flow_discard_checkout_from_remote_with_progress(
    current_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    if current_branch.trim().is_empty() {
        anyhow::bail!("checkout a branch first");
    }

    let actual_branch = head_branch().context("cannot discard checkout while HEAD is detached")?;
    if actual_branch != current_branch {
        anyhow::bail!("expected current branch {current_branch}, got {actual_branch}");
    }

    let upstream = branch_upstream(current_branch)?;
    let fetch_remote = upstream
        .as_deref()
        .and_then(remote_name_from_ref)
        .unwrap_or(DEFAULT_PUSH_REMOTE);

    progress();
    run(&["fetch", fetch_remote])?;
    let remote_ref = remote_ref_for_branch(current_branch, upstream.as_deref())?;

    progress();
    run(&["reset", "--hard", &remote_ref])?;

    progress();
    run(&["clean", "-fd"])?;

    Ok(format!(
        "discarded local checkout of {current_branch}; reset to {remote_ref}"
    ))
}

pub fn flow_create_feature_branch(current_branch: &str, new_branch: &str) -> Result<String> {
    if new_branch.trim().is_empty() {
        anyhow::bail!("branch name cannot be empty");
    }
    if !is_valid_branch_name(new_branch) {
        anyhow::bail!("invalid branch name: {new_branch}");
    }
    let stashed = stash_uncommitted_changes(AUTO_STASH_NEW_BRANCH)?;
    create_feature_branch(current_branch, new_branch, stashed).map_err(|err| {
        restore_auto_stash_after_failure(err, stashed, AUTO_STASH_NEW_BRANCH, current_branch)
    })
}

fn create_feature_branch(current_branch: &str, new_branch: &str, stashed: bool) -> Result<String> {
    run(&["fetch"])?;
    let start_point = if current_branch == BRANCH_MAIN {
        run(&["pull", "--rebase"])?;
        BRANCH_MAIN.to_string()
    } else {
        format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}")
    };
    run(&["checkout", "--no-track", "-b", new_branch, &start_point])?;
    pop_stash_if_needed(stashed)?;
    let upstream = push_new_feature_branch_upstream(new_branch)?;
    Ok(if let Some(upstream) = upstream {
        format!("created {new_branch} from {start_point}, tracking {upstream}")
    } else {
        format!("created {new_branch} from {start_point}")
    })
}

fn push_new_feature_branch_upstream(new_branch: &str) -> Result<Option<String>> {
    if !remote_exists(DEFAULT_PUSH_REMOTE) {
        return Ok(None);
    }
    run(&["push", "-u", DEFAULT_PUSH_REMOTE, new_branch])?;
    Ok(Some(format!("{DEFAULT_PUSH_REMOTE}/{new_branch}")))
}

pub fn flow_transfer_diff_to_feature_branch(
    source_branch: &str,
    new_branch: &str,
) -> Result<String> {
    flow_transfer_diff_to_feature_branch_with_progress(source_branch, new_branch, &mut || {})
}

pub fn flow_transfer_diff_to_feature_branch_with_progress(
    source_branch: &str,
    new_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    ensure_feature_branch(source_branch)?;
    if new_branch.trim().is_empty() {
        anyhow::bail!("branch name cannot be empty");
    }
    if !is_valid_branch_name(new_branch) {
        anyhow::bail!("invalid branch name: {new_branch}");
    }
    if source_branch == new_branch {
        anyhow::bail!("new branch must differ from source branch");
    }
    if ref_exists(new_branch) {
        anyhow::bail!("branch already exists: {new_branch}");
    }
    if has_uncommitted_changes()? {
        anyhow::bail!("stash or commit local changes before transferring a branch diff");
    }
    if !ref_exists(source_branch) {
        anyhow::bail!("source branch does not exist: {source_branch}");
    }

    progress();
    run(&["fetch"])?;
    let remote_main = format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    let base_ref = if ref_exists(&remote_main) {
        remote_main
    } else if ref_exists(BRANCH_MAIN) {
        BRANCH_MAIN.to_string()
    } else {
        anyhow::bail!("could not find {BRANCH_MAIN} or {DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    };

    progress();
    let patch = diff_against_base(&base_ref, source_branch)?;
    if patch.trim().is_empty() {
        anyhow::bail!("no diff between {source_branch} and {base_ref}");
    }

    progress();
    run(&["checkout", "--no-track", "-b", new_branch, &base_ref])?;

    progress();
    apply_patch_to_index(&patch)?;

    Ok(format!(
        "transferred {source_branch} diff against {base_ref} to {new_branch}"
    ))
}

pub fn delete_local_branch(name: &str, force: bool) -> Result<String> {
    if name.is_empty() {
        anyhow::bail!("branch name must not be empty");
    }
    if is_protected_branch(name) {
        anyhow::bail!("cannot delete protected branch {name}");
    }
    let mut prefix = String::new();
    if let Ok(current) = head_branch()
        && current == name
    {
        let checkout = checkout_branch(BRANCH_MAIN)?;
        let checkout_line = checkout
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_owned();
        prefix = if checkout_line.is_empty() {
            format!("checked out {BRANCH_MAIN}; ")
        } else {
            format!("checked out {BRANCH_MAIN} ({checkout_line}); ")
        };
    }
    let flag = if force { "-D" } else { "-d" };
    let out = run(&["branch", flag, name])?;
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("deleted")
        .to_owned();
    Ok(format!("{prefix}{line}"))
}

pub fn delete_remote_branch(name: &str) -> Result<String> {
    if name.is_empty() {
        anyhow::bail!("branch name must not be empty");
    }
    if is_protected_branch(name) {
        anyhow::bail!("cannot delete protected branch {name}");
    }
    run_combined(&["push", DEFAULT_PUSH_REMOTE, "--delete", name]).map(|text| {
        text.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("deleted")
            .to_owned()
    })
}

pub fn flow_clean_orphan_branches(current_branch: &str) -> Result<String> {
    run(&["fetch"])?;
    let branches = orphan_branches()?;
    if branches.is_empty() {
        return Ok("no orphan branches found".to_string());
    }

    let mut deleted = 0usize;
    let mut skipped = 0usize;
    for branch in branches {
        if branch == current_branch {
            skipped += 1;
            continue;
        }
        match run(&["branch", "-D", &branch]) {
            Ok(_) => deleted += 1,
            Err(_) => skipped += 1,
        }
    }
    Ok(format!(
        "deleted {deleted} orphan branches, skipped {skipped}"
    ))
}

fn diff_against_base(base_ref: &str, branch: &str) -> Result<String> {
    let revspec = format!("{base_ref}...{branch}");
    let out = git_command(&["diff", "--binary", "--full-index", &revspec])
        .output()
        .with_context(|| format!("failed to spawn git diff {revspec}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(anyhow::anyhow!(
            "git diff {revspec} failed: {}",
            stderr.trim()
        ))
    }
}

fn apply_patch_to_index(patch: &str) -> Result<()> {
    let mut child = git_command(&["apply", "--index", "--3way", "--binary"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn git apply")?;
    child
        .stdin
        .as_mut()
        .context("failed to open git apply stdin")?
        .write_all(patch.as_bytes())
        .context("failed to write patch to git apply")?;
    let out = child
        .wait_with_output()
        .context("failed to run git apply")?;
    if out.status.success() {
        Ok(())
    } else {
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(anyhow::anyhow!("git apply failed:\n{text}"))
    }
}

pub(super) fn restore_stash_after_failed_checkout(stashed: bool) -> Result<()> {
    if stashed {
        pop_stash_with_index_if_needed(true)
            .context("checkout failed after auto-stash; stash was not restored")?;
    }
    Ok(())
}

fn checkout_output_with_stash_notice(mut output: String, stashed: bool) -> String {
    if stashed {
        output.push_str("applied stashed local changes after checkout\n");
    }
    output
}

fn orphan_branches() -> Result<Vec<String>> {
    let out = run(&["branch", "--format=%(refname:short)"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut orphans = Vec::new();
    for branch in text.lines().map(str::trim).filter(|b| !b.is_empty()) {
        if is_protected_branch_name(branch) {
            continue;
        }
        let upstream = git_command(&["rev-parse", "--abbrev-ref", &format!("{branch}@{{u}}")])
            .output()
            .with_context(|| format!("failed to check upstream for {branch}"))?;
        if !upstream.status.success() {
            orphans.push(branch.to_string());
        }
    }
    Ok(orphans)
}
