//! Repositories checked out inside this one, and checking out branches in them.

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

use super::{git_command_in_dir, repo_root, run_in_dir, worktree::same_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedRepo {
    pub path: String,
    pub branch: Option<String>,
    pub detached_at: Option<String>,
    pub has_changes: bool,
    /// The nested repository this directory is a linked worktree of, as a
    /// workspace-relative path, when its main checkout is in the workspace
    /// too. A worktree is not a repository of its own and is listed under the
    /// one it belongs to.
    pub worktree_of: Option<String>,
}

pub fn nested_repositories() -> Result<Vec<NestedRepo>> {
    let root = PathBuf::from(repo_root()?);
    nested_repositories_at(&root)
}

pub fn nested_repositories_at(root: &Path) -> Result<Vec<NestedRepo>> {
    let mut dirs = Vec::new();
    collect_nested_repo_dirs(root, root, &mut dirs);
    dirs.sort();

    let mut repos = dirs
        .into_iter()
        .map(|dir| nested_repo_status(root, &dir))
        .collect::<Result<Vec<_>>>()?;
    group_worktrees_under_their_repository(&mut repos);
    Ok(repos)
}

/// Put every linked worktree right after the repository it belongs to, so the
/// tree can show it as a child rather than as a sibling repository that
/// happens to share a name prefix.
fn group_worktrees_under_their_repository(repos: &mut [NestedRepo]) {
    let known = repos
        .iter()
        .map(|repo| repo.path.clone())
        .collect::<std::collections::HashSet<_>>();
    repos.sort_by_cached_key(|repo| {
        let anchor = repo
            .worktree_of
            .as_deref()
            .filter(|main| known.contains(*main))
            .unwrap_or(&repo.path);
        (anchor.to_owned(), anchor != repo.path, repo.path.clone())
    });
}

pub fn checkout_nested_branch(repo_path: &str, branch: &str) -> Result<String> {
    let dir = nested_repo_dir(repo_path)?;
    checkout_branch_in_dir(&dir, branch)
}

pub fn checkout_nested_branch_at(root: &Path, repo_path: &str, branch: &str) -> Result<String> {
    let dir = nested_repo_dir_at(root, repo_path)?;
    checkout_branch_in_dir(&dir, branch)
}

pub fn checkout_nested_remote_branch(repo_path: &str, remote_ref: &str) -> Result<String> {
    let dir = nested_repo_dir(repo_path)?;
    checkout_remote_branch_in_dir(&dir, remote_ref)
}

pub fn checkout_nested_remote_branch_at(
    root: &Path,
    repo_path: &str,
    remote_ref: &str,
) -> Result<String> {
    let dir = nested_repo_dir_at(root, repo_path)?;
    checkout_remote_branch_in_dir(&dir, remote_ref)
}

pub(super) fn nested_repo_dir(repo_path: &str) -> Result<PathBuf> {
    let root = PathBuf::from(repo_root()?);
    nested_repo_dir_at(&root, repo_path)
}

pub(super) fn nested_repo_dir_at(root: &Path, repo_path: &str) -> Result<PathBuf> {
    let rel = Path::new(repo_path);
    if repo_path.trim().is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        anyhow::bail!("invalid nested repository path: {repo_path}");
    }
    let dir = root.join(rel);
    if !dir.join(".git").exists() {
        anyhow::bail!("nested repository not found: {repo_path}");
    }
    Ok(dir)
}

fn collect_nested_repo_dirs(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if ignored_discovery_dir(&path) {
            continue;
        }
        if path != root && path.join(".git").exists() {
            out.push(path);
            continue;
        }
        collect_nested_repo_dirs(root, &path, out);
    }
}

pub(super) fn ignored_discovery_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | "build" | ".gradle" | "node_modules")
    )
}

fn nested_repo_status(root: &Path, dir: &Path) -> Result<NestedRepo> {
    let path = dir
        .strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .into_owned();
    let branch = nested_head_branch(dir)?;
    let detached_at = if branch.is_none() {
        nested_head_short_sha(dir).ok()
    } else {
        None
    };
    let has_changes = nested_repo_has_changes(dir).unwrap_or(false);
    let worktree_of = linked_worktree_main(root, dir);
    Ok(NestedRepo {
        path,
        branch,
        detached_at,
        has_changes,
        worktree_of,
    })
}

/// The workspace-relative path of the main checkout when `dir` is a linked
/// worktree of a repository inside the workspace. A plain repository keeps its
/// git directory under itself; a linked worktree's common git directory lives
/// in the main checkout, which is what gives that checkout away.
fn linked_worktree_main(root: &Path, dir: &Path) -> Option<String> {
    let out = run_in_dir(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let common = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let main = common.parent()?;
    if same_dir(main, dir) {
        return None;
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let main = main.canonicalize().unwrap_or_else(|_| main.to_path_buf());
    let rel = main.strip_prefix(&root).ok()?;
    (!rel.as_os_str().is_empty()).then(|| rel.to_string_lossy().into_owned())
}

fn nested_head_branch(dir: &Path) -> Result<Option<String>> {
    let out = run_in_dir(dir, &["branch", "--show-current"])?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok((!branch.is_empty()).then_some(branch))
}

fn nested_head_short_sha(dir: &Path) -> Result<String> {
    let out = run_in_dir(dir, &["rev-parse", "--short", "HEAD"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn nested_repo_has_changes(dir: &Path) -> Result<bool> {
    let out = run_in_dir(dir, &["status", "--porcelain"])?;
    Ok(!out.stdout.is_empty())
}

fn checkout_branch_in_dir(dir: &Path, branch: &str) -> Result<String> {
    let stashed = if nested_head_branch(dir).is_ok_and(|current| current.as_deref() == Some(branch))
    {
        false
    } else {
        stash_uncommitted_changes_in_dir(dir, "lg: auto-stash before checkout")?
    };
    let out = git_command_in_dir(dir, &["checkout", branch])
        .output()
        .with_context(|| format!("failed to spawn git -C {} checkout {branch}", dir.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    if out.status.success() {
        pop_stash_with_index_if_needed_in_dir(dir, stashed)?;
        Ok(checkout_output_with_stash_notice(combined, stashed))
    } else {
        restore_stash_after_failed_checkout_in_dir(dir, stashed)?;
        Err(anyhow::anyhow!("git checkout failed: {}", combined.trim()))
    }
}

fn checkout_remote_branch_in_dir(dir: &Path, remote_ref: &str) -> Result<String> {
    let stashed = stash_uncommitted_changes_in_dir(dir, "lg: auto-stash before remote checkout")?;
    let out = git_command_in_dir(dir, &["switch", "--track", remote_ref])
        .output()
        .with_context(|| {
            format!(
                "failed to spawn git -C {} switch --track {remote_ref}",
                dir.display()
            )
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    if out.status.success() {
        pop_stash_with_index_if_needed_in_dir(dir, stashed)?;
        Ok(checkout_output_with_stash_notice(combined, stashed))
    } else {
        restore_stash_after_failed_checkout_in_dir(dir, stashed)?;
        Err(anyhow::anyhow!("git switch failed: {}", combined.trim()))
    }
}

fn stash_uncommitted_changes_in_dir(dir: &Path, message: &str) -> Result<bool> {
    let stashed = nested_repo_has_changes(dir)?;
    if stashed {
        run_in_dir(dir, &["stash", "push", "-u", "-m", message])?;
    }
    Ok(stashed)
}

fn pop_stash_with_index_if_needed_in_dir(dir: &Path, stashed: bool) -> Result<()> {
    if stashed {
        run_in_dir(dir, &["stash", "pop", "--index"])?;
    }
    Ok(())
}

fn restore_stash_after_failed_checkout_in_dir(dir: &Path, stashed: bool) -> Result<()> {
    if stashed {
        pop_stash_with_index_if_needed_in_dir(dir, true)
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
