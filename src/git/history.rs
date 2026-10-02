//! Rewriting and replaying history one commit at a time: amending the last
//! commit, reverting one, cherry-picking one onto the current branch.

use anyhow::Result;

use super::{run, run_combined};

/// The full object name of `rev`.
pub fn full_sha(rev: &str) -> Result<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = run(&["rev-parse", "--verify", "--quiet", &spec])?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        anyhow::bail!("{rev} names no commit");
    }
    Ok(sha)
}

/// The last commit's whole message, subject and body.
pub fn head_commit_message() -> Result<String> {
    let out = run(&["log", "-1", "--format=%B", "HEAD"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// A remote branch that already has HEAD, when there is one: amending HEAD
/// then leaves that branch with a commit the local one no longer has, and
/// pushing again needs a force push. The upstream is named first when it is
/// one of them.
pub fn head_published_on() -> Option<String> {
    let out = run(&[
        "branch",
        "-r",
        "--contains",
        "HEAD",
        "--format=%(refname:short)",
    ])
    .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let remotes: Vec<&str> = text
        .lines()
        .map(str::trim)
        // `origin/HEAD` is a pointer to another branch, not a branch.
        .filter(|name| !name.is_empty() && !name.ends_with("/HEAD") && *name != "origin")
        .collect();
    let upstream = run(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
    upstream
        .filter(|upstream| remotes.contains(&upstream.as_str()))
        .or_else(|| remotes.first().map(|name| (*name).to_string()))
}

/// Replace the last commit with one holding what is staged now, under
/// `message`.
pub fn amend_commit(message: &str) -> Result<String> {
    if message.trim().is_empty() {
        anyhow::bail!("commit message must not be empty");
    }
    run(&["rev-parse", "--verify", "--quiet", "HEAD"])
        .map_err(|_| anyhow::anyhow!("there is no commit to amend yet"))?;
    let out = run(&["commit", "--amend", "-m", message])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Refuse a merge commit: replaying one needs a mainline chosen, and picking
/// the wrong one silently undoes or repeats the other side.
fn refuse_merge_commit(sha: &str, what: &str) -> Result<()> {
    let spec = format!("{sha}^2");
    if run(&["rev-parse", "--verify", "--quiet", &spec]).is_ok() {
        anyhow::bail!("{sha} is a merge commit; {what} a merge is not offered here");
    }
    Ok(())
}

/// Add a commit that undoes `sha`, with git's own message for it. A conflict
/// stops with the revert in progress, for the conflict modal to settle.
pub fn revert_commit(sha: &str) -> Result<String> {
    super::flow::ensure_no_conflict_in_progress()?;
    refuse_merge_commit(sha, "reverting")?;
    replay(&["revert", "--no-edit", sha], "REVERT_HEAD", "revert", sha)
}

/// Replay `sha` on top of the current branch. A conflict stops with the
/// cherry-pick in progress, for the conflict modal to settle.
pub fn cherry_pick_commit(sha: &str) -> Result<String> {
    super::flow::ensure_no_conflict_in_progress()?;
    refuse_merge_commit(sha, "cherry-picking")?;
    let spec = format!("{sha}^{{commit}}");
    if run(&["merge-base", "--is-ancestor", &spec, "HEAD"]).is_ok() {
        anyhow::bail!("{sha} is already on the current branch");
    }
    replay(
        &["cherry-pick", sha],
        "CHERRY_PICK_HEAD",
        "cherry-pick",
        sha,
    )
}

/// Run a revert or cherry-pick. One that conflicts is left in progress for the
/// conflict modal. One that stops for any other reason — most often because
/// the change is already here and the commit would be empty — is backed out,
/// so the checkout is not left part-way through something nobody can see.
fn replay(args: &[&str], marker: &str, command: &str, sha: &str) -> Result<String> {
    match run_combined(args) {
        Ok(out) => Ok(out),
        Err(err) => {
            let conflicted = !super::conflicted_files().unwrap_or_default().is_empty();
            if !conflicted && super::flow::git_path_exists(marker).unwrap_or(false) {
                let _ = run_combined(&[command, "--abort"]);
                let text = err.to_string();
                if text.contains("empty") || text.contains("nothing to commit") {
                    anyhow::bail!("{command} of {sha} changes nothing: its change is already here");
                }
            }
            Err(err)
        }
    }
}
