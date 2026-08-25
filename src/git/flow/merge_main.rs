//! Merging main into a branch, one branch or all of them.

use anyhow::Result;

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE};

use super::super::{
    counts_ahead_behind, head_branch, list_branches, run, run_combined, run_in_dir,
};
use super::*;

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
