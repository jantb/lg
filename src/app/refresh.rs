use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Component, Path, PathBuf};
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

fn path_has_ignored_component(path: &Path) -> bool {
    if is_git_head_path(path) {
        return false;
    }
    path.components().any(|component| match component {
        Component::Normal(name) => name
            .to_str()
            .is_some_and(|name| matches!(name, ".git" | "target")),
        _ => false,
    })
}

fn is_git_head_path(path: &Path) -> bool {
    let mut components = path.components().rev();
    matches!(
        (components.next(), components.next()),
        (
            Some(Component::Normal(file)),
            Some(Component::Normal(dir))
        ) if file.to_str() == Some("HEAD") && dir.to_str() == Some(".git")
    )
}

pub(super) fn should_refresh_for_fs_event(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    if event.paths.is_empty() {
        return true;
    }
    event
        .paths
        .iter()
        .any(|path| !path_has_ignored_component(path))
}

pub(super) fn watch_current_dir()
-> Result<(RecommendedWatcher, Receiver<notify::Result<notify::Event>>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("start file watcher")?;

    let cwd = std::env::current_dir().context("resolve current directory")?;
    watcher
        .watch(&cwd, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", cwd.display()))?;

    Ok((watcher, rx))
}
