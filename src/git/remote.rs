//! Talking to the remote: pushing, pulling, and the stash that keeps it safe.

use anyhow::{Context, Result};

use super::{counts_ahead_behind, fetch_updates, git_command, run, run_combined};

pub fn remote_url(name: &str) -> Result<String> {
    let out = run(&["remote", "get-url", name])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn push(remote: &str, branch: &str) -> Result<String> {
    let _ = fetch_updates();
    if let Ok((ahead, behind)) = counts_ahead_behind() {
        if ahead > 0 && behind > 0 {
            anyhow::bail!("branch diverged from remote; merge upstream before pushing");
        }
        if behind > 0 {
            anyhow::bail!("branch is behind remote; pull before pushing");
        }
    }

    // Capture both stdout and stderr for the status display.
    let out = git_command(&["push", remote, branch])
        .output()
        .context("failed to spawn git push")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    if out.status.success() {
        Ok(combined)
    } else {
        Err(anyhow::anyhow!("git push failed: {}", combined.trim()))
    }
}

pub fn set_branch_upstream(branch: &str, upstream: &str) -> Result<String> {
    if branch.trim().is_empty() {
        anyhow::bail!("branch name must not be empty");
    }
    if upstream.trim().is_empty() {
        anyhow::bail!("upstream name must not be empty");
    }
    run(&["branch", "--set-upstream-to", upstream, branch])?;
    Ok(format!("{branch} tracks {upstream}"))
}

pub fn pull(remote: &str, branch: &str) -> Result<String> {
    if branch.trim().is_empty() {
        anyhow::bail!("branch name must not be empty");
    }
    let _ = fetch_updates();
    let stashed = stash_uncommitted_changes("lg: auto-stash before pull")?;
    let res = if let Ok((ahead, behind)) = counts_ahead_behind()
        && ahead > 0
        && behind > 0
    {
        run_combined(&["merge", "--no-edit", "@{u}"])
    } else {
        run_combined(&["pull", "--ff-only", remote, branch])
    };

    match res {
        Ok(mut out) => {
            pop_stash_with_index_if_needed(stashed)?;
            if stashed {
                out.push_str("applied stashed local changes after pull\n");
            }
            Ok(out)
        }
        Err(err) => {
            if stashed {
                Err(anyhow::anyhow!(
                    "{err}\nauto-stashed local changes were left in stash"
                ))
            } else {
                Err(err)
            }
        }
    }
}

pub fn merge_upstream() -> Result<String> {
    let _ = fetch_updates();
    run_combined(&["merge", "--no-edit", "@{u}"])
}

fn has_uncommitted_changes() -> Result<bool> {
    let out = run(&["status", "--porcelain"])?;
    Ok(!out.stdout.is_empty())
}

fn stash_uncommitted_changes(message: &str) -> Result<bool> {
    let stashed = has_uncommitted_changes()?;
    if stashed {
        run(&["stash", "push", "-u", "-m", message])?;
    }
    Ok(stashed)
}

fn pop_stash_with_index_if_needed(stashed: bool) -> Result<()> {
    if stashed {
        run(&["stash", "pop", "--index"])?;
    }
    Ok(())
}
