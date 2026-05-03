use anyhow::Result;

use super::{run, run_combined};

pub fn staged_diff() -> Result<String> {
    let out = run(&["diff", "--cached"])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn repo_root() -> Result<String> {
    let out = run(&["rev-parse", "--show-toplevel"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn fetch_updates() -> Result<String> {
    let remotes = run(&["remote"])?;
    if String::from_utf8_lossy(&remotes.stdout).trim().is_empty() {
        return Ok("no remotes configured".to_string());
    }

    let text = run_combined(&["fetch", "--all", "--prune"])?;
    let status = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_owned())
        .unwrap_or_else(|| "fetched branch updates".to_string());
    Ok(status)
}

pub fn all_diffs() -> Result<String> {
    let cached_out = run(&["diff", "--cached"])?;
    let worktree_out = run(&["diff"])?;
    let cached = String::from_utf8_lossy(&cached_out.stdout).into_owned();
    let worktree = String::from_utf8_lossy(&worktree_out.stdout).into_owned();
    Ok(format!(
        "== staged (--cached) ==\n{}\n== worktree ==\n{}",
        cached, worktree
    ))
}

pub fn file_diff(path: &str) -> Result<String> {
    let cached_out = run(&["diff", "--cached", "--", path])?;
    let worktree_out = run(&["diff", "--", path])?;
    let cached = String::from_utf8_lossy(&cached_out.stdout).into_owned();
    let worktree = String::from_utf8_lossy(&worktree_out.stdout).into_owned();
    Ok(format!(
        "== staged (--cached) ==\n{}\n== worktree ==\n{}",
        cached, worktree
    ))
}

/// Aggregate staged + worktree diff for everything under a folder prefix.
pub fn folder_diff(prefix: &str) -> Result<String> {
    let spec = if prefix.is_empty() {
        ".".to_string()
    } else {
        format!("{prefix}/")
    };
    let cached_out = run(&["diff", "--cached", "--", &spec])?;
    let worktree_out = run(&["diff", "--", &spec])?;
    let cached = String::from_utf8_lossy(&cached_out.stdout).into_owned();
    let worktree = String::from_utf8_lossy(&worktree_out.stdout).into_owned();
    Ok(format!(
        "== staged (--cached) {prefix}/ ==\n{}\n== worktree {prefix}/ ==\n{}",
        cached, worktree
    ))
}

pub fn show_commit(sha: &str) -> Result<String> {
    let format = "format:commit %H%nAuthor: %an <%ae>%nDate:   %ad%n%nMessage:%n%B%nFiles changed:";
    let out = run(&[
        "show",
        "--date=short",
        "--patch-with-stat",
        "--find-renames",
        "--root",
        &format!("--format={format}"),
        sha,
    ])?;
    Ok(label_commit_patch(&String::from_utf8_lossy(&out.stdout)))
}

pub fn branch_log(reference: &str, limit: usize) -> Result<String> {
    let n = limit.to_string();
    let out = run(&[
        "log",
        "--graph",
        "--decorate",
        "--date=relative",
        "--abbrev-commit",
        "-n",
        &n,
        reference,
        "--",
    ])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub(super) fn label_commit_patch(text: &str) -> String {
    if let Some(pos) = text.find("\ndiff --git ") {
        let mut out = String::with_capacity(text.len() + "\nPatch:\n".len());
        out.push_str(text[..pos].trim_end());
        out.push_str("\n\nPatch:\n");
        out.push_str(&text[pos + 1..]);
        out
    } else {
        text.to_owned()
    }
}
