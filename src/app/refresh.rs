use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Component, Path};
use std::sync::mpsc::Receiver;

use crate::config::{COMMIT_LIST_LIMIT, DEFAULT_PUSH_REMOTE};
use crate::state::{AppState, RefreshSnapshot};

pub(super) fn build_refresh_snapshot() -> RefreshSnapshot {
    let mut errors = Vec::new();
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
        repo_root: crate::git::repo_root().ok(),
        files,
        branches,
        remote_branches,
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

fn path_has_ignored_component(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name
            .to_str()
            .is_some_and(|name| matches!(name, ".git" | "target")),
        _ => false,
    })
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
