//! Talking to the remote: pushing, pulling, and the stash that keeps it safe.

use anyhow::Result;

use super::{counts_ahead_behind, run, run_combined};

/// What a fetch of every remote came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// The repository has no remote, so there was nothing to fetch.
    NoRemotes,
    /// The last line git printed, or a stand-in when it printed nothing.
    Fetched(String),
}

pub fn fetch_updates() -> Result<FetchOutcome> {
    let remotes = run(&["remote"])?;
    if String::from_utf8_lossy(&remotes.stdout).trim().is_empty() {
        return Ok(FetchOutcome::NoRemotes);
    }

    let text = run_combined(&["fetch", "--all", "--prune"])?;
    let status = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_owned())
        .unwrap_or_else(|| "fetched branch updates".to_string());
    Ok(FetchOutcome::Fetched(status))
}

pub fn remote_url(name: &str) -> Result<String> {
    let out = run(&["remote", "get-url", name])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Fetch before an operation that compares the branch with its remote.
///
/// A fetch that fails does not stop the operation: the remote-tracking refs
/// already here are still the best answer there is. But the comparison was then
/// made against the remote as it was at the last fetch that worked, and the
/// result has to say so — `Some` is the line that does.
fn fetch_before(operation: &str) -> Option<String> {
    let err = fetch_updates().err()?;
    let reason = err.to_string();
    let reason = reason
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("unknown error");
    Some(format!(
        "warning: fetch before {operation} failed ({reason}); compared against possibly stale remote refs"
    ))
}

/// `text` with the stale-refs warning, if there is one, as its last line —
/// the line the status bar shows.
fn with_fetch_warning(mut text: String, warning: Option<&str>) -> String {
    if let Some(warning) = warning {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(warning);
        text.push('\n');
    }
    text
}

pub fn push(remote: &str, branch: &str) -> Result<String> {
    let stale = fetch_before("push");
    let stale = stale.as_deref();
    if let Ok((ahead, behind)) = counts_ahead_behind() {
        if ahead > 0 && behind > 0 {
            anyhow::bail!(
                "{}",
                with_fetch_warning(
                    "branch diverged from remote; merge upstream before pushing".into(),
                    stale
                )
                .trim_end()
            );
        }
        if behind > 0 {
            anyhow::bail!(
                "{}",
                with_fetch_warning("branch is behind remote; pull before pushing".into(), stale)
                    .trim_end()
            );
        }
    }

    // Not timed: see `TIMED_SUBCOMMANDS`.
    match run_combined(&["push", remote, branch]) {
        Ok(text) => Ok(with_fetch_warning(text, stale)),
        Err(err) => Err(anyhow::anyhow!(
            "{}",
            with_fetch_warning(err.to_string(), stale).trim()
        )),
    }
}

/// Push `branch` to `remote` and have it track what it was pushed to, for a
/// branch the remote may not have seen yet.
pub fn push_with_upstream(remote: &str, branch: &str) -> Result<String> {
    if branch.trim().is_empty() {
        anyhow::bail!("branch name must not be empty");
    }
    run_combined(&["push", "--set-upstream", remote, branch])
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
    let stale = fetch_before("pull");
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
            Ok(with_fetch_warning(out, stale.as_deref()))
        }
        Err(err) => {
            let mut message = err.to_string();
            if stashed {
                message.push_str("\nauto-stashed local changes were left in stash");
            }
            Err(anyhow::anyhow!(
                "{}",
                with_fetch_warning(message, stale.as_deref()).trim_end()
            ))
        }
    }
}

pub fn merge_upstream() -> Result<String> {
    let stale = fetch_before("merge");
    match run_combined(&["merge", "--no-edit", "@{u}"]) {
        Ok(out) => Ok(with_fetch_warning(out, stale.as_deref())),
        Err(err) => Err(anyhow::anyhow!(
            "{}",
            with_fetch_warning(err.to_string(), stale.as_deref()).trim_end()
        )),
    }
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

/// What a push with `--force-with-lease` of the current branch would replace on
/// its upstream, read from the remote-tracking ref as the last fetch left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForcePushPlan {
    /// The local branch being pushed.
    pub branch: String,
    /// The remote it goes to, as configured for the branch.
    pub remote: String,
    /// The branch's name on that remote.
    pub remote_branch: String,
    /// The commit the remote branch is expected to be at. The push is refused
    /// if it has moved on since, which is the lease.
    pub expected: String,
    /// Commits on the remote branch that HEAD does not have: what the push
    /// throws away there.
    pub overwritten: Vec<String>,
}

impl ForcePushPlan {
    /// `origin/feature`, the way the remote branch is named on screen.
    pub fn remote_ref(&self) -> String {
        format!("{}/{}", self.remote, self.remote_branch)
    }
}

/// How a force push of the current branch over its upstream would go, without
/// pushing anything. Refuses the branches lg never rewrites.
pub fn force_push_plan() -> Result<ForcePushPlan> {
    let branch = super::head_branch()?;
    let config = |key: &str| {
        run(&["config", "--get", &format!("branch.{branch}.{key}")])
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let remote = config("remote")
        .filter(|remote| remote != ".")
        .ok_or_else(|| anyhow::anyhow!("{branch} has no upstream on a remote to force-push to"))?;
    let merge = config("merge")
        .ok_or_else(|| anyhow::anyhow!("{branch} has no upstream on a remote to force-push to"))?;
    let remote_branch = merge
        .strip_prefix("refs/heads/")
        .unwrap_or(&merge)
        .to_string();
    for name in [&branch, &remote_branch] {
        if crate::config::is_protected_branch_name(name)
            || crate::config::is_deploy_branch_name(name)
        {
            anyhow::bail!("{name} is a protected branch; lg never force-pushes it");
        }
    }
    let out = run(&["rev-parse", "--verify", "--quiet", "@{u}"])
        .map_err(|_| anyhow::anyhow!("{remote}/{remote_branch} has not been fetched"))?;
    let expected = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let out = run(&["log", "--format=%h %s", "HEAD..@{u}"])?;
    let overwritten = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    Ok(ForcePushPlan {
        branch,
        remote,
        remote_branch,
        expected,
        overwritten,
    })
}

/// Push the current branch over its upstream, but only while the upstream is
/// still at `plan.expected`. Never a plain `--force`: a remote branch someone
/// pushed to since the plan was made is left alone and the push fails.
pub fn push_force_with_lease(plan: &ForcePushPlan) -> Result<String> {
    for name in [&plan.branch, &plan.remote_branch] {
        if crate::config::is_protected_branch_name(name)
            || crate::config::is_deploy_branch_name(name)
        {
            anyhow::bail!("{name} is a protected branch; lg never force-pushes it");
        }
    }
    if super::head_branch()? != plan.branch {
        anyhow::bail!("{} is no longer checked out", plan.branch);
    }
    let lease = format!(
        "--force-with-lease=refs/heads/{}:{}",
        plan.remote_branch, plan.expected
    );
    let refspec = format!(
        "refs/heads/{}:refs/heads/{}",
        plan.branch, plan.remote_branch
    );
    // Not timed: see `TIMED_SUBCOMMANDS`.
    let out = run_combined(&["push", &lease, &plan.remote, &refspec])?;
    Ok(out
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("force-pushed")
        .trim()
        .to_string())
}
