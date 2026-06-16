use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;

use crate::config::{COMMIT_LIST_LIMIT, DEFAULT_PUSH_REMOTE};
use crate::state::{AppState, RefreshSnapshot};

pub(super) fn build_refresh_snapshot(workspace_root: Option<String>) -> RefreshSnapshot {
    let mut errors = Vec::new();
    let current_root = crate::git::repo_root().ok();
    let workspace_root = workspace_root.or_else(|| current_root.clone());
    let files = match crate::git::status_entries() {
        Ok(files) => Some(files),
        Err(e) => {
            errors.push(format!("git status failed: {e}"));
            None
        }
    };
    let branches = match crate::git::list_branches() {
        Ok(branches) => Some(branches),
        Err(e) => {
            errors.push(format!("git branch failed: {e}"));
            None
        }
    };
    let remote_branches = match crate::git::list_remote_branches() {
        Ok(branches) => Some(branches),
        Err(e) => {
            errors.push(format!("git remote branch failed: {e}"));
            None
        }
    };
    let nested_repositories = match workspace_root
        .as_deref()
        .map(PathBuf::from)
        .map(|root| crate::git::nested_repositories_at(&root))
        .unwrap_or_else(crate::git::nested_repositories)
    {
        Ok(repositories) => Some(repositories),
        Err(e) => {
            errors.push(format!("nested repository scan failed: {e}"));
            None
        }
    };
    let unpushed_shas = match crate::git::unpushed_shas() {
        Ok(shas) => Some(shas),
        Err(e) => {
            errors.push(format!("unpushed check failed: {e}"));
            None
        }
    };
    let branch = crate::git::head_branch().ok().or_else(|| {
        branches.as_ref().and_then(|branches| {
            branches
                .iter()
                .find(|branch| branch.is_current)
                .map(|branch| branch.name.clone())
        })
    });
    let commits = match crate::git::list_commits(COMMIT_LIST_LIMIT) {
        Ok(commits) => Some(commits),
        Err(e) => {
            errors.push(format!("git log failed: {e}"));
            None
        }
    };
    RefreshSnapshot {
        repo_root: current_root,
        workspace_root,
        files,
        branches,
        remote_branches,
        nested_repositories,
        flow_branches_available: crate::git::flow_branches_available(),
        commits,
        unpushed_shas,
        branch,
        remote_url: crate::git::remote_url(DEFAULT_PUSH_REMOTE).ok(),
        ahead_behind: crate::git::counts_ahead_behind().ok(),
        errors,
    }
}

pub(super) fn prime_branches(state: &mut AppState) {
    state.repo_root = crate::git::repo_root().ok();
    if state.workspace_root.is_none() {
        state.workspace_root = state.repo_root.clone();
    }
    if let Ok(branches) = crate::git::list_branches() {
        state.branch = branches
            .iter()
            .find(|branch| branch.is_current)
            .map(|branch| branch.name.clone());
        state.branches = branches;
        state.clamp();
    }
    if let Ok(branches) = crate::git::list_remote_branches() {
        state.remote_branches = branches;
        state.clamp();
    }
}

pub(super) fn prime_files(state: &mut AppState) {
    if let Ok(files) = crate::git::status_entries() {
        state.files = files;
        state.clamp();
    }
}

fn path_should_refresh(path: &Path) -> bool {
    let mut git_relative = Vec::new();
    let mut in_git_dir = false;

    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            continue;
        };
        if in_git_dir {
            git_relative.push(name);
        } else if name == ".git" {
            in_git_dir = true;
        } else if name == "target" {
            return false;
        }
    }

    !in_git_dir || git_metadata_path_should_refresh(&git_relative)
}

fn git_metadata_path_should_refresh(path: &[&str]) -> bool {
    let Some(first) = path.first().copied() else {
        return true;
    };

    matches!(
        first,
        "HEAD"
            | "ORIG_HEAD"
            | "FETCH_HEAD"
            | "MERGE_HEAD"
            | "MERGE_MODE"
            | "MERGE_MSG"
            | "REBASE_HEAD"
            | "CHERRY_PICK_HEAD"
            | "REVERT_HEAD"
            | "SQUASH_MSG"
            | "AUTO_MERGE"
            | "index"
            | "packed-refs"
            | "shallow"
            | "config"
            | "refs"
            | "logs"
            | "worktrees"
            | "modules"
            | "rebase-apply"
            | "rebase-merge"
            | "sequencer"
    ) || first.starts_with("sharedindex.")
        || matches!(path, ["info", "exclude"])
}

pub(super) fn should_refresh_for_fs_event(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    if event.paths.is_empty() {
        return true;
    }
    event.paths.iter().any(|path| path_should_refresh(path))
}

pub(super) fn watch_current_dir()
-> Result<(RecommendedWatcher, Receiver<notify::Result<notify::Event>>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("start file watcher")?;

    let cwd = std::env::current_dir().context("resolve current directory")?;
    let repo_root = crate::git::repo_root()
        .map(PathBuf::from)
        .unwrap_or_else(|_| cwd.clone());
    watcher
        .watch(&repo_root, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", repo_root.display()))?;

    let repo_root_canonical = canonical_path(&repo_root);
    for git_dir in git_metadata_dirs(&cwd) {
        let git_dir_canonical = canonical_path(&git_dir);
        if git_dir_canonical.starts_with(&repo_root_canonical) {
            continue;
        }
        watcher
            .watch(&git_dir, RecursiveMode::Recursive)
            .with_context(|| format!("watch {}", git_dir.display()))?;
    }

    Ok((watcher, rx))
}

fn git_metadata_dirs(cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for arg in ["--git-dir", "--git-common-dir"] {
        let Some(path) = git_rev_parse_path(cwd, arg) else {
            continue;
        };
        push_unique_path(&mut dirs, canonical_path(&path));
    }
    dirs
}

fn git_rev_parse_path(cwd: &Path, arg: &str) -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", arg])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let path = text.trim();
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    Some(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::should_refresh_for_fs_event;
    use notify::{
        Event, EventKind,
        event::{AccessKind, AccessMode, ModifyKind},
    };
    use std::path::PathBuf;

    fn modify_event(path: &str) -> Event {
        Event::new(EventKind::Modify(ModifyKind::Any)).add_path(PathBuf::from(path))
    }

    #[test]
    fn refreshes_for_git_refs_written_by_external_commits_and_pushes() {
        for path in [
            ".git/HEAD",
            ".git/refs/heads/main",
            ".git/refs/heads/target",
            ".git/refs/remotes/origin/main",
            ".git/logs/refs/heads/main",
            ".git/packed-refs",
            ".git/index",
            ".git/worktrees/feature/HEAD",
            ".git/modules/lib/refs/heads/main",
        ] {
            assert!(
                should_refresh_for_fs_event(&modify_event(path)),
                "{path} should trigger refresh"
            );
        }
    }

    #[test]
    fn ignores_noisy_git_internals_and_build_output() {
        for path in [
            ".git/objects/ab/cdef",
            ".git/gc.log",
            ".git/hooks/pre-commit",
            "target/debug/lg",
        ] {
            assert!(
                !should_refresh_for_fs_event(&modify_event(path)),
                "{path} should not trigger refresh"
            );
        }
    }

    #[test]
    fn refreshes_for_worktree_changes_and_unknown_path_events() {
        assert!(should_refresh_for_fs_event(&modify_event("src/main.rs")));
        assert!(should_refresh_for_fs_event(&Event::new(EventKind::Modify(
            ModifyKind::Any
        ))));
    }

    #[test]
    fn ignores_access_events() {
        let event = Event::new(EventKind::Access(AccessKind::Close(AccessMode::Read)))
            .add_path(PathBuf::from(".git/refs/heads/main"));

        assert!(!should_refresh_for_fs_event(&event));
    }
}
