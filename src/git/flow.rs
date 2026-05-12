use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST, DEFAULT_PUSH_REMOTE};

use super::{head_branch, list_branches, run, run_combined, stage};

const SAFETY_REF_PREFIX: &str = "lg/backup/";
const SAFETY_REF_KEEP: usize = 20;

pub fn checkout_branch(name: &str) -> Result<String> {
    let stashed = stash_before_branch_change(name, "lg: auto-stash before checkout")?;
    let out = Command::new("git")
        .args(["checkout", name])
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
    let out = Command::new("git")
        .args(["switch", "--track", remote_ref])
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

pub fn flow_merge_main_into_current(current_branch: &str) -> Result<String> {
    flow_merge_main_into_current_with_progress(current_branch, &mut || {})
}

pub fn flow_merge_main_into_all_local_branches() -> Result<String> {
    let original_branch = head_branch().ok();
    let stashed = stash_uncommitted_changes("lg: auto-stash before syncing all branches")?;
    let result = flow_merge_main_into_all_local_branches_clean(original_branch.as_deref());

    if result.is_err() && !conflicted_files().unwrap_or_default().is_empty() {
        return result.map(|summary| summary_with_stash_notice(summary, stashed));
    }

    if let Some(original) = original_branch.as_deref()
        && ref_exists(original)
        && head_branch()
            .map(|current| current != original)
            .unwrap_or(true)
    {
        run_combined(&["checkout", original])?;
    }
    pop_stash_with_index_if_needed(stashed)?;
    result.map(|summary| summary_with_stash_notice(summary, stashed))
}

fn flow_merge_main_into_all_local_branches_clean(original_branch: Option<&str>) -> Result<String> {
    run(&["fetch", "--all", "--prune"])?;
    if !ref_exists(BRANCH_MAIN) {
        anyhow::bail!("could not find local {BRANCH_MAIN}");
    }

    if head_branch()
        .map(|current| current != BRANCH_MAIN)
        .unwrap_or(true)
    {
        run_combined(&["checkout", BRANCH_MAIN])?;
    }
    let remote_main = format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    let base_ref = if ref_exists(&remote_main) {
        run_combined(&["pull", "--rebase", DEFAULT_PUSH_REMOTE, BRANCH_MAIN])?;
        remote_main
    } else {
        BRANCH_MAIN.to_string()
    };

    let branches = list_branches()?;
    let mut merged = 0usize;
    let mut pushed = 0usize;
    let mut skipped_push = 0usize;
    let mut failed_pushes = Vec::new();
    for branch in branches {
        if branch.name == BRANCH_MAIN || branch.name.starts_with(SAFETY_REF_PREFIX) {
            continue;
        }

        run_combined(&["checkout", &branch.name])?;
        if branch.upstream.is_some()
            && !branch.upstream_gone
            && let Err(err) = run_combined(&["merge", "--no-edit", "@{u}"])
        {
            anyhow::bail!(
                "merge upstream into {} failed:\n{err}\nresolve conflicts outside lg, then validate the conflict in lg",
                branch.name
            );
        }
        if let Err(err) = run_combined(&["merge", "--no-edit", &base_ref]) {
            anyhow::bail!(
                "merge {base_ref} into {} failed:\n{err}\nresolve conflicts outside lg, then validate the conflict in lg",
                branch.name
            );
        }
        merged += 1;

        if !branch.upstream_gone
            && let Some((remote, remote_branch)) =
                branch.upstream.as_deref().and_then(upstream_push_target)
        {
            let refspec = format!("refs/heads/{}:refs/heads/{remote_branch}", branch.name);
            match run_combined(&["push", remote, &refspec]) {
                Ok(_) => pushed += 1,
                Err(_) => failed_pushes.push(format!("{remote}/{remote_branch}")),
            }
        } else {
            skipped_push += 1;
        }
    }

    if let Some(original) = original_branch
        && ref_exists(original)
        && head_branch()
            .map(|current| current != original)
            .unwrap_or(true)
    {
        run_combined(&["checkout", original])?;
    }

    let mut summary = format!(
        "merged {base_ref} into {merged} branches, pushed {pushed}, skipped push {skipped_push}"
    );
    if !failed_pushes.is_empty() {
        summary.push_str(&format!(
            ", failed push {} ({})",
            failed_pushes.len(),
            failed_pushes.join(", ")
        ));
    }
    Ok(summary)
}

fn summary_with_stash_notice(mut summary: String, stashed: bool) -> String {
    if stashed {
        summary.push_str(", restored stashed changes");
    }
    summary
}

pub fn flow_merge_main_into_current_with_progress(
    current_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    ensure_merge_main_branch(current_branch)?;
    progress();
    let stashed = stash_uncommitted_changes("lg flow: auto-stash before merging main")?;
    progress();
    let safety_ref = create_safety_ref("merge-main")?;
    progress();
    run(&["fetch"])?;
    progress();
    run(&["checkout", BRANCH_MAIN])?;
    progress();
    run(&["pull", "--rebase", DEFAULT_PUSH_REMOTE, BRANCH_MAIN])?;
    progress();
    run(&["checkout", current_branch])?;
    progress();
    run(&["merge", &format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}")])?;
    progress();
    run(&["push"])?;
    progress();
    pop_stash_if_needed(stashed)?;
    progress();
    delete_safety_ref(&safety_ref)?;
    Ok(format!("merged origin/{BRANCH_MAIN} into {current_branch}"))
}

pub fn flow_release_current(current_branch: &str, target_branch: &str) -> Result<String> {
    flow_release_current_with_progress(current_branch, target_branch, &mut || {})
}

pub fn flow_release_current_with_progress(
    current_branch: &str,
    target_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    ensure_feature_branch(current_branch)?;
    progress();
    let stashed = stash_uncommitted_changes("lg flow: auto-stash before release")?;
    progress();
    let safety_ref = create_safety_ref("release-current")?;
    progress();
    run(&["push", DEFAULT_PUSH_REMOTE, current_branch])?;
    if target_branch != current_branch {
        progress();
        run(&["fetch"])?;
        progress();
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
    progress();
    pop_stash_if_needed(stashed)?;
    delete_safety_ref(&safety_ref)?;

    let env = if target_branch == BRANCH_DEV {
        "dev"
    } else if target_branch == BRANCH_TEST {
        "test"
    } else {
        target_branch
    };
    Ok(format!(
        "released {current_branch} to {target_branch} -> {env}"
    ))
}

pub(super) fn update_release_branch_from_main_before_commit() -> Result<Option<String>> {
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

pub fn flow_create_feature_branch(current_branch: &str, new_branch: &str) -> Result<String> {
    if new_branch.trim().is_empty() {
        anyhow::bail!("branch name cannot be empty");
    }
    if !is_valid_branch_name(new_branch) {
        anyhow::bail!("invalid branch name: {new_branch}");
    }
    let stashed = has_uncommitted_changes()?;
    if stashed {
        run(&[
            "stash",
            "push",
            "-u",
            "-m",
            "lg flow: auto-stash before branch creation",
        ])?;
    }
    run(&["fetch"])?;
    let start_point = if current_branch == BRANCH_MAIN {
        run(&["pull", "--rebase"])?;
        BRANCH_MAIN.to_string()
    } else {
        format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}")
    };
    run(&["checkout", "--no-track", "-b", new_branch, &start_point])?;
    if stashed {
        run(&["stash", "pop"])?;
    }
    Ok(format!("created {new_branch} from {start_point}"))
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

fn is_protected_branch(name: &str) -> bool {
    matches!(name, BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST)
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

fn git_path_exists(name: &str) -> Result<bool> {
    let out = run(&["rev-parse", "--git-path", name])?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(Path::new(&path).exists())
}

fn ensure_feature_branch(branch: &str) -> Result<()> {
    if branch.is_empty() || matches!(branch, BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST) {
        anyhow::bail!(
            "checkout a feature branch first; protected branches: {BRANCH_MAIN}, {BRANCH_DEV}, {BRANCH_TEST}"
        );
    }
    Ok(())
}

fn ensure_merge_main_branch(branch: &str) -> Result<()> {
    if branch.is_empty() || branch == BRANCH_MAIN {
        anyhow::bail!("checkout a feature, {BRANCH_DEV}, or {BRANCH_TEST} branch first");
    }
    Ok(())
}

fn ensure_release_branch(branch: &str) -> Result<()> {
    if !is_release_branch(branch) {
        anyhow::bail!("expected {BRANCH_DEV} or {BRANCH_TEST}, got {branch}");
    }
    Ok(())
}

fn is_release_branch(branch: &str) -> bool {
    matches!(branch, BRANCH_DEV | BRANCH_TEST)
}

fn upstream_push_target(upstream: &str) -> Option<(&str, &str)> {
    let (remote, branch) = upstream.split_once('/')?;
    (!remote.is_empty() && !branch.is_empty()).then_some((remote, branch))
}

fn ref_exists(name: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", name])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn is_valid_branch_name(name: &str) -> bool {
    Command::new("git")
        .args(["check-ref-format", "--branch", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn has_uncommitted_changes() -> Result<bool> {
    let out = run(&["status", "--porcelain"])?;
    Ok(!out.stdout.is_empty())
}

fn stash_before_branch_change(target: &str, message: &str) -> Result<bool> {
    if head_branch().is_ok_and(|current| current == target) {
        return Ok(false);
    }
    stash_uncommitted_changes(message)
}

fn stash_uncommitted_changes(message: &str) -> Result<bool> {
    let stashed = has_uncommitted_changes()?;
    if stashed {
        run(&["stash", "push", "-u", "-m", message])?;
    }
    Ok(stashed)
}

fn pop_stash_if_needed(stashed: bool) -> Result<()> {
    if stashed {
        run(&["stash", "pop"])?;
    }
    Ok(())
}

fn pop_stash_with_index_if_needed(stashed: bool) -> Result<()> {
    if stashed {
        run(&["stash", "pop", "--index"])?;
    }
    Ok(())
}

fn restore_stash_after_failed_checkout(stashed: bool) -> Result<()> {
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
        if matches!(branch, BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST) {
            continue;
        }
        let upstream = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", &format!("{branch}@{{u}}")])
            .output()
            .with_context(|| format!("failed to check upstream for {branch}"))?;
        if !upstream.status.success() {
            orphans.push(branch.to_string());
        }
    }
    Ok(orphans)
}

fn create_safety_ref(label: &str) -> Result<String> {
    let branch = head_branch().unwrap_or_else(|_| "detached".to_string());
    let clean_label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let clean_branch: String = branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{SAFETY_REF_PREFIX}{clean_label}-{clean_branch}-{ts}");
    run(&["branch", &name, "HEAD"])?;
    prune_safety_refs(SAFETY_REF_KEEP)?;
    Ok(name)
}

fn delete_safety_ref(name: &str) -> Result<()> {
    if !name.starts_with(SAFETY_REF_PREFIX) {
        anyhow::bail!("refusing to delete non-safety branch {name}");
    }
    run(&["update-ref", "-d", &format!("refs/heads/{name}")])?;
    Ok(())
}

fn delete_latest_safety_ref(label: &str, branch: &str) -> Result<Option<String>> {
    let prefix = safety_ref_name_prefix(label, branch);
    let out = run(&[
        "for-each-ref",
        "--format=%(refname:short)",
        &format!("refs/heads/{SAFETY_REF_PREFIX}"),
    ])?;
    let latest = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(&prefix))
        .filter_map(|name| safety_ref_timestamp(name).map(|ts| (name.to_string(), ts)))
        .max_by(|(_, a), (_, b)| a.cmp(b))
        .map(|(name, _)| name);

    if let Some(name) = latest {
        delete_safety_ref(&name)?;
        Ok(Some(name))
    } else {
        Ok(None)
    }
}

fn safety_ref_name_prefix(label: &str, branch: &str) -> String {
    let clean_label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let clean_branch: String = branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{SAFETY_REF_PREFIX}{clean_label}-{clean_branch}-")
}

fn prune_safety_refs(keep: usize) -> Result<usize> {
    let out = run(&[
        "for-each-ref",
        "--format=%(refname:short)",
        &format!("refs/heads/{SAFETY_REF_PREFIX}"),
    ])?;
    let mut refs: Vec<(String, u128)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(SAFETY_REF_PREFIX))
        .filter_map(|name| safety_ref_timestamp(name).map(|ts| (name.to_string(), ts)))
        .collect();
    refs.sort_by(|a, b| b.1.cmp(&a.1));

    let mut deleted = 0usize;
    for (name, _) in refs.into_iter().skip(keep) {
        run(&["update-ref", "-d", &format!("refs/heads/{name}")])?;
        deleted += 1;
    }
    Ok(deleted)
}

fn safety_ref_timestamp(name: &str) -> Option<u128> {
    name.strip_prefix(SAFETY_REF_PREFIX)?
        .rsplit_once('-')?
        .1
        .parse()
        .ok()
}
