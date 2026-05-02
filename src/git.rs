use anyhow::{Context, Result};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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

const SAFETY_REF_PREFIX: &str = "lg/backup/";
const SAFETY_REF_KEEP: usize = 20;

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
    pub upstream: Option<String>,
    pub upstream_gone: bool,
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
    pub graph: String,
    pub is_first_parent: bool,
    pub parent_count: usize,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorConfig {
    pub name: Option<String>,
    pub email: Option<String>,
    pub local_name: Option<String>,
    pub local_email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewFile {
    status: String,
    path: String,
    old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewEntryPoint {
    path: String,
    line: Option<usize>,
    symbol: String,
    description: String,
    hunk: String,
    patch: Vec<String>,
    context: Vec<String>,
    added: usize,
    removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistedReview {
    pub report: String,
    pub nodes: Vec<ReviewNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNode {
    pub id: String,
    pub parent: Option<String>,
    pub depth: u16,
    pub title: String,
    pub body: Vec<String>,
    pub context: Vec<String>,
}

struct ReviewRender<'a> {
    branch: &'a str,
    base_ref: &'a str,
    merge_base: &'a str,
    commits: &'a [String],
    files: &'a [ReviewFile],
    stat: &'a str,
    entries: &'a [ReviewEntryPoint],
    diff: &'a str,
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
    let out = run(&[
        "branch",
        "--format=%(refname:short)\x1f%(HEAD)\x1f%(upstream:short)\x1f%(upstream:track)",
    ])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let branches = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\x1f');
            let name = parts.next()?.trim().to_owned();
            let head = parts.next()?.trim();
            let upstream = parts.next().unwrap_or("").trim();
            let track = parts.next().unwrap_or("").trim();
            if name.is_empty() {
                return None;
            }
            Some(Branch {
                name,
                is_current: head == "*",
                upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
                upstream_gone: track.contains("gone"),
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
    list_commits_for_ref("HEAD", limit)
}

pub fn list_commits_for_ref(reference: &str, limit: usize) -> Result<Vec<Commit>> {
    trace_enter("list_commits");
    let n = limit.to_string();
    let first_parent = first_parent_shas(reference, limit).unwrap_or_default();
    let fmt = "--format=%x1f%h%x1f%an%x1f%P%x1f%s";
    let result = run(&["log", "--graph", fmt, "-n", &n, reference]);
    match result {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let commits = text
                .lines()
                .filter_map(|line| {
                    let marker = line.find('\x1f')?;
                    let graph = line[..marker].to_owned();
                    let mut parts = line[marker + 1..].splitn(4, '\x1f');
                    let sha = parts.next()?.trim().to_owned();
                    let author = parts.next().unwrap_or("").trim().to_owned();
                    let parents = parts.next().unwrap_or("").trim();
                    let subject = parts.next().unwrap_or("").trim().to_owned();
                    if sha.is_empty() {
                        return None;
                    }
                    let is_first_parent = first_parent.contains(&sha);
                    Some(Commit {
                        sha,
                        author_short: short_author_name(&author),
                        author,
                        graph,
                        is_first_parent,
                        parent_count: parents.split_whitespace().count(),
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

pub fn author_config() -> Result<AuthorConfig> {
    Ok(AuthorConfig {
        name: config_value(&["config", "--get", "user.name"])?,
        email: config_value(&["config", "--get", "user.email"])?,
        local_name: config_value(&["config", "--local", "--get", "user.name"])?,
        local_email: config_value(&["config", "--local", "--get", "user.email"])?,
    })
}

pub fn repo_root() -> Result<String> {
    let out = run(&["rev-parse", "--show-toplevel"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn set_local_author(name: &str, email: &str) -> Result<()> {
    set_optional_local_config("user.name", name)?;
    set_optional_local_config("user.email", email)?;
    Ok(())
}

pub fn set_subtree_author(path: &str, name: &str, email: &str) -> Result<()> {
    let path = normalize_author_path(path)?;
    let config_path = subtree_author_config_path(&path)?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = format!(
        "[user]\n\tname = {}\n\temail = {}\n",
        escape_config_value(name.trim()),
        escape_config_value(email.trim())
    );
    fs::write(&config_path, text)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    let key = subtree_include_key(&path);
    let config_path_str = config_path.to_string_lossy().to_string();
    run(&["config", "--global", &key, &config_path_str]).map(|_| ())
}

pub fn clear_subtree_author(path: &str) -> Result<()> {
    let path = normalize_author_path(path)?;
    let key = subtree_include_key(&path);
    let out = Command::new("git")
        .args(["config", "--global", "--unset-all", &key])
        .output()
        .with_context(|| format!("failed to spawn git config --global --unset-all {key}"))?;
    trace(&["config", "--global", "--unset-all", &key], &out);
    if let Ok(config_path) = subtree_author_config_path(&path) {
        let _ = fs::remove_file(config_path);
    }
    Ok(())
}

pub fn subtree_author_rule_exists(path: &str) -> bool {
    normalize_author_path(path)
        .map(|path| subtree_include_key(&path))
        .ok()
        .and_then(|key| config_value(&["config", "--global", "--get", &key]).ok())
        .flatten()
        .is_some()
}

pub fn clear_local_author() -> Result<()> {
    unset_optional_local_config("user.name")?;
    unset_optional_local_config("user.email")?;
    Ok(())
}

fn normalize_author_path(path: &str) -> Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        anyhow::bail!("author folder is empty");
    }
    let expanded = if trimmed == "~" {
        home_dir()?
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        home_dir()?.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        Ok(std::env::current_dir()?.join(expanded))
    }
}

fn subtree_include_key(path: &Path) -> String {
    let mut path = path.to_string_lossy().trim_end_matches('/').to_string();
    path.push_str("/**");
    format!("includeIf.gitdir:{path}.path")
}

fn subtree_author_config_path(path: &Path) -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config/lg/git-author")
        .join(format!("{}.gitconfig", author_path_slug(path))))
}

fn author_path_slug(path: &Path) -> String {
    let slug: String = path
        .to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    slug.trim_matches('-').to_string()
}

fn escape_config_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', " ")
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

fn set_optional_local_config(key: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        unset_optional_local_config(key)
    } else {
        run(&["config", "--local", key, value]).map(|_| ())
    }
}

fn unset_optional_local_config(key: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["config", "--local", "--unset-all", key])
        .output()
        .with_context(|| format!("failed to spawn git config --local --unset-all {key}"))?;
    trace(&["config", "--local", "--unset-all", key], &out);
    Ok(())
}

fn config_value(args: &[&str]) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
    trace(args, &out);
    if !out.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!value.is_empty()).then_some(value))
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

pub fn assisted_review_against_main() -> Result<String> {
    Ok(build_assisted_review_against_main()?.report)
}

pub fn build_assisted_review_against_main() -> Result<AssistedReview> {
    let base_ref =
        preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"), BRANCH_MAIN)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not find {DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN} or {BRANCH_MAIN}"
                )
            })?;
    let branch = head_branch().unwrap_or_else(|_| "HEAD".to_string());
    let range = format!("{base_ref}...HEAD");

    let merge_base = run(&["merge-base", &base_ref, "HEAD"])
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    let commits = branch_review_commits(&base_ref)?;
    let files = branch_review_files(&range)?;
    let stat = run(&[
        "diff",
        "--ignore-all-space",
        "--stat",
        "--find-renames",
        &range,
    ])
    .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    .unwrap_or_default();
    let diff = run(&["diff", "--ignore-all-space", "--find-renames", &range])
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())?;
    let entries = review_entry_points(&diff);

    let render = ReviewRender {
        branch: &branch,
        base_ref: &base_ref,
        merge_base: &merge_base,
        commits: &commits,
        files: &files,
        stat: &stat,
        entries: &entries,
        diff: &diff,
    };
    let report = render_assisted_review(&render);
    let nodes = build_review_nodes(&render);

    Ok(AssistedReview { report, nodes })
}

fn branch_review_commits(base_ref: &str) -> Result<Vec<String>> {
    let range = format!("{base_ref}..HEAD");
    let out = run(&["log", "--oneline", "--decorate=no", &range])?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn branch_review_files(range: &str) -> Result<Vec<ReviewFile>> {
    let out = run(&[
        "diff",
        "--ignore-all-space",
        "--name-status",
        "--find-renames",
        range,
    ])?;
    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 && parts[0].starts_with('R') {
            files.push(ReviewFile {
                status: parts[0].to_string(),
                path: parts[2].to_string(),
                old_path: Some(parts[1].to_string()),
            });
        } else if parts.len() >= 2 {
            files.push(ReviewFile {
                status: parts[0].to_string(),
                path: parts[1].to_string(),
                old_path: None,
            });
        }
    }
    Ok(files)
}

fn review_entry_points(diff: &str) -> Vec<ReviewEntryPoint> {
    let mut entries = Vec::new();
    let mut current_path = String::new();
    let mut current_hunk: Option<ReviewHunk> = None;

    for line in diff.lines() {
        if let Some(path) = parse_review_diff_path(line) {
            flush_review_hunk(&mut entries, &current_path, current_hunk.take());
            current_path = path;
            continue;
        }
        if line.starts_with("@@") {
            flush_review_hunk(&mut entries, &current_path, current_hunk.take());
            let new_line = parse_new_hunk_start(line).unwrap_or(0);
            current_hunk = Some(ReviewHunk {
                start_line: new_line,
                current_line: new_line,
                first_added_line: None,
                hunk: line.to_string(),
                patch: vec![line.to_string()],
                added: 0,
                removed: 0,
            });
            continue;
        }
        if let Some(hunk) = current_hunk.as_mut() {
            hunk.patch.push(line.to_string());
            if line.starts_with('+') && !line.starts_with("+++") {
                hunk.added += 1;
                hunk.first_added_line.get_or_insert(hunk.current_line);
                hunk.current_line = hunk.current_line.saturating_add(1);
            } else if line.starts_with('-') && !line.starts_with("---") {
                hunk.removed += 1;
            } else if !line.starts_with('\\') {
                hunk.current_line = hunk.current_line.saturating_add(1);
            }
        }
    }
    flush_review_hunk(&mut entries, &current_path, current_hunk.take());
    entries
}

struct ReviewHunk {
    start_line: usize,
    current_line: usize,
    first_added_line: Option<usize>,
    hunk: String,
    patch: Vec<String>,
    added: usize,
    removed: usize,
}

fn parse_review_diff_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let (_, b_path) = rest.split_once(" b/")?;
    Some(b_path.to_owned())
}

fn flush_review_hunk(entries: &mut Vec<ReviewEntryPoint>, path: &str, hunk: Option<ReviewHunk>) {
    let Some(hunk) = hunk else {
        return;
    };
    if path.is_empty() {
        return;
    }
    if is_import_only_hunk(path, &hunk.patch) {
        return;
    }
    let line = hunk.first_added_line.unwrap_or(hunk.start_line);
    let symbol = infer_entry_symbol(path, line, &hunk.hunk);
    let context = source_context(path, line);
    let description = describe_hunk(&hunk.patch, hunk.added, hunk.removed);
    entries.push(ReviewEntryPoint {
        path: path.to_string(),
        line: (line > 0).then_some(line),
        symbol,
        description,
        hunk: hunk.hunk,
        patch: hunk.patch,
        context,
        added: hunk.added,
        removed: hunk.removed,
    });
}

fn is_import_only_hunk(path: &str, patch: &[String]) -> bool {
    let mut changed = 0usize;
    for line in patch
        .iter()
        .filter(|line| line.starts_with('+') || line.starts_with('-'))
        .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
    {
        let body = line[1..].trim();
        if body.is_empty() {
            continue;
        }
        changed += 1;
        if !is_import_line(path, body) {
            return false;
        }
    }
    changed > 0
}

fn is_import_line(path: &str, line: &str) -> bool {
    let line = line
        .strip_prefix("pub ")
        .or_else(|| line.strip_prefix("public "))
        .unwrap_or(line);
    if path.ends_with(".rs") {
        line.starts_with("use ") || line.starts_with("extern crate ")
    } else if matches_kotlin_path(path) || path.ends_with(".java") {
        line.starts_with("import ") || line.starts_with("package ")
    } else {
        line.starts_with("import ") || line.starts_with("from ") || line.starts_with("export ")
    }
}

fn describe_hunk(patch: &[String], added: usize, removed: usize) -> String {
    let operation = match (added > 0, removed > 0) {
        (true, true) => "updates",
        (true, false) => "adds",
        (false, true) => "removes",
        (false, false) => "touches",
    };
    let mut signals = Vec::new();
    for line in patch
        .iter()
        .filter(|line| line.starts_with('+') || line.starts_with('-'))
        .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
    {
        collect_signal_words(&line[1..], &mut signals);
        if signals.len() >= 4 {
            break;
        }
    }
    if signals.is_empty() {
        format!("{operation} this block (+{added} -{removed})")
    } else {
        format!(
            "{operation} {} (+{added} -{removed})",
            signals.into_iter().take(4).collect::<Vec<_>>().join(", ")
        )
    }
}

fn collect_signal_words(line: &str, signals: &mut Vec<String>) {
    let trimmed = line.trim();
    if trimmed.is_empty() || matches!(trimmed, "{" | "}" | ");" | ")" | "]") {
        return;
    }
    for word in trimmed
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map(str::trim)
        .filter(|word| word.chars().count() >= 3)
        .filter(|word| !is_low_signal_word(word))
    {
        let word = truncate_review_text(word, 32);
        if !signals.contains(&word) {
            signals.push(word);
        }
        if signals.len() >= 4 {
            return;
        }
    }
}

fn is_low_signal_word(word: &str) -> bool {
    matches!(
        word,
        "let"
            | "mut"
            | "pub"
            | "fn"
            | "impl"
            | "self"
            | "Some"
            | "None"
            | "true"
            | "false"
            | "String"
            | "Vec"
            | "format"
            | "return"
            | "val"
            | "var"
            | "fun"
            | "class"
            | "object"
    )
}

fn parse_new_hunk_start(line: &str) -> Option<usize> {
    let plus = line.find(" +")? + 2;
    let rest = &line[plus..];
    let end = rest
        .find(|c: char| c == ',' || c.is_whitespace())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn infer_entry_symbol(path: &str, line: usize, hunk: &str) -> String {
    if path.ends_with(".rs") {
        if let Some(symbol) = infer_rust_symbol(path, line) {
            return symbol;
        }
    }
    if matches_kotlin_path(path) {
        if let Some(symbol) = infer_kotlin_symbol(path, line) {
            return symbol;
        }
    }
    if let Some(symbol) = hunk_symbol(hunk) {
        return symbol;
    }
    "file scope".to_string()
}

fn hunk_symbol(hunk: &str) -> Option<String> {
    let symbol = hunk.rsplit("@@").next()?.trim();
    if symbol.is_empty()
        || symbol == "where"
        || symbol.starts_with("use ")
        || symbol.starts_with("impl ")
    {
        return None;
    }
    Some(truncate_review_text(symbol, 96))
}

fn infer_rust_symbol(path: &str, line: usize) -> Option<String> {
    infer_source_symbol(path, line, rust_item_label)
}

fn infer_kotlin_symbol(path: &str, line: usize) -> Option<String> {
    infer_source_symbol(path, line, kotlin_item_label)
}

fn infer_source_symbol(
    path: &str,
    line: usize,
    label: fn(&str) -> Option<String>,
) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let target = line.saturating_sub(1);
    let lines: Vec<&str> = text.lines().collect();
    let start = target.saturating_sub(160);
    for raw in lines
        .get(start..=target.min(lines.len().saturating_sub(1)))?
        .iter()
        .rev()
    {
        let trimmed = raw.trim_start();
        if let Some(symbol) = label(trimmed) {
            return Some(symbol);
        }
    }
    None
}

fn matches_kotlin_path(path: &str) -> bool {
    path.ends_with(".kt") || path.ends_with(".kts")
}

fn rust_item_label(line: &str) -> Option<String> {
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    for prefix in [
        "async fn ",
        "fn ",
        "impl ",
        "trait ",
        "struct ",
        "enum ",
        "mod ",
        "const ",
        "static ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| c == '(' || c == '<' || c == ':' || c == '{' || c.is_whitespace())
                .next()
                .unwrap_or(rest)
                .trim();
            if !name.is_empty() {
                return Some(format!("{} {name}", prefix.trim_end()));
            }
        }
    }
    None
}

fn kotlin_item_label(line: &str) -> Option<String> {
    let line = line
        .strip_prefix("private ")
        .or_else(|| line.strip_prefix("internal "))
        .or_else(|| line.strip_prefix("protected "))
        .or_else(|| line.strip_prefix("public "))
        .unwrap_or(line);
    let line = line
        .strip_prefix("suspend ")
        .or_else(|| line.strip_prefix("inline "))
        .unwrap_or(line);
    for prefix in [
        "fun ",
        "class ",
        "data class ",
        "sealed class ",
        "enum class ",
        "object ",
        "interface ",
        "companion object",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| c == '(' || c == '<' || c == ':' || c == '{' || c.is_whitespace())
                .next()
                .unwrap_or(rest)
                .trim();
            let label = prefix.trim_end();
            if prefix == "companion object" {
                return Some(label.to_string());
            }
            if !name.is_empty() {
                return Some(format!("{label} {name}"));
            }
        }
    }
    None
}

fn source_context(path: &str, line: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let target = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    let start = find_source_item_start(path, &lines, target).unwrap_or(target.saturating_sub(8));
    let end = find_source_item_end(&lines, start)
        .unwrap_or_else(|| target.saturating_add(24).min(lines.len().saturating_sub(1)));

    lines[start..=end]
        .iter()
        .enumerate()
        .map(|(idx, text)| format!("{:>5} | {}", start + idx + 1, text))
        .collect()
}

fn find_source_item_start(path: &str, lines: &[&str], target: usize) -> Option<usize> {
    let start = target.saturating_sub(160);
    for (idx, raw) in lines.iter().enumerate().take(target + 1).skip(start).rev() {
        let trimmed = raw.trim_start();
        let is_item = if path.ends_with(".rs") {
            rust_item_label(trimmed).is_some()
        } else if matches_kotlin_path(path) {
            kotlin_item_label(trimmed).is_some()
        } else {
            false
        };
        if is_item {
            return Some(idx);
        }
    }
    None
}

fn find_source_item_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut balance = 0isize;
    let mut saw_open = false;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '{' => {
                    balance += 1;
                    saw_open = true;
                }
                '}' => balance -= 1,
                _ => {}
            }
        }
        if saw_open && balance <= 0 {
            return Some(idx);
        }
        if !saw_open && idx > start && line.trim().is_empty() {
            return Some(idx.saturating_sub(1));
        }
    }
    (!lines.is_empty()).then_some(lines.len() - 1)
}

fn build_review_nodes(review: &ReviewRender<'_>) -> Vec<ReviewNode> {
    let mut nodes = Vec::new();
    push_entry_nodes(
        &mut nodes,
        "branch",
        "Full diff against main",
        review.entries,
        review.diff.trim().is_empty(),
    );
    nodes.push(ReviewNode {
        id: "summary".to_string(),
        parent: None,
        depth: 0,
        title: format!(
            "Summary: {} vs {} ({} commits, {} files)",
            review.branch,
            review.base_ref,
            review.commits.len(),
            review.files.len()
        ),
        body: effect_summary(review.files, review.entries, review.commits),
        context: Vec::new(),
    });
    nodes.push(ReviewNode {
        id: "checklist".to_string(),
        parent: None,
        depth: 0,
        title: "Review checklist".to_string(),
        body: review_checklist(review.files, review.entries),
        context: Vec::new(),
    });
    nodes
}

fn push_entry_nodes(
    nodes: &mut Vec<ReviewNode>,
    prefix: &str,
    title: &str,
    entries: &[ReviewEntryPoint],
    empty: bool,
) {
    let root_id = prefix.to_string();
    nodes.push(ReviewNode {
        id: root_id.clone(),
        parent: None,
        depth: 0,
        title: title.to_string(),
        body: if empty {
            vec!["(empty)".to_string()]
        } else if entries.is_empty() {
            vec!["(only import changes hidden)".to_string()]
        } else {
            Vec::new()
        },
        context: Vec::new(),
    });
    if entries.is_empty() {
        return;
    }
    let groups = entry_groups(entries);
    let parents = entry_group_parents(entries, &groups);
    let tree = EntryTree {
        prefix,
        entries,
        groups: &groups,
        parents: &parents,
    };
    let mut emitted = BTreeSet::new();
    for (path, group_indices) in groups_by_path(
        groups
            .iter()
            .enumerate()
            .filter(|(group_idx, _)| parents[*group_idx].is_none())
            .map(|(group_idx, _)| group_idx),
        &groups,
    ) {
        tree.push_file(nodes, &path, &group_indices, &root_id, 1, &mut emitted);
    }
    for (group_idx, group) in groups.iter().enumerate() {
        tree.push_file(nodes, &group.path, &[group_idx], &root_id, 1, &mut emitted);
    }
}

struct EntryTree<'a> {
    prefix: &'a str,
    entries: &'a [ReviewEntryPoint],
    groups: &'a [EntryGroup],
    parents: &'a [Option<usize>],
}

impl EntryTree<'_> {
    fn push_file(
        &self,
        nodes: &mut Vec<ReviewNode>,
        path: &str,
        group_indices: &[usize],
        parent_id: &str,
        depth: u16,
        emitted: &mut BTreeSet<usize>,
    ) {
        let pending: Vec<usize> = group_indices
            .iter()
            .copied()
            .filter(|group_idx| !emitted.contains(group_idx))
            .collect();
        if pending.is_empty() {
            return;
        }

        let file_id = format!("{}:file:{}", self.prefix, pending[0]);
        nodes.push(ReviewNode {
            id: file_id.clone(),
            parent: Some(parent_id.to_string()),
            depth,
            title: self.file_title(path),
            body: self.file_patch_body(path),
            context: Vec::new(),
        });
        for group_idx in pending {
            self.push_group(nodes, group_idx, &file_id, depth.saturating_add(1), emitted);
        }
    }

    fn push_group(
        &self,
        nodes: &mut Vec<ReviewNode>,
        group_idx: usize,
        parent_id: &str,
        depth: u16,
        emitted: &mut BTreeSet<usize>,
    ) {
        if !emitted.insert(group_idx) {
            return;
        }

        let group = &self.groups[group_idx];
        let group_id = format!("{}:entry:{group_idx}", self.prefix);
        nodes.push(ReviewNode {
            id: group_id.clone(),
            parent: Some(parent_id.to_string()),
            depth,
            title: format!(
                "{} in {} - {}",
                group.path,
                group.symbol,
                entry_group_description(self.entries, group)
            ),
            body: self.group_patch_body(group),
            context: Vec::new(),
        });
        for idx in &group.indices {
            let entry = &self.entries[*idx];
            let location = entry
                .line
                .map(|line| format!(":{line}"))
                .unwrap_or_default();
            nodes.push(ReviewNode {
                id: format!("{}:hunk:{idx}", self.prefix),
                parent: Some(group_id.clone()),
                depth: depth.saturating_add(1),
                title: format!("{}{} - {}", entry.path, location, entry.description),
                body: std::iter::once(format!("effect: {}", entry.description))
                    .chain(entry.patch.iter().cloned())
                    .collect(),
                context: entry.context.clone(),
            });
        }

        for child_idx in 0..self.groups.len() {
            if self.parents[child_idx] == Some(group_idx) {
                if self.groups[child_idx].path == group.path {
                    self.push_group(
                        nodes,
                        child_idx,
                        &group_id,
                        depth.saturating_add(1),
                        emitted,
                    );
                } else {
                    self.push_file(
                        nodes,
                        &self.groups[child_idx].path,
                        &[child_idx],
                        &group_id,
                        depth.saturating_add(1),
                        emitted,
                    );
                }
            }
        }
    }

    fn file_title(&self, path: &str) -> String {
        let mut entry_count = 0usize;
        let mut added = 0usize;
        let mut removed = 0usize;
        for group in self.groups.iter().filter(|group| group.path == path) {
            entry_count += 1;
            added += group
                .indices
                .iter()
                .map(|idx| self.entries[*idx].added)
                .sum::<usize>();
            removed += group
                .indices
                .iter()
                .map(|idx| self.entries[*idx].removed)
                .sum::<usize>();
        }
        let noun = if entry_count == 1 {
            "entry point"
        } else {
            "entry points"
        };
        format!("{path} - {entry_count} {noun} (+{added} -{removed})")
    }

    fn file_patch_body(&self, path: &str) -> Vec<String> {
        self.groups
            .iter()
            .filter(|group| group.path == path)
            .flat_map(|group| {
                group
                    .indices
                    .iter()
                    .flat_map(|idx| self.entries[*idx].patch.iter().cloned())
            })
            .collect()
    }

    fn group_patch_body(&self, group: &EntryGroup) -> Vec<String> {
        group
            .indices
            .iter()
            .flat_map(|idx| self.entries[*idx].patch.iter().cloned())
            .collect()
    }
}

struct EntryGroup {
    path: String,
    symbol: String,
    indices: Vec<usize>,
}

fn entry_group_description(entries: &[ReviewEntryPoint], group: &EntryGroup) -> String {
    let added: usize = group.indices.iter().map(|idx| entries[*idx].added).sum();
    let removed: usize = group.indices.iter().map(|idx| entries[*idx].removed).sum();
    if group.indices.len() == 1 {
        entries[group.indices[0]].description.clone()
    } else {
        format!(
            "{} hunks update this entry point (+{added} -{removed})",
            group.indices.len()
        )
    }
}

fn entry_groups(entries: &[ReviewEntryPoint]) -> Vec<EntryGroup> {
    let mut groups = Vec::<EntryGroup>::new();
    for (idx, entry) in entries.iter().enumerate() {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.path == entry.path && group.symbol == entry.symbol)
        {
            group.indices.push(idx);
        } else {
            groups.push(EntryGroup {
                path: entry.path.clone(),
                symbol: entry.symbol.clone(),
                indices: vec![idx],
            });
        }
    }
    groups
}

fn groups_by_path<I>(indices: I, groups: &[EntryGroup]) -> Vec<(String, Vec<usize>)>
where
    I: IntoIterator<Item = usize>,
{
    let mut files = Vec::<(String, Vec<usize>)>::new();
    for idx in indices {
        let path = &groups[idx].path;
        if let Some((_, group_indices)) = files.iter_mut().find(|(candidate, _)| candidate == path)
        {
            group_indices.push(idx);
        } else {
            files.push((path.clone(), vec![idx]));
        }
    }
    files
}

fn entry_group_parents(entries: &[ReviewEntryPoint], groups: &[EntryGroup]) -> Vec<Option<usize>> {
    groups
        .iter()
        .enumerate()
        .map(|(callee_idx, callee)| {
            let callable = callable_symbol_name(&callee.symbol)?;
            groups
                .iter()
                .enumerate()
                .filter(|(caller_idx, _)| *caller_idx != callee_idx)
                .filter(|(_, caller)| entry_group_references(entries, caller, &callable))
                .map(|(caller_idx, _)| caller_idx)
                .next()
        })
        .collect()
}

fn entry_group_references(
    entries: &[ReviewEntryPoint],
    group: &EntryGroup,
    callable: &str,
) -> bool {
    group
        .indices
        .iter()
        .any(|idx| patch_references_callable(&entries[*idx].patch, callable))
}

fn patch_references_callable(patch: &[String], callable: &str) -> bool {
    patch
        .iter()
        .filter_map(|line| {
            line.strip_prefix('+')
                .or_else(|| line.strip_prefix(' '))
                .or_else(|| line.strip_prefix('-'))
        })
        .any(|line| line_references_callable(line, callable))
}

fn line_references_callable(line: &str, callable: &str) -> bool {
    line.match_indices(callable).any(|(idx, _)| {
        let before = line[..idx].chars().next_back();
        let after = line[idx + callable.len()..].chars().next();
        !before.is_some_and(is_ident_continue) && !after.is_some_and(is_ident_continue)
    })
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn callable_symbol_name(symbol: &str) -> Option<String> {
    let rest = symbol
        .strip_prefix("fn ")
        .or_else(|| symbol.strip_prefix("async fn "))
        .or_else(|| symbol.strip_prefix("fun "))?;
    let name = rest
        .split(|ch: char| ch == '(' || ch == '<' || ch == ':' || ch == '{' || ch.is_whitespace())
        .next()
        .unwrap_or(rest)
        .trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn render_assisted_review(review: &ReviewRender<'_>) -> String {
    let mut out = String::new();
    out.push_str("Assisted review against main\n");
    out.push_str("============================\n\n");
    out.push_str(&format!("Branch: {}\n", review.branch));
    out.push_str(&format!("Base: {}\n", review.base_ref));
    if !review.merge_base.is_empty() {
        out.push_str(&format!("Merge base: {}\n", short_oid(review.merge_base)));
    }
    out.push_str(&format!(
        "Scope: {} commit{}, {} file{}\n",
        review.commits.len(),
        plural(review.commits.len()),
        review.files.len(),
        plural(review.files.len())
    ));
    out.push_str("\nEffect summary\n");
    out.push_str("--------------\n");
    for line in effect_summary(review.files, review.entries, review.commits) {
        out.push_str("- ");
        out.push_str(&line);
        out.push('\n');
    }

    if !review.commits.is_empty() {
        out.push_str("\nCommits in review range\n");
        out.push_str("-----------------------\n");
        for commit in review.commits.iter().take(24) {
            out.push_str("- ");
            out.push_str(commit);
            out.push('\n');
        }
        if review.commits.len() > 24 {
            out.push_str(&format!(
                "- ... {} more commits\n",
                review.commits.len() - 24
            ));
        }
    }

    out.push_str("\nFiles changed\n");
    out.push_str("-------------\n");
    if review.files.is_empty() {
        out.push_str("- No committed branch diff against main.\n");
    } else {
        for file in review.files {
            out.push_str("- ");
            out.push_str(&file.status);
            out.push(' ');
            if let Some(old) = &file.old_path {
                out.push_str(old);
                out.push_str(" -> ");
            }
            out.push_str(&file.path);
            out.push('\n');
        }
    }

    if !review.stat.trim().is_empty() {
        out.push_str("\nDiffstat\n");
        out.push_str("--------\n");
        out.push_str(review.stat.trim_end());
        out.push('\n');
    }

    out.push_str("\nEntry point trace\n");
    out.push_str("-----------------\n");
    if review.entries.is_empty() {
        out.push_str("- No patch hunks found in the branch diff.\n");
    } else {
        render_entry_points(&mut out, review.entries);
    }
    out.push_str("\nReview checklist\n");
    out.push_str("----------------\n");
    for line in review_checklist(review.files, review.entries) {
        out.push_str("- ");
        out.push_str(&line);
        out.push('\n');
    }

    out.push_str("\nFull diff against main\n");
    out.push_str("----------------------\n");
    if review.diff.trim().is_empty() {
        out.push_str("(empty)\n");
    } else {
        out.push_str(review.diff.trim_end());
        out.push('\n');
    }
    out
}

fn effect_summary(
    files: &[ReviewFile],
    entries: &[ReviewEntryPoint],
    commits: &[String],
) -> Vec<String> {
    let mut lines = Vec::new();
    if files.is_empty() {
        lines.push("No committed branch changes were found against main.".to_string());
    } else {
        lines.push(format!(
            "The branch changes {} file{} across {} commit{}.",
            files.len(),
            plural(files.len()),
            commits.len(),
            plural(commits.len())
        ));
    }

    let mut areas = Vec::new();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();

    if paths.contains(&"src/app.rs") {
        areas.push("runtime orchestration and keyboard/job handling");
    }
    if paths.contains(&"src/state.rs") {
        areas.push("application state");
    }
    if paths.contains(&"src/git.rs") {
        areas.push("Git integration");
    }
    if paths.iter().any(|path| path.starts_with("src/panel/")) {
        areas.push("terminal UI panels");
    }
    if paths.iter().any(|path| path.starts_with("tests/")) {
        areas.push("test coverage");
    }
    if paths
        .iter()
        .any(|path| matches!(*path, "Cargo.toml" | "Cargo.lock" | "Makefile"))
    {
        areas.push("build or dependency configuration");
    }
    if !areas.is_empty() {
        lines.push(format!("Primary touched areas: {}.", areas.join(", ")));
    }

    let mut symbols: Vec<String> = entries
        .iter()
        .filter(|entry| entry.symbol != "file scope")
        .map(|entry| entry.symbol.clone())
        .collect();
    symbols.sort();
    symbols.dedup();
    if !symbols.is_empty() {
        lines.push(format!(
            "Start tracing at: {}{}.",
            symbols
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            if symbols.len() > 8 { ", ..." } else { "" }
        ));
    }

    lines
}

fn render_entry_points(out: &mut String, entries: &[ReviewEntryPoint]) {
    let mut last_path = "";
    for entry in entries {
        if entry.path != last_path {
            out.push_str(&format!("\n{}\n", entry.path));
            last_path = &entry.path;
        }
        let location = entry
            .line
            .map(|line| format!(":{}", line))
            .unwrap_or_default();
        out.push_str(&format!(
            "- {}{} in {} - {}\n",
            entry.path, location, entry.symbol, entry.description
        ));
        out.push_str("  ");
        out.push_str(&truncate_review_text(&entry.hunk, 140));
        out.push('\n');
    }
}

fn review_checklist(files: &[ReviewFile], entries: &[ReviewEntryPoint]) -> Vec<String> {
    let mut lines = Vec::new();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();

    if paths.contains(&"src/git.rs") {
        lines.push(
            "Verify Git commands on a temporary repository before trusting the workflow."
                .to_string(),
        );
    }
    if paths.contains(&"src/app.rs") || paths.contains(&"src/state.rs") {
        lines.push(
            "Check state transitions and background jobs for stale output or focus changes."
                .to_string(),
        );
    }
    if paths.iter().any(|path| path.starts_with("src/panel/")) {
        lines.push(
            "Exercise the affected keybindings and render at narrow terminal widths.".to_string(),
        );
    }
    if !paths.iter().any(|path| path.starts_with("tests/")) && !entries.is_empty() {
        lines.push(
            "No test files changed; consider adding coverage for the user-visible flow."
                .to_string(),
        );
    }
    if lines.is_empty() {
        lines.push(
            "Review the entry point trace and diffstat, then run the standard test command."
                .to_string(),
        );
    }
    lines
}

fn truncate_review_text(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn short_oid(oid: &str) -> &str {
    oid.get(..12).unwrap_or(oid)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
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
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        let push = run_combined(&["push", DEFAULT_PUSH_REMOTE, &refspec])?;
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
    use super::{FileEntry, label_commit_patch, parse_porcelain, parse_porcelain_xy};

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
