//! Creating, checking out, resetting and deleting branches.

use anyhow::{Context, Result};
use std::io::Write;
use std::process::Stdio;

use crate::config::is_protected_branch_name;

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
    let configured_base = crate::preferences::base_branch();
    let configured_remote = crate::preferences::remote();
    ensure_no_conflict_in_progress()?;
    progress();
    run(&["fetch"])?;
    if current_branch != target_branch {
        progress();
        run(&["checkout", target_branch])?;
    }
    // Where the remote branch is now, read before anything moves: the push
    // below may only overwrite exactly that, so a commit someone pushed to it
    // since the fetch is refused rather than thrown away.
    let upstream = branch_upstream(target_branch)?;
    let (push_remote, remote_branch) = upstream
        .as_deref()
        .and_then(upstream_push_target)
        .map(|(remote, branch)| (remote.to_string(), branch.to_string()))
        .unwrap_or_else(|| (configured_remote.to_string(), target_branch.to_string()));
    // Empty when the remote has no such branch: the lease then holds only
    // while it still has none.
    let expected = run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/remotes/{push_remote}/{remote_branch}"),
    ])
    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    .unwrap_or_default();
    progress();
    let safety_ref = create_safety_ref(&format!("reset-{target_branch}"))?;
    progress();
    run(&[
        "reset",
        "--hard",
        &format!("{configured_remote}/{configured_base}"),
    ])?;
    progress();
    let lease = format!("--force-with-lease=refs/heads/{remote_branch}:{expected}");
    let refspec = format!("refs/heads/{target_branch}:refs/heads/{remote_branch}");
    run(&["push", &lease, &push_remote, &refspec])?;
    if current_branch != target_branch {
        progress();
        run(&["checkout", current_branch])?;
    }
    progress();
    delete_safety_ref(&safety_ref)?;
    Ok(format!(
        "reset {target_branch} from origin/{configured_base}"
    ))
}

pub fn flow_discard_checkout_from_remote(current_branch: &str) -> Result<String> {
    flow_discard_checkout_from_remote_with_progress(current_branch, &mut || {})
}

pub fn flow_discard_checkout_from_remote_with_progress(
    current_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    let configured_remote = crate::preferences::remote();
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
        .unwrap_or(configured_remote.as_str());

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
    let configured_base = crate::preferences::base_branch();
    let configured_remote = crate::preferences::remote();
    run(&["fetch"])?;
    let start_point = if current_branch == configured_base.as_str() {
        run(&["pull", "--rebase"])?;
        configured_base.as_str().to_string()
    } else {
        format!("{configured_remote}/{configured_base}")
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
    let configured_remote = crate::preferences::remote();
    if !remote_exists(configured_remote.as_str()) {
        return Ok(None);
    }
    run(&["push", "-u", configured_remote.as_str(), new_branch])?;
    Ok(Some(format!("{configured_remote}/{new_branch}")))
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
    let configured_base = crate::preferences::base_branch();
    let configured_remote = crate::preferences::remote();
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
    let remote_main = format!("{configured_remote}/{configured_base}");
    let base_ref = if ref_exists(&remote_main) {
        remote_main
    } else if ref_exists(configured_base.as_str()) {
        configured_base.as_str().to_string()
    } else {
        anyhow::bail!("could not find {configured_base} or {configured_remote}/{configured_base}");
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
    let configured_base = crate::preferences::base_branch();
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
        let checkout = checkout_branch(configured_base.as_str())?;
        let checkout_line = checkout
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_owned();
        prefix = if checkout_line.is_empty() {
            format!("checked out {configured_base}; ")
        } else {
            format!("checked out {configured_base} ({checkout_line}); ")
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
    let configured_remote = crate::preferences::remote();
    if name.is_empty() {
        anyhow::bail!("branch name must not be empty");
    }
    if is_protected_branch(name) {
        anyhow::bail!("cannot delete protected branch {name}");
    }
    run_combined(&["push", configured_remote.as_str(), "--delete", name]).map(|text| {
        text.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("deleted")
            .to_owned()
    })
}

/// Delete the local branches that track nothing, as far as git agrees to.
///
/// Each goes through `git branch -d`, which keeps a branch holding commits the
/// checkout does not have: having no upstream is often exactly what a branch
/// nobody pushed yet looks like, and that is the one whose loss would hurt.
/// lg's own backups never come up at all. The first line sums up; each line
/// after it names one branch and what became of it, with git's reason for any
/// it kept.
pub fn flow_clean_orphan_branches(current_branch: &str) -> Result<String> {
    run(&["fetch"])?;
    let branches = orphan_branches(current_branch)?;
    if branches.is_empty() {
        return Ok("no orphan branches found".to_string());
    }

    let mut deleted = Vec::new();
    let mut kept = Vec::new();
    for branch in branches {
        match run(&["branch", "-d", &branch.name]) {
            Ok(_) => deleted.push(format!("deleted {}", branch.name)),
            Err(err) => kept.push(format!(
                "kept {}: {}",
                branch.name,
                refusal_reason(&err.to_string())
            )),
        }
    }
    let mut report = vec![format!(
        "deleted {} orphan branches, kept {}",
        deleted.len(),
        kept.len()
    )];
    report.extend(deleted);
    report.extend(kept);
    Ok(report.join("\n"))
}

/// The line of a failed `git branch -d` that says why, without the command
/// it was run as or git's hints after it.
fn refusal_reason(message: &str) -> String {
    let reason = message
        .split_once(" failed: ")
        .map_or(message, |(_, reason)| reason);
    reason
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.strip_prefix("error: ").unwrap_or(line).to_string())
        .unwrap_or_else(|| "git refused".to_string())
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

/// A local branch that tracks nothing, and what deleting it would lose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanBranch {
    pub name: String,
    /// Commits on it the checkout does not have. `git branch -d` keeps a
    /// branch while this is above zero.
    pub unmerged: u32,
    /// Commits on it that no remote-tracking branch has: the ones that exist
    /// only here.
    pub unpushed: u32,
}

/// The branches cleaning orphans would try to delete: every local branch with
/// no upstream, or one whose upstream is gone, other than the protected ones,
/// the one checked out, and lg's own backups.
pub fn orphan_branches(current_branch: &str) -> Result<Vec<OrphanBranch>> {
    let out = run(&["branch", "--format=%(refname:short)"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut orphans = Vec::new();
    for branch in text.lines().map(str::trim).filter(|b| !b.is_empty()) {
        if is_protected_branch_name(branch) || is_safety_ref(branch) || branch == current_branch {
            continue;
        }
        let upstream = git_command(&["rev-parse", "--abbrev-ref", &format!("{branch}@{{u}}")])
            .output()
            .with_context(|| format!("failed to check upstream for {branch}"))?;
        if upstream.status.success() {
            continue;
        }
        let unpushed = run(&["rev-list", "--count", branch, "--not", "--remotes"])?;
        orphans.push(OrphanBranch {
            name: branch.to_string(),
            unmerged: commits_missing_from("HEAD", branch)?,
            unpushed: String::from_utf8_lossy(&unpushed.stdout)
                .trim()
                .parse()
                .with_context(|| format!("parsing how much of {branch} is unpushed"))?,
        });
    }
    Ok(orphans)
}
