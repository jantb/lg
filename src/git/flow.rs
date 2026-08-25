use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{
    BRANCH_MAIN, BRANCH_TEST, DEFAULT_PUSH_REMOTE, DEV_BRANCH_NAMES, deploy_branch_list,
    is_deploy_branch_name, is_protected_branch_name, protected_branch_list,
};

use super::{
    counts_ahead_behind, git_command, head_branch, list_branches, parse_worktree_list, run,
    run_combined, run_in_dir, stage,
};

const SAFETY_REF_PREFIX: &str = "lg/backup/";
const SAFETY_REF_KEEP: usize = 20;

/// What lg calls the stashes it takes on the way into a flow. A flow that
/// fails puts its own stash back and a conflict says where it is, so the
/// message has to still be recognisable long after it was written.
const AUTO_STASH_MERGE_MAIN: &str = "lg flow: auto-stash before merging main";
const AUTO_STASH_RELEASE: &str = "lg flow: auto-stash before release";
const AUTO_STASH_NEW_BRANCH: &str = "lg flow: auto-stash before branch creation";
const AUTO_STASH_TAGS: [&str; 2] = ["lg flow: auto-stash", "lg: auto-stash"];

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
    let stashed = stash_uncommitted_changes(AUTO_STASH_MERGE_MAIN)?;
    progress();
    let safety_ref = create_safety_ref("merge-main")?;
    match merge_main_into_current(current_branch, progress) {
        Ok(mut steps) => {
            progress();
            pop_stash_if_needed(stashed)?;
            if stashed {
                steps.push("restored stashed changes".to_string());
            }
            progress();
            delete_safety_ref(&safety_ref)?;
            Ok(steps.join("; "))
        }
        Err(err) => Err(restore_auto_stash_after_failure(
            err,
            stashed,
            AUTO_STASH_MERGE_MAIN,
            current_branch,
        )),
    }
}

/// Merge `main` into the checked-out branch without ever leaving it. Checking
/// `main` out here cannot work from a linked worktree, which is where lg
/// expects the work to happen: git hands the same branch to one checkout at a
/// time, and the main one is already holding it.
fn merge_main_into_current(
    current_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<Vec<String>> {
    let mut steps = Vec::new();
    progress();
    // Being offline is no reason to refuse a local merge, so a failed fetch
    // only means main is merged as the last fetch left it.
    if run(&["fetch"]).is_err() {
        steps.push(format!("could not reach {DEFAULT_PUSH_REMOTE}"));
    }
    progress();
    pull_current_branch_for_merge_main(current_branch)?;
    progress();
    steps.extend(update_local_main()?);
    progress();
    let (base, note) = merge_main_base_ref()?;
    steps.extend(note);
    run_combined(&["merge", "--no-edit", &base])?;
    steps.push(format!("merged {base} into {current_branch}"));
    progress();
    steps.push(push_after_merge_main(current_branch)?);
    Ok(steps)
}

/// Advance the local `main` to the remote's without checking it out here. It
/// may be checked out in another worktree, so the fast-forward runs where it
/// lives; a checkout that will not take it is worth a note rather than a
/// failed merge, because what gets merged below is whichever `main` is ahead.
fn update_local_main() -> Result<Option<String>> {
    let remote_main = format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    if !ref_exists(&remote_main) {
        return Ok(None);
    }
    if !ref_exists(BRANCH_MAIN) {
        run(&["branch", BRANCH_MAIN, &remote_main])?;
        return Ok(Some(format!("created {BRANCH_MAIN} from {remote_main}")));
    }
    if commits_missing_from(BRANCH_MAIN, &remote_main)? == 0 {
        return Ok(None);
    }
    let Some(host) = branch_checkout_dir(BRANCH_MAIN)? else {
        run(&[
            "fetch",
            DEFAULT_PUSH_REMOTE,
            &format!("{BRANCH_MAIN}:{BRANCH_MAIN}"),
        ])?;
        return Ok(Some(format!("updated {BRANCH_MAIN} from {remote_main}")));
    };
    match run_in_dir(&host, &["merge", "--ff-only", &remote_main]) {
        Ok(_) => Ok(Some(format!("updated {BRANCH_MAIN} from {remote_main}"))),
        Err(_) => Ok(Some(format!(
            "left {BRANCH_MAIN} where {} has it",
            host.display()
        ))),
    }
}

/// Which `main` to merge. The local branch and the remote one can hold
/// different commits — a commit made on `main` here, a remote that has moved
/// on — and merging the one that is behind reports success while leaving the
/// branch short of what the branch list says it is missing.
fn merge_main_base_ref() -> Result<(String, Option<String>)> {
    let remote_main = format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    let local = ref_exists(BRANCH_MAIN);
    let remote = ref_exists(&remote_main);
    if !local && !remote {
        anyhow::bail!("could not find {BRANCH_MAIN} or {remote_main}");
    }
    if !local {
        return Ok((remote_main, None));
    }
    if !remote {
        return Ok((BRANCH_MAIN.to_string(), None));
    }
    if commits_missing_from(&remote_main, BRANCH_MAIN)? == 0 {
        return Ok((remote_main, None));
    }
    let behind = commits_missing_from(BRANCH_MAIN, &remote_main)?;
    let note = (behind > 0).then(|| {
        format!(
            "{remote_main} has {behind} commits {BRANCH_MAIN} does not; reconcile those separately"
        )
    });
    Ok((BRANCH_MAIN.to_string(), note))
}

/// Push the merge when the branch has somewhere to go. A branch nobody has
/// published stays local: publishing it as a side effect of a merge is not
/// what the key was pressed for.
fn push_after_merge_main(branch: &str) -> Result<String> {
    if branch_upstream(branch)?.is_none() {
        return Ok(format!("kept {branch} local, it has no upstream"));
    }
    run_combined(&["push"])?;
    Ok(format!("pushed {branch}"))
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

fn pull_current_branch_for_merge_main(current_branch: &str) -> Result<()> {
    if let Ok((ahead, behind)) = counts_ahead_behind() {
        if ahead > 0 && behind > 0 {
            run_combined(&["merge", "--no-edit", "@{u}"])?;
        } else {
            run_combined(&["pull", "--ff-only"])?;
        }
        return Ok(());
    }

    let remote_branch = format!("{DEFAULT_PUSH_REMOTE}/{current_branch}");
    if ref_exists(&remote_branch) {
        run_combined(&["pull", "--ff-only", DEFAULT_PUSH_REMOTE, current_branch])?;
    }
    Ok(())
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

fn is_protected_branch(name: &str) -> bool {
    is_protected_branch_name(name)
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

/// Undo the auto-stash a failed flow took, and say in the error what became
/// of it. Leaving it stashed empties the checkout the user was working in and
/// nothing on screen says why, which is how a flow that stopped half-way looks
/// like lost work.
///
/// A conflicted merge is the one case the stash cannot come back on top of, so
/// that error carries where the work is instead; the conflict modal is showing
/// the whole message by then.
fn restore_auto_stash_after_failure(
    err: anyhow::Error,
    stashed: bool,
    message: &str,
    branch: &str,
) -> anyhow::Error {
    if !stashed {
        return err;
    }
    // The flow may have popped it already and failed on a later step.
    if !matches!(newest_stash_subject(), Some(subject) if subject.contains(message)) {
        return err;
    }
    if merge_or_rebase_in_progress() {
        return anyhow::anyhow!(
            "{err:#}\nyour uncommitted changes are stashed as \"{message}\"; git stash pop brings them back once this is settled"
        );
    }
    // The flow may have stopped on another branch, and the stash belongs to
    // the one it started on.
    if head_branch().map(|head| head != branch).unwrap_or(true)
        && let Err(back) = run(&["checkout", branch])
    {
        return anyhow::anyhow!(
            "your uncommitted changes are stashed as \"{message}\" and {branch} could not be checked out again: {back:#}\n{err:#}"
        );
    }
    match pop_stash_with_index_if_needed(true) {
        Ok(()) => anyhow::anyhow!("{err:#}\nrestored your uncommitted changes"),
        Err(pop) => anyhow::anyhow!(
            "your uncommitted changes are stashed as \"{message}\" and could not be restored: {pop:#}\n{err:#}"
        ),
    }
}

/// Whether git is part-way through something a stash cannot be applied over.
fn merge_or_rebase_in_progress() -> bool {
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

/// Bring back what a flow stashed before it ran into the conflict now being
/// validated. Only lg's own stash is popped, and only while it is the newest
/// one, so a stash the user made themselves is never pulled out from under
/// them.
fn restore_auto_stash_after_conflict() -> Option<String> {
    let subject = newest_stash_subject()?;
    if !AUTO_STASH_TAGS.iter().any(|tag| subject.contains(tag)) {
        return None;
    }
    match pop_stash_with_index_if_needed(true) {
        Ok(()) => Some(format!("restored \"{subject}\"")),
        Err(err) => Some(format!("could not restore \"{subject}\": {err:#}")),
    }
}

fn newest_stash_subject() -> Option<String> {
    let out = run(&["stash", "list", "-1", "--format=%gs"]).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let subject = text.lines().next()?.trim().to_string();
    (!subject.is_empty()).then_some(subject)
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
    refs.sort_by_key(|(_, ts)| std::cmp::Reverse(*ts));

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
