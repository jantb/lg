use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST, DEFAULT_PUSH_REMOTE};

fn trace_enter(label: &str) {
    let Some(path) = std::env::var_os("LG_TRACE") else {
        return;
    };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(f, "ENTER {label}");
}

fn trace(args: &[&str], out: &Output) {
    let Some(path) = std::env::var_os("LG_TRACE") else {
        return;
    };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = writeln!(
        f,
        "git {} -> status={} stdout_bytes={} stderr_bytes={}\n--- stdout ---\n{}\n--- stderr ---\n{}\n---",
        args.join(" "),
        out.status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_owned()),
        stdout.len(),
        stderr.len(),
        stdout,
        stderr,
    );
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ReleaseStatusCacheKey {
    branch: String,
    branch_oid: String,
    base_oid: String,
    develop_oid: Option<String>,
    test_oid: Option<String>,
}

static RELEASE_STATUS_CACHE: OnceLock<Mutex<HashMap<ReleaseStatusCacheKey, BranchReleaseStatus>>> =
    OnceLock::new();

fn release_status_cache() -> &'static Mutex<HashMap<ReleaseStatusCacheKey, BranchReleaseStatus>> {
    RELEASE_STATUS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn run(args: &[&str]) -> Result<Output> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
    trace(args, &out);
    if out.status.success() {
        Ok(out)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(anyhow::anyhow!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        ))
    }
}

fn run_combined(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
    trace(args, &out);
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(anyhow::anyhow!("git {} failed:\n{text}", args.join(" ")))
    }
}

pub fn is_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse `git status -z --porcelain=v1` output.
/// Returns `(unstaged, staged)` — a file may appear in both.
pub fn parse_porcelain(bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let mut unstaged = Vec::new();
    let mut staged = Vec::new();

    // Records are NUL-separated. For rename/copy (R/C) the record contains
    // "XY path" and the *old* path follows as a second NUL-terminated record.
    let mut records: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    // Remove trailing empty entry produced by a trailing NUL.
    if records.last().map(|r| r.is_empty()).unwrap_or(false) {
        records.pop();
    }

    let mut i = 0;
    while i < records.len() {
        let rec = records[i];
        i += 1;

        if rec.len() < 4 {
            // Must be at least "XY p" — skip short/empty records.
            continue;
        }

        let x = rec[0] as char; // index status
        let y = rec[1] as char; // worktree status
        // rec[2] is the space separator; path starts at index 3.
        let path = String::from_utf8_lossy(&rec[3..]).into_owned();

        // Rename/copy: consume the *old* path record (we only show new path).
        if x == 'R' || x == 'C' {
            i += 1; // skip old-path record
        }

        // Index (staged) side: X ∈ {M, A, D, R, C, U} and not ' '.
        if x != ' ' && x != '?' {
            staged.push(path.clone());
        }

        // Worktree (unstaged) side: Y ∈ {M, D, A, ?, U} and not ' '.
        if y != ' ' && y != '.' {
            unstaged.push(path.clone());
        }
    }

    (unstaged, staged)
}

pub fn status_porcelain() -> Result<(Vec<String>, Vec<String>)> {
    let out = run(&["status", "-z", "--porcelain=v1"])?;
    Ok(parse_porcelain(&out.stdout))
}

pub fn stage(path: &str) -> Result<()> {
    run(&["add", "--", path]).map(|_| ())
}

pub fn unstage(path: &str) -> Result<()> {
    // `git reset -q HEAD -- <path>` works even pre-initial-commit (falls back
    // to `git rm --cached` semantics when there is no HEAD).
    let result = run(&["reset", "-q", "HEAD", "--", path]);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            // Pre-initial-commit: "HEAD" doesn't exist yet; use rm --cached.
            let msg = e.to_string();
            if msg.contains("unknown revision") || msg.contains("Failed to resolve") {
                run(&["rm", "--cached", "--", path]).map(|_| ())
            } else {
                Err(e)
            }
        }
    }
}

pub fn stage_all() -> Result<()> {
    run(&["add", "-A"]).map(|_| ())
}

pub fn unstage_all() -> Result<()> {
    let result = run(&["reset", "-q", "HEAD"]);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("unknown revision") || msg.contains("Failed to resolve") {
                // Nothing staged pre-initial-commit; treat as success.
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

pub fn head_branch() -> Result<String> {
    let out = run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn remote_url(name: &str) -> Result<String> {
    let out = run(&["remote", "get-url", name])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn commit(msg: &str) -> Result<String> {
    if msg.trim().is_empty() {
        anyhow::bail!("commit message must not be empty");
    }
    let out = run(&["commit", "-m", msg])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn push(remote: &str, branch: &str) -> Result<String> {
    // Capture both stdout and stderr for the status display.
    let out = Command::new("git")
        .args(["push", remote, branch])
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

pub fn staged_diff() -> Result<String> {
    let out = run(&["diff", "--cached"])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ── New unified types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub x: char,
    pub y: char,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchReleaseStatus {
    pub develop: Option<ReleaseTargetStatus>,
    pub test: Option<ReleaseTargetStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTargetStatus {
    pub released_at: String,
    pub missing_commits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub author: String,
    pub author_short: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictHunk {
    pub ours: String,
    pub base: Option<String>,
    pub theirs: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    Ours,
    Theirs,
    Both,
}

/// Parse `git status -z --porcelain=v1` output into unified `FileEntry` vec.
/// Each entry carries the raw x and y status chars.
pub fn parse_porcelain_xy(bytes: &[u8]) -> Vec<FileEntry> {
    let mut records: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    if records.last().map(|r| r.is_empty()).unwrap_or(false) {
        records.pop();
    }

    let mut entries = Vec::new();
    let mut i = 0;
    while i < records.len() {
        let rec = records[i];
        i += 1;

        if rec.len() < 4 {
            continue;
        }

        let x = rec[0] as char;
        let y = rec[1] as char;
        let path = String::from_utf8_lossy(&rec[3..]).into_owned();

        if x == 'R' || x == 'C' {
            i += 1; // skip old-path record
        }

        entries.push(FileEntry { path, x, y });
    }

    entries
}

pub fn status_entries() -> Result<Vec<FileEntry>> {
    trace_enter("status_entries");
    let out = run(&["status", "-z", "--porcelain=v1"])?;
    Ok(parse_porcelain_xy(&out.stdout))
}

pub fn list_branches() -> Result<Vec<Branch>> {
    trace_enter("list_branches");
    let out = run(&["branch", "--format=%(refname:short)\x1f%(HEAD)"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let branches = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\x1f');
            let name = parts.next()?.trim().to_owned();
            let head = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(Branch {
                name,
                is_current: head == "*",
            })
        })
        .collect();
    Ok(branches)
}

pub fn branch_release_status(branch: &str) -> Result<BranchReleaseStatus> {
    if branch.is_empty() || matches!(branch, BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST) {
        return Ok(BranchReleaseStatus::default());
    }

    let base_ref =
        preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"), BRANCH_MAIN)
            .unwrap_or_else(|| BRANCH_MAIN.to_string());
    let Some(base_oid) = commit_oid(&base_ref) else {
        return Ok(BranchReleaseStatus::default());
    };
    let Some(branch_oid) = commit_oid(branch) else {
        return Ok(BranchReleaseStatus::default());
    };
    let develop_ref =
        preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_DEV}"), BRANCH_DEV);
    let test_ref =
        preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_TEST}"), BRANCH_TEST);
    let key = ReleaseStatusCacheKey {
        branch: branch.to_string(),
        branch_oid,
        base_oid,
        develop_oid: develop_ref.as_deref().and_then(commit_oid),
        test_oid: test_ref.as_deref().and_then(commit_oid),
    };
    if let Ok(cache) = release_status_cache().lock()
        && let Some(status) = cache.get(&key)
    {
        return Ok(status.clone());
    }

    let unique_commits = rev_list(&["--reverse", branch, &format!("^{base_ref}")])?;
    if unique_commits.is_empty() {
        let status = BranchReleaseStatus::default();
        if let Ok(mut cache) = release_status_cache().lock() {
            cache.insert(key, status.clone());
        }
        return Ok(status);
    }

    let status = BranchReleaseStatus {
        develop: release_target_status(branch, &unique_commits, &base_ref, develop_ref.as_deref())?,
        test: release_target_status(branch, &unique_commits, &base_ref, test_ref.as_deref())?,
    };
    if let Ok(mut cache) = release_status_cache().lock() {
        cache.insert(key, status.clone());
    }
    Ok(status)
}

pub fn flow_branches_available() -> bool {
    preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_DEV}"), BRANCH_DEV).is_some()
        && preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_TEST}"), BRANCH_TEST)
            .is_some()
}

fn release_target_status(
    branch: &str,
    unique_commits: &[String],
    base_ref: &str,
    target_ref: Option<&str>,
) -> Result<Option<ReleaseTargetStatus>> {
    let Some(target_ref) = target_ref else {
        return Ok(None);
    };

    let missing = rev_list(&[branch, &format!("^{base_ref}"), "--not", target_ref])?;
    let missing_set: HashSet<&str> = missing.iter().map(String::as_str).collect();
    let latest_released = unique_commits
        .iter()
        .rev()
        .find(|sha| !missing_set.contains(sha.as_str()));

    let Some(latest_released) = latest_released else {
        return Ok(None);
    };

    let released_at = first_containing_commit_date(target_ref, latest_released)
        .or_else(|| commit_date(latest_released).ok())
        .unwrap_or_else(|| "unknown".to_string());
    Ok(Some(ReleaseTargetStatus {
        released_at,
        missing_commits: missing.len(),
    }))
}

pub fn list_commits(limit: usize) -> Result<Vec<Commit>> {
    trace_enter("list_commits");
    let n = limit.to_string();
    let fmt = "--format=%h\x1f%an\x1f%s";
    let result = run(&["log", fmt, "-n", &n]);
    match result {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let commits = text
                .lines()
                .filter_map(|line| {
                    let mut parts = line.splitn(3, '\x1f');
                    let sha = parts.next()?.trim().to_owned();
                    let author = parts.next().unwrap_or("").trim().to_owned();
                    let subject = parts.next().unwrap_or("").trim().to_owned();
                    if sha.is_empty() {
                        return None;
                    }
                    Some(Commit {
                        sha,
                        author_short: short_author_name(&author),
                        author,
                        subject,
                    })
                })
                .collect();
            Ok(commits)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("does not have any commits") || msg.contains("no commits yet") {
                Ok(vec![])
            } else {
                Err(e)
            }
        }
    }
}

fn short_author_name(author: &str) -> String {
    let trimmed = author.trim();
    let first = trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .trim_matches(|c: char| !c.is_alphanumeric());
    let short = if first.is_empty() { trimmed } else { first };
    short.chars().take(12).collect()
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

fn label_commit_patch(text: &str) -> String {
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

pub fn checkout_branch(name: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["checkout", name])
        .output()
        .with_context(|| format!("failed to spawn git checkout {name}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    if out.status.success() {
        Ok(combined)
    } else {
        Err(anyhow::anyhow!("git checkout failed: {}", combined.trim()))
    }
}

pub fn flow_merge_main_into_current(current_branch: &str) -> Result<String> {
    flow_merge_main_into_current_with_progress(current_branch, &mut || {})
}

pub fn flow_merge_main_into_current_with_progress(
    current_branch: &str,
    progress: &mut impl FnMut(),
) -> Result<String> {
    ensure_feature_branch(current_branch)?;
    progress();
    let stashed = stash_uncommitted_changes("lg flow: auto-stash before merging main")?;
    progress();
    create_safety_ref("merge-main")?;
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
    create_safety_ref("release-current")?;
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
    run(&["merge", &format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}")])?;
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
    create_safety_ref(&format!("reset-{target_branch}"))?;
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

pub fn conflict_bundle(files: &[String], validation_log: &str) -> Result<String> {
    let mut out = String::new();
    out.push_str("Merge conflict context for lg.\n\n");
    if !validation_log.trim().is_empty() {
        out.push_str("Validation / previous error log:\n");
        out.push_str(&truncate(validation_log, 12_000));
        out.push_str("\n\n");
    }

    out.push_str("Conflicted files:\n");
    for path in files {
        out.push_str("- ");
        out.push_str(path);
        out.push('\n');
    }
    out.push('\n');

    for path in files {
        out.push_str("===== ");
        out.push_str(path);
        out.push_str(" =====\n");
        match std::fs::read_to_string(path) {
            Ok(contents) => out.push_str(&truncate(&contents, 24_000)),
            Err(e) => out.push_str(&format!("failed to read file: {e}")),
        }
        out.push_str("\n\n");
    }

    if let Ok(diff) = run_combined(&["diff", "--cc"]) {
        out.push_str("Combined diff:\n");
        out.push_str(&truncate(&diff, 24_000));
    }

    Ok(out)
}

pub fn apply_patch_text(patch: &str) -> Result<()> {
    let mut child = Command::new("git")
        .args(["apply", "--whitespace=fix"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn git apply")?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(patch.as_bytes())
            .context("failed to send patch to git apply")?;
    }

    let out = child
        .wait_with_output()
        .context("failed to wait for git apply")?;
    if out.status.success() {
        Ok(())
    } else {
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(anyhow::anyhow!("git apply failed:\n{text}"))
    }
}

pub fn conflict_hunks(path: &str) -> Result<Vec<ConflictHunk>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    Ok(parse_conflict_hunks(&text))
}

pub fn resolve_conflict_hunk(path: &str, idx: usize, choice: ConflictChoice) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let resolved = resolve_conflict_text(&text, idx, choice)
        .ok_or_else(|| anyhow::anyhow!("conflict hunk {idx} not found in {path}"))?;
    std::fs::write(path, resolved).with_context(|| format!("write {path}"))?;
    Ok(())
}

pub fn stage_if_resolved(path: &str) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if has_conflict_markers(&text) {
        anyhow::bail!("conflict markers remain in {path}");
    }
    stage(path)
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

fn parse_conflict_hunks(text: &str) -> Vec<ConflictHunk> {
    let mut hunks = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        if !lines[i].starts_with("<<<<<<<") {
            i += 1;
            continue;
        }
        i += 1;
        let mut ours = Vec::new();
        while i < lines.len()
            && !lines[i].starts_with("|||||||")
            && !lines[i].starts_with("=======")
        {
            ours.push(lines[i]);
            i += 1;
        }

        let mut base = None;
        if i < lines.len() && lines[i].starts_with("|||||||") {
            i += 1;
            let mut base_lines = Vec::new();
            while i < lines.len() && !lines[i].starts_with("=======") {
                base_lines.push(lines[i]);
                i += 1;
            }
            base = Some(join_lines(&base_lines));
        }

        if i < lines.len() && lines[i].starts_with("=======") {
            i += 1;
        }

        let mut theirs = Vec::new();
        while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
            theirs.push(lines[i]);
            i += 1;
        }
        if i < lines.len() && lines[i].starts_with(">>>>>>>") {
            i += 1;
        }

        hunks.push(ConflictHunk {
            ours: join_lines(&ours),
            base,
            theirs: join_lines(&theirs),
        });
    }

    hunks
}

fn resolve_conflict_text(text: &str, target_idx: usize, choice: ConflictChoice) -> Option<String> {
    let mut out = String::new();
    let lines: Vec<&str> = text.lines().collect();
    let had_trailing_newline = text.ends_with('\n');
    let mut i = 0usize;
    let mut hunk_idx = 0usize;
    let mut found = false;

    while i < lines.len() {
        if !lines[i].starts_with("<<<<<<<") {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
            continue;
        }

        let marker_start = i;
        i += 1;
        let ours_start = i;
        while i < lines.len()
            && !lines[i].starts_with("|||||||")
            && !lines[i].starts_with("=======")
        {
            i += 1;
        }
        let ours = &lines[ours_start..i];

        if i < lines.len() && lines[i].starts_with("|||||||") {
            i += 1;
            while i < lines.len() && !lines[i].starts_with("=======") {
                i += 1;
            }
        }

        if i < lines.len() && lines[i].starts_with("=======") {
            i += 1;
        }
        let theirs_start = i;
        while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
            i += 1;
        }
        let theirs = &lines[theirs_start..i];
        if i < lines.len() && lines[i].starts_with(">>>>>>>") {
            i += 1;
        }

        if hunk_idx == target_idx {
            found = true;
            let selected: Vec<&str> = match choice {
                ConflictChoice::Ours => ours.to_vec(),
                ConflictChoice::Theirs => theirs.to_vec(),
                ConflictChoice::Both => ours.iter().chain(theirs.iter()).copied().collect(),
            };
            for line in selected {
                out.push_str(line);
                out.push('\n');
            }
        } else {
            for line in &lines[marker_start..i] {
                out.push_str(line);
                out.push('\n');
            }
        }
        hunk_idx += 1;
    }

    if !had_trailing_newline && out.ends_with('\n') {
        out.pop();
    }
    found.then_some(out)
}

fn join_lines(lines: &[&str]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        let mut s = lines.join("\n");
        s.push('\n');
        s
    }
}

pub fn run_detected_validation() -> Result<String> {
    let commands = detected_validation_commands();
    if commands.is_empty() {
        return Ok(
            "No Cargo.toml or Gradle build files found; no validation command detected.".into(),
        );
    }

    let mut log = String::new();
    for cmd in commands {
        log.push_str("$ ");
        log.push_str(&cmd.join(" "));
        log.push('\n');

        let out = Command::new(&cmd[0])
            .args(&cmd[1..])
            .output()
            .with_context(|| format!("failed to spawn {}", cmd.join(" ")))?;

        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        log.push('\n');

        if !out.status.success() {
            return Err(anyhow::anyhow!("{}", truncate(&log, 32_000)));
        }
    }
    Ok(truncate(&log, 32_000))
}

fn detected_validation_commands() -> Vec<Vec<String>> {
    if let Some(commands) = configured_validation_commands() {
        return commands;
    }

    if Path::new("Cargo.toml").exists() {
        return vec![
            vec!["cargo".into(), "test".into(), "--all-targets".into()],
            vec![
                "cargo".into(),
                "clippy".into(),
                "--all-targets".into(),
                "--".into(),
                "-D".into(),
                "warnings".into(),
            ],
        ];
    }

    if Path::new("build.gradle").exists()
        || Path::new("build.gradle.kts").exists()
        || Path::new("settings.gradle").exists()
        || Path::new("settings.gradle.kts").exists()
    {
        return vec![vec!["gradle".into(), "test".into()]];
    }

    Vec::new()
}

fn configured_validation_commands() -> Option<Vec<Vec<String>>> {
    let text = std::fs::read_to_string(".lg.toml").ok()?;
    let mut in_validation = false;
    let mut commands = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_validation = line == "[validation]";
            continue;
        }
        if !in_validation || !line.starts_with("commands") {
            continue;
        }
        let (_, rhs) = line.split_once('=')?;
        let rhs = rhs.trim();
        let rhs = rhs.strip_prefix('[')?.strip_suffix(']')?;
        for part in rhs.split(',') {
            let cmd = part.trim().trim_matches('"').trim_matches('\'');
            if !cmd.is_empty() {
                commands.push(split_command(cmd));
            }
        }
    }

    (!commands.is_empty()).then_some(commands)
}

fn split_command(cmd: &str) -> Vec<String> {
    cmd.split_whitespace().map(str::to_owned).collect()
}

pub fn continue_in_progress_operation() -> Result<String> {
    continue_in_progress_operation_with_followup(None, None)
}

pub fn continue_in_progress_operation_with_followup(
    push_branch: Option<&str>,
    return_branch: Option<&str>,
) -> Result<String> {
    let staged = stage_resolved_conflicts()?;
    let conflicts = conflicted_files()?;
    if !conflicts.is_empty() {
        anyhow::bail!(
            "unresolved conflicts remain: {}\nResolve the remaining conflict markers, then press c again; lg will stage resolved files automatically.",
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
        out = "no merge, rebase, or cherry-pick operation is in progress".to_string();
    }

    if let Some(branch) = push_branch {
        let push = run_combined(&[
            "push",
            DEFAULT_PUSH_REMOTE,
            &format!("HEAD:refs/heads/{branch}"),
        ])?;
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

    Ok(out)
}

pub fn abort_in_progress_operation() -> Result<String> {
    abort_in_progress_operation_with_return(None)
}

pub fn abort_in_progress_operation_with_return(return_branch: Option<&str>) -> Result<String> {
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

    Ok(out)
}

fn git_path_exists(name: &str) -> Result<bool> {
    let out = run(&["rev-parse", "--git-path", name])?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(Path::new(&path).exists())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}\n... truncated ...", &s[..max])
    }
}

fn ensure_feature_branch(branch: &str) -> Result<()> {
    if branch.is_empty() || matches!(branch, BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST) {
        anyhow::bail!(
            "checkout a feature branch first; protected branches: {BRANCH_MAIN}, {BRANCH_DEV}, {BRANCH_TEST}"
        );
    }
    Ok(())
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

fn preferred_commit_ref(remote_ref: &str, local_ref: &str) -> Option<String> {
    if commit_ref_exists(remote_ref) {
        Some(remote_ref.to_string())
    } else if commit_ref_exists(local_ref) {
        Some(local_ref.to_string())
    } else {
        None
    }
}

fn commit_ref_exists(reference: &str) -> bool {
    run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{reference}^{{commit}}"),
    ])
    .is_ok()
}

fn commit_oid(reference: &str) -> Option<String> {
    let out = run(&["rev-parse", "--verify", &format!("{reference}^{{commit}}")]).ok()?;
    let oid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!oid.is_empty()).then_some(oid)
}

fn rev_list(args: &[&str]) -> Result<Vec<String>> {
    let mut cmd = vec!["rev-list"];
    cmd.extend_from_slice(args);
    let out = run(&cmd)?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn is_ancestor(ancestor: &str, descendant: &str) -> Result<bool> {
    let out = Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .output()
        .with_context(|| {
            format!("failed to spawn git merge-base --is-ancestor {ancestor} {descendant}")
        })?;
    trace(&["merge-base", "--is-ancestor", ancestor, descendant], &out);
    Ok(out.status.success())
}

fn first_containing_commit_date(target_ref: &str, commit: &str) -> Option<String> {
    let commits = rev_list(&["--first-parent", "--reverse", target_ref]).ok()?;
    for target_commit in commits {
        if is_ancestor(commit, &target_commit).ok()? {
            return commit_date(&target_commit).ok();
        }
    }
    None
}

fn commit_date(commit: &str) -> Result<String> {
    let out = run(&[
        "show",
        "-s",
        "--format=%cd",
        "--date=format:%Y-%m-%d %H:%M",
        commit,
    ])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
    let name = format!("lg/backup/{clean_label}-{clean_branch}-{ts}");
    run(&["branch", &name, "HEAD"])?;
    Ok(name)
}

pub fn unpushed_shas() -> Result<std::collections::HashSet<String>> {
    match run(&["rev-list", "--abbrev-commit", "@{u}..HEAD"]) {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            Ok(text
                .lines()
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect())
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("no upstream")
                || msg.contains("unknown revision")
                || msg.contains("ambiguous argument '@{u}'")
            {
                Ok(std::collections::HashSet::new())
            } else {
                Err(e)
            }
        }
    }
}

pub fn counts_ahead_behind() -> Result<(u32, u32)> {
    let out = run(&["rev-list", "--left-right", "--count", "@{u}...HEAD"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    let mut parts = text.splitn(2, '\t');
    let behind: u32 = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("unexpected rev-list output: {text}"))?
        .trim()
        .parse()
        .context("parsing behind count")?;
    let ahead: u32 = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("unexpected rev-list output: {text}"))?
        .trim()
        .parse()
        .context("parsing ahead count")?;
    Ok((ahead, behind))
}

#[cfg(test)]
mod tests {
    use super::{
        ConflictChoice, FileEntry, label_commit_patch, parse_conflict_hunks, parse_porcelain,
        parse_porcelain_xy, resolve_conflict_text,
    };

    #[test]
    fn parse_porcelain_empty() {
        let (u, s) = parse_porcelain(b"");
        assert!(u.is_empty());
        assert!(s.is_empty());
    }

    #[test]
    fn parse_porcelain_untracked() {
        // "?? foo.txt\0"
        let input = b"?? foo.txt\0";
        let (u, s) = parse_porcelain(input);
        assert_eq!(u, vec!["foo.txt"]);
        assert!(s.is_empty());
    }

    #[test]
    fn parse_porcelain_staged_modified() {
        // " M bar.rs\0" — only worktree modified (unstaged only)
        let input = b" M bar.rs\0";
        let (u, s) = parse_porcelain(input);
        assert_eq!(u, vec!["bar.rs"]);
        assert!(s.is_empty());
    }

    #[test]
    fn parse_porcelain_both_modified() {
        // "MM baz.rs\0" — modified in both index and worktree
        let input = b"MM baz.rs\0";
        let (u, s) = parse_porcelain(input);
        assert_eq!(u, vec!["baz.rs"]);
        assert_eq!(s, vec!["baz.rs"]);
    }

    #[test]
    fn parse_porcelain_staged_added() {
        // "A  new.rs\0" — staged new file
        let input = b"A  new.rs\0";
        let (u, s) = parse_porcelain(input);
        assert!(u.is_empty());
        assert_eq!(s, vec!["new.rs"]);
    }

    #[test]
    fn parse_porcelain_rename() {
        // "R  new.rs\0old.rs\0" — rename old→new, staged
        let input = b"R  new.rs\0old.rs\0";
        let (u, s) = parse_porcelain(input);
        // Shows new path in staged; nothing unstaged for a clean rename.
        assert_eq!(s, vec!["new.rs"]);
        assert!(u.is_empty());
    }

    #[test]
    fn parse_porcelain_mixed() {
        // "A  staged.rs\0?? untracked.txt\0MM both.rs\0"
        let input = b"A  staged.rs\0?? untracked.txt\0MM both.rs\0";
        let (u, s) = parse_porcelain(input);
        assert_eq!(s, vec!["staged.rs", "both.rs"]);
        assert_eq!(u, vec!["untracked.txt", "both.rs"]);
    }

    // ── parse_porcelain_xy tests ─────────────────────────────────────────────

    fn fe(path: &str, x: char, y: char) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            x,
            y,
        }
    }

    #[test]
    fn label_commit_patch_inserts_patch_heading() {
        let text = "commit abc\n\nFiles changed:\n a.rs | 1 +\n\ndiff --git a/a.rs b/a.rs\n";
        assert_eq!(
            label_commit_patch(text),
            "commit abc\n\nFiles changed:\n a.rs | 1 +\n\nPatch:\ndiff --git a/a.rs b/a.rs\n"
        );
    }

    #[test]
    fn parse_conflict_hunks_reads_ours_base_theirs() {
        let text =
            "a\n<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> branch\nz\n";
        let hunks = parse_conflict_hunks(text);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].ours, "ours\n");
        assert_eq!(hunks[0].base.as_deref(), Some("base\n"));
        assert_eq!(hunks[0].theirs, "theirs\n");
    }

    #[test]
    fn resolve_conflict_text_accepts_both_for_selected_hunk() {
        let text = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nz\n";
        assert_eq!(
            resolve_conflict_text(text, 0, ConflictChoice::Both).unwrap(),
            "a\nours\ntheirs\nz\n"
        );
    }

    #[test]
    fn parse_porcelain_xy_empty() {
        assert_eq!(parse_porcelain_xy(b""), vec![]);
    }

    #[test]
    fn parse_porcelain_xy_untracked() {
        assert_eq!(
            parse_porcelain_xy(b"?? foo.txt\0"),
            vec![fe("foo.txt", '?', '?')]
        );
    }

    #[test]
    fn parse_porcelain_xy_worktree_modified() {
        assert_eq!(
            parse_porcelain_xy(b" M bar.rs\0"),
            vec![fe("bar.rs", ' ', 'M')]
        );
    }

    #[test]
    fn parse_porcelain_xy_both_modified() {
        assert_eq!(
            parse_porcelain_xy(b"MM baz.rs\0"),
            vec![fe("baz.rs", 'M', 'M')]
        );
    }

    #[test]
    fn parse_porcelain_xy_staged_added() {
        assert_eq!(
            parse_porcelain_xy(b"A  new.rs\0"),
            vec![fe("new.rs", 'A', ' ')]
        );
    }

    #[test]
    fn parse_porcelain_xy_rename() {
        // R  new.rs\0old.rs\0 — should yield one entry with new path, skip old-path record
        let input = b"R  new.rs\0old.rs\0";
        assert_eq!(parse_porcelain_xy(input), vec![fe("new.rs", 'R', ' ')]);
    }

    #[test]
    fn parse_porcelain_xy_mixed() {
        // A  staged.rs\0?? untracked.txt\0MM both.rs\0
        let input = b"A  staged.rs\0?? untracked.txt\0MM both.rs\0";
        let entries = parse_porcelain_xy(input);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], fe("staged.rs", 'A', ' '));
        assert_eq!(entries[1], fe("untracked.txt", '?', '?'));
        assert_eq!(entries[2], fe("both.rs", 'M', 'M'));
    }
}
