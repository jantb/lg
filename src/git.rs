use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
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

pub fn list_commits(limit: usize) -> Result<Vec<Commit>> {
    trace_enter("list_commits");
    let n = limit.to_string();
    let fmt = "--format=%h\x1f%s";
    let result = run(&["log", fmt, "-n", &n]);
    match result {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let commits = text
                .lines()
                .filter_map(|line| {
                    let mut parts = line.splitn(2, '\x1f');
                    let sha = parts.next()?.trim().to_owned();
                    let subject = parts.next().unwrap_or("").trim().to_owned();
                    if sha.is_empty() {
                        return None;
                    }
                    Some(Commit { sha, subject })
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
    ensure_feature_branch(current_branch)?;
    run(&["fetch"])?;
    run(&["checkout", BRANCH_MAIN])?;
    run(&["pull", "--rebase"])?;
    run(&["checkout", current_branch])?;
    run(&["pull", "--rebase"])?;
    run(&["merge", &format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}")])?;
    run(&["push"])?;
    Ok(format!("merged origin/{BRANCH_MAIN} into {current_branch}"))
}

pub fn flow_release_current(current_branch: &str, target_branch: &str) -> Result<String> {
    ensure_feature_branch(current_branch)?;
    run(&["push"])?;
    if target_branch != current_branch {
        reset_local_to_remote(target_branch)?;
    } else {
        run(&["fetch"])?;
        run(&["pull", "--rebase"])?;
    }
    run(&["checkout", target_branch])?;
    run(&["merge", &format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}")])?;
    run(&["merge", &format!("{DEFAULT_PUSH_REMOTE}/{current_branch}")])?;
    run(&["push"])?;
    run(&["checkout", current_branch])?;

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
    run(&["fetch"])?;
    if current_branch != target_branch {
        run(&["checkout", target_branch])?;
    }
    run(&[
        "reset",
        "--hard",
        &format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"),
    ])?;
    run(&["push", "--force"])?;
    if current_branch != target_branch {
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

pub fn continue_in_progress_operation() -> Result<String> {
    let conflicts = conflicted_files()?;
    if !conflicts.is_empty() {
        anyhow::bail!("unresolved conflicts remain: {}", conflicts.join(", "));
    }

    if git_path_exists("rebase-merge")? || git_path_exists("rebase-apply")? {
        return run_combined(&["rebase", "--continue"]);
    }
    if git_path_exists("CHERRY_PICK_HEAD")? {
        return run_combined(&["cherry-pick", "--continue"]);
    }
    if git_path_exists("MERGE_HEAD")? {
        run(&["add", "-A"])?;
        return run_combined(&["commit", "--no-edit"]);
    }
    Ok("no merge, rebase, or cherry-pick operation is in progress".to_string())
}

pub fn abort_in_progress_operation() -> Result<String> {
    if git_path_exists("rebase-merge")? || git_path_exists("rebase-apply")? {
        return run_combined(&["rebase", "--abort"]);
    }
    if git_path_exists("CHERRY_PICK_HEAD")? {
        return run_combined(&["cherry-pick", "--abort"]);
    }
    if git_path_exists("MERGE_HEAD")? {
        return run_combined(&["merge", "--abort"]);
    }
    Ok("no merge, rebase, or cherry-pick operation is in progress".to_string())
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

fn reset_local_to_remote(branch: &str) -> Result<()> {
    run(&["fetch"])?;
    run(&[
        "branch",
        "-f",
        branch,
        &format!("{DEFAULT_PUSH_REMOTE}/{branch}"),
    ])
    .map(|_| ())
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
