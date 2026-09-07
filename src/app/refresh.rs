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
    if current_root.is_none() {
        return no_repo_snapshot(workspace_root);
    }
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
    let nested_repositories = scan_nested_repositories(workspace_root.as_deref(), &mut errors);
    let worktrees = match crate::git::worktrees() {
        Ok(worktrees) => Some(worktrees),
        Err(e) => {
            errors.push(format!("worktree list failed: {e}"));
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
        worktrees,
        release_branches: crate::git::release_branches(),
        commits,
        unpushed_shas,
        branch,
        remote_url: crate::git::remote_url(DEFAULT_PUSH_REMOTE).ok(),
        ahead_behind: crate::git::counts_ahead_behind().ok(),
        errors,
    }
}

fn scan_nested_repositories(
    workspace_root: Option<&str>,
    errors: &mut Vec<String>,
) -> Option<Vec<crate::git::NestedRepo>> {
    match workspace_root
        .map(PathBuf::from)
        .map(|root| crate::git::nested_repositories_at(&root))
        .unwrap_or_else(crate::git::nested_repositories)
    {
        Ok(repositories) => Some(repositories),
        Err(e) => {
            errors.push(format!("nested repository scan failed: {e}"));
            None
        }
    }
}

/// What a refresh reports for a directory that is no checkout — a plain folder
/// lg was started in, with nothing cloned into it yet.
///
/// Git is not asked anything, because every question would fail and each
/// failure is a red banner on the refresh timer. The panels are emptied rather
/// than left holding a repository that is no longer there, and the scan for
/// repositories inside the folder still runs: that is what makes a clone
/// landing in it appear in the workspace pane.
fn no_repo_snapshot(workspace_root: Option<String>) -> RefreshSnapshot {
    let mut errors = Vec::new();
    // Without a folder to scan there is nothing to find; the scan would fall
    // back to asking git where it is, which is the question that just failed.
    let nested_repositories = match workspace_root.as_deref() {
        Some(root) => scan_nested_repositories(Some(root), &mut errors),
        None => Some(Vec::new()),
    };
    RefreshSnapshot {
        repo_root: None,
        workspace_root,
        files: Some(Vec::new()),
        branches: Some(Vec::new()),
        remote_branches: Some(Vec::new()),
        nested_repositories,
        worktrees: Some(Vec::new()),
        release_branches: Default::default(),
        commits: Some(Vec::new()),
        unpushed_shas: Some(Default::default()),
        branch: None,
        remote_url: None,
        ahead_behind: None,
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

/// Watch `dir` and the git metadata it uses. A linked worktree keeps its git
/// directory inside the main repository, which `--git-common-dir` reports, so
/// worktrees get the same ref and index notifications as a plain checkout.
pub(super) fn watch_repo(
    dir: &Path,
) -> Result<(RecommendedWatcher, Receiver<notify::Result<notify::Event>>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("start file watcher")?;

    watcher
        .watch(dir, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", dir.display()))?;

    let dir_canonical = canonical_path(dir);
    for git_dir in git_metadata_dirs(dir) {
        let git_dir_canonical = canonical_path(&git_dir);
        if git_dir_canonical.starts_with(&dir_canonical) {
            continue;
        }
        watcher
            .watch(&git_dir, RecursiveMode::Recursive)
            .with_context(|| format!("watch {}", git_dir.display()))?;
    }

    Ok((watcher, rx))
}

/// Where lg starts: the directory to run git in, and the workspace it sits in
/// when that is a different directory.
///
/// Started inside a repository, that repository is the checkout and there is
/// no separate workspace. Started in a directory that only holds repositories
/// — a folder of projects — the directory is the workspace and the first
/// repository in it is the checkout, so the tree of repositories is there to
/// pick another from. Started in a plain directory, that directory is the
/// workspace and there is no checkout at all: an empty folder is where a clone
/// or a `git init` is about to happen, and lg opens on it so a session can be
/// started there to do exactly that.
pub(super) fn startup_roots() -> Result<StartupRoots> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    roots_for_dir(&cwd)
}

fn roots_for_dir(cwd: &Path) -> Result<StartupRoots> {
    if let Some(root) = crate::git::repo_root_at(cwd) {
        return Ok(StartupRoots {
            workspace: None,
            start_dir: PathBuf::from(root),
        });
    }
    let nested = crate::git::nested_repositories_at(cwd)
        .with_context(|| format!("scan {} for repositories", cwd.display()))?;
    let start_dir = match nested.first() {
        Some(first) => cwd.join(&first.path),
        None => cwd.to_path_buf(),
    };
    Ok(StartupRoots {
        start_dir,
        workspace: Some(cwd.to_path_buf()),
    })
}

pub(super) struct StartupRoots {
    /// The directory holding the repositories, when lg was started in one.
    pub workspace: Option<PathBuf>,
    /// The directory git commands run in and the watcher watches: the checkout
    /// lg opens on, or the workspace itself when it holds no repository yet.
    pub start_dir: PathBuf,
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
    use super::{build_refresh_snapshot, roots_for_dir, should_refresh_for_fs_event};
    use notify::{
        Event, EventKind,
        event::{AccessKind, AccessMode, ModifyKind},
    };
    use std::path::{Path, PathBuf};

    fn modify_event(path: &str) -> Event {
        Event::new(EventKind::Modify(ModifyKind::Any)).add_path(PathBuf::from(path))
    }

    fn init_repo_at(dir: &Path) {
        std::fs::create_dir_all(dir).expect("create repo dir");
        let out = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .output()
            .expect("run git init");
        assert!(out.status.success(), "git init failed");
    }

    /// lg is often opened on the folder a project is about to be cloned into.
    #[test]
    fn a_plain_directory_opens_as_a_workspace_with_no_checkout() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let roots = roots_for_dir(tmp.path()).expect("a plain directory must open");

        assert_eq!(roots.workspace.as_deref(), Some(tmp.path()));
        assert_eq!(roots.start_dir, tmp.path());
    }

    #[test]
    fn a_folder_of_projects_opens_on_a_repository_inside_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        init_repo_at(&tmp.path().join("project"));

        let roots = roots_for_dir(tmp.path()).expect("a workspace must open");

        assert_eq!(roots.workspace.as_deref(), Some(tmp.path()));
        assert_eq!(roots.start_dir, tmp.path().join("project"));
    }

    /// Nothing to report and nothing to complain about: a folder that is no
    /// checkout must not produce an error banner on every refresh.
    #[test]
    fn refreshing_a_plain_directory_reports_no_repository_and_no_errors() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workspace = tmp.path().to_string_lossy().into_owned();

        let snapshot =
            crate::git::with_repo(tmp.path(), || build_refresh_snapshot(Some(workspace.clone())));

        assert!(snapshot.errors.is_empty(), "errors: {:?}", snapshot.errors);
        assert_eq!(snapshot.repo_root, None);
        assert_eq!(snapshot.workspace_root, Some(workspace));
        assert_eq!(snapshot.branch, None);
        assert!(snapshot.files.unwrap_or_default().is_empty());
        assert!(snapshot.commits.unwrap_or_default().is_empty());
    }

    /// The folder was opened to clone into, so the clone has to show up.
    #[test]
    fn refreshing_a_plain_directory_finds_a_repository_cloned_into_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        init_repo_at(&tmp.path().join("cloned"));
        let workspace = tmp.path().to_string_lossy().into_owned();

        let snapshot = crate::git::with_repo(tmp.path(), || build_refresh_snapshot(Some(workspace)));

        assert!(snapshot.errors.is_empty(), "errors: {:?}", snapshot.errors);
        assert!(
            snapshot
                .nested_repositories
                .unwrap_or_default()
                .iter()
                .any(|repo| repo.path == "cloned")
        );
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
