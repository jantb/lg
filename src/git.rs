use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};

use crate::config::{BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST, DEFAULT_PUSH_REMOTE};

mod config;
mod flow;
mod review;

pub use config::{
    AuthorConfig, IdeOpenCommand, add_to_gitignore, author_config, clear_local_author,
    clear_subtree_author, ide_open_command, open_file_in_ide, open_project_in_ide,
    project_open_command, set_local_author, set_subtree_author, subtree_author_rule_exists,
};
pub use flow::{
    abort_in_progress_operation, abort_in_progress_operation_with_return, checkout_branch,
    checkout_remote_branch, conflicted_files, delete_local_branch, delete_remote_branch,
    flow_clean_orphan_branches, flow_create_feature_branch, flow_merge_main_into_current,
    flow_merge_main_into_current_with_progress, flow_release_current,
    flow_release_current_with_progress, flow_reset_branch_from_main,
    flow_reset_branch_from_main_with_progress, stage_resolved_conflicts,
    validate_conflict_resolution_with_followup,
};
pub use review::{
    AssistedReview, ReviewNode, assisted_review_against_main, build_assisted_review_against_main,
};

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

pub fn pull(remote: &str, branch: &str) -> Result<String> {
    if branch.trim().is_empty() {
        anyhow::bail!("branch name must not be empty");
    }
    run_combined(&["pull", "--ff-only", remote, branch])
}

pub fn staged_diff() -> Result<String> {
    let out = run(&["diff", "--cached"])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

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
    pub upstream: Option<String>,
    pub upstream_gone: bool,
    pub last_commit_unix: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranch {
    pub name: String,
    pub remote: String,
    pub local_name: String,
    pub last_commit_unix: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchReleaseStatus {
    pub main: Option<ReleaseTargetStatus>,
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
    pub parents: Vec<String>,
    pub is_first_parent: bool,
    pub subject: String,
}

impl Commit {
    pub fn parent_count(&self) -> usize {
        self.parents.len()
    }

    pub fn is_graph_row(&self) -> bool {
        false
    }
}

impl crate::graph::CommitNode for Commit {
    fn sha(&self) -> &str {
        &self.sha
    }
    fn parents(&self) -> &[String] {
        &self.parents
    }
    fn is_first_parent(&self) -> bool {
        self.is_first_parent
    }
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
    let out = run(&["status", "-z", "--porcelain=v1"])?;
    Ok(parse_porcelain_xy(&out.stdout))
}

pub fn list_branches() -> Result<Vec<Branch>> {
    let out = run(&[
        "branch",
        "--format=%(refname:short)\x1f%(HEAD)\x1f%(upstream:short)\x1f%(upstream:track)\x1f%(committerdate:unix)",
    ])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let branches = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\x1f');
            let name = parts.next()?.trim().to_owned();
            let head = parts.next()?.trim();
            let upstream = parts.next().unwrap_or("").trim();
            let track = parts.next().unwrap_or("").trim();
            let last_commit_unix = parse_unix_timestamp(parts.next().unwrap_or("").trim());
            if name.is_empty() {
                return None;
            }
            Some(Branch {
                name,
                is_current: head == "*",
                upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
                upstream_gone: track.contains("gone"),
                last_commit_unix,
            })
        })
        .collect();
    Ok(branches)
}

pub fn list_remote_branches() -> Result<Vec<RemoteBranch>> {
    let out = run(&[
        "for-each-ref",
        "refs/remotes",
        "--format=%(refname:short)\x1f%(committerdate:unix)",
    ])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let branches = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\x1f');
            let name = parts.next()?.trim().to_owned();
            if name.is_empty() || name.ends_with("/HEAD") {
                return None;
            }
            let (remote, local_name) = name.split_once('/')?;
            let remote = remote.to_owned();
            let local_name = local_name.to_owned();
            Some(RemoteBranch {
                name,
                remote,
                local_name,
                last_commit_unix: parse_unix_timestamp(parts.next().unwrap_or("").trim()),
            })
        })
        .collect();
    Ok(branches)
}

fn parse_unix_timestamp(value: &str) -> Option<i64> {
    value.parse::<i64>().ok().filter(|ts| *ts > 0)
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
        // Branch tip is reachable from main (regular or rebase merge); record
        // the merge date so the deployment panel can show it as merged.
        let released_at = first_containing_commit_date(&base_ref, branch)
            .or_else(|| commit_date(branch).ok())
            .unwrap_or_else(|| "unknown".to_string());
        let status = BranchReleaseStatus {
            main: Some(ReleaseTargetStatus {
                released_at,
                missing_commits: 0,
            }),
            develop: None,
            test: None,
        };
        if let Ok(mut cache) = release_status_cache().lock() {
            cache.insert(key, status.clone());
        }
        return Ok(status);
    }

    let status = BranchReleaseStatus {
        main: None,
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
    list_commits_for_ref("HEAD", limit)
}

pub fn list_commits_for_ref(reference: &str, limit: usize) -> Result<Vec<Commit>> {
    let n = limit.to_string();
    let first_parent = first_parent_shas(reference, limit).unwrap_or_default();
    let fmt = "--format=%x1f%h%x1f%an%x1f%p%x1f%s";
    let result = run(&["log", fmt, "-n", &n, reference]);
    match result {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut commits = Vec::new();
            for line in text.lines() {
                let Some(marker) = line.find('\x1f') else {
                    continue;
                };
                let mut parts = line[marker + 1..].splitn(4, '\x1f');
                let Some(sha) = parts.next().map(str::trim).map(str::to_owned) else {
                    continue;
                };
                if sha.is_empty() {
                    continue;
                }
                let author = parts.next().unwrap_or("").trim().to_owned();
                let parents_str = parts.next().unwrap_or("").trim();
                let subject = parts.next().unwrap_or("").trim().to_owned();
                let parents: Vec<String> =
                    parents_str.split_whitespace().map(str::to_owned).collect();
                let is_first_parent = first_parent.contains(&sha);
                commits.push(Commit {
                    sha,
                    author_short: short_author_name(&author),
                    author,
                    parents,
                    is_first_parent,
                    subject,
                });
            }
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

fn first_parent_shas(reference: &str, limit: usize) -> Result<HashSet<String>> {
    let n = limit.to_string();
    let out = run(&[
        "rev-list",
        "--first-parent",
        "--abbrev-commit",
        "-n",
        &n,
        reference,
    ])?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn short_author_name(author: &str) -> String {
    let trimmed = author.trim();
    let parts: Vec<&str> = trimmed
        .split_whitespace()
        .map(|part| part.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() >= 2 {
        return parts
            .iter()
            .take(2)
            .filter_map(|part| part.chars().next())
            .flat_map(char::to_uppercase)
            .take(2)
            .collect();
    }
    parts
        .first()
        .copied()
        .unwrap_or(trimmed)
        .chars()
        .take(2)
        .collect()
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

fn first_containing_commit_date(target_ref: &str, commit: &str) -> Option<String> {
    let first_parent = rev_list(&["--first-parent", "--reverse", target_ref]).ok()?;
    if first_parent
        .iter()
        .any(|target_commit| target_commit == commit)
    {
        return commit_date(commit).ok();
    }

    let range = format!("{commit}..{target_ref}");
    let containing_path =
        rev_list(&["--first-parent", "--reverse", "--ancestry-path", &range]).ok()?;
    containing_path
        .first()
        .and_then(|target_commit| commit_date(target_commit).ok())
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
    use super::config::{build_ide_open_command, diff_hunk_start_line};
    use super::{FileEntry, label_commit_patch, parse_porcelain, parse_porcelain_xy};

    #[test]
    fn parse_porcelain_empty() {
        let (u, s) = parse_porcelain(b"");
        assert!(u.is_empty());
        assert!(s.is_empty());
    }

    #[test]
    fn ide_open_command_uses_jetbrains_launcher_for_source_type() {
        let kotlin =
            build_ide_open_command("/repo", "src/main/kotlin/App.kt", 42).expect("kotlin command");
        assert_eq!(kotlin.program, "idea");
        assert_eq!(
            kotlin.args,
            vec!["/repo", "--line", "42", "/repo/src/main/kotlin/App.kt"]
        );

        let rust = build_ide_open_command("/repo", "src/main.rs", 7).expect("rust command");
        assert_eq!(rust.program, "rustrover");
        assert_eq!(rust.args, vec!["/repo", "--line", "7", "/repo/src/main.rs"]);

        assert!(build_ide_open_command("/repo", "README.md", 1).is_none());
    }

    #[test]
    fn diff_hunk_start_line_reads_changed_line_number() {
        assert_eq!(diff_hunk_start_line("@@ -12,3 +42,8 @@ fun main"), Some(42));
        assert_eq!(diff_hunk_start_line("@@ -9,0 +0,0 @@ removed"), Some(9));
        assert_eq!(diff_hunk_start_line("diff --git a/a b/a"), None);
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
