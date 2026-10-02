use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::Receiver;

use crate::config::COMMIT_LIST_LIMIT;
use crate::state::{AppState, RefreshScope, RefreshSnapshot};

use super::watch::WatchRoots;

/// Read what `scope` covers. `changed` are the worktree files whose events
/// asked for a [`RefreshScope::Files`] refresh: when git ignores every one of
/// them, there is nothing to read again and nothing is.
pub(super) fn refresh_snapshot(
    workspace_root: Option<String>,
    scope: RefreshScope,
    changed: &[PathBuf],
) -> RefreshSnapshot {
    if scope == RefreshScope::Files && crate::git::repo_root().is_ok() {
        if !changed.is_empty() && crate::git::all_ignored(changed) {
            return files_snapshot(None, Vec::new());
        }
        let mut errors = Vec::new();
        let files = status_files(&mut errors);
        return files_snapshot(files, errors);
    }
    // A folder that is no checkout has no cheap tier: what changed in it may
    // be a repository arriving.
    let scope = scope.max(RefreshScope::Repository);
    build_refresh_snapshot(workspace_root, scope)
}

fn status_files(errors: &mut Vec<String>) -> Option<Vec<crate::git::FileEntry>> {
    match crate::git::status_entries() {
        Ok(files) => Some(files),
        Err(e) => {
            errors.push(format!("git status failed: {e}"));
            None
        }
    }
}

/// A [`RefreshScope::Files`] snapshot: the file list, or `None` when there
/// was nothing to read again.
fn files_snapshot(
    files: Option<Vec<crate::git::FileEntry>>,
    errors: Vec<String>,
) -> RefreshSnapshot {
    RefreshSnapshot {
        scope: RefreshScope::Files,
        repo_root: None,
        workspace_root: None,
        files,
        branches: None,
        remote_branches: None,
        nested_repositories: None,
        worktrees: None,
        release_branches: Default::default(),
        commits: None,
        unpushed_shas: None,
        branch: None,
        remote_url: None,
        ahead_behind: None,
        commit_author: None,
        decorative_animations: true,
        errors,
    }
}

pub(super) fn build_refresh_snapshot(
    workspace_root: Option<String>,
    scope: RefreshScope,
) -> RefreshSnapshot {
    let scope = scope.max(RefreshScope::Repository);
    let configured_remote = crate::preferences::remote();
    let mut errors = Vec::new();
    let current_root = crate::git::repo_root().ok();
    let workspace_root = workspace_root.or_else(|| current_root.clone());
    if current_root.is_none() {
        return no_repo_snapshot(workspace_root, scope);
    }
    let files = status_files(&mut errors);
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
    let nested_repositories = (scope == RefreshScope::Workspace)
        .then(|| scan_nested_repositories(workspace_root.as_deref(), &mut errors))
        .flatten();
    // The status just read already says whether this checkout has changes.
    let current = current_root
        .as_deref()
        .zip(files.as_ref())
        .map(|(root, files)| (Path::new(root), !files.is_empty()));
    let worktrees = match crate::git::worktrees_knowing(current) {
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
    let (commit_author, decorative_animations) = author_and_animations();
    RefreshSnapshot {
        scope,
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
        remote_url: crate::git::remote_url(configured_remote.as_str()).ok(),
        ahead_behind: crate::git::counts_ahead_behind().ok(),
        commit_author,
        decorative_animations,
        errors,
    }
}

/// Who commits here, and whether the decorative animations run for them.
fn author_and_animations() -> (Option<String>, bool) {
    let author = crate::git::author_config().ok();
    let animations = crate::preferences::animations_enabled_for(author.as_ref());
    let line = author.map(|author| {
        format!(
            "{} <{}>",
            author.name.unwrap_or_default(),
            author.email.unwrap_or_default()
        )
    });
    (line, animations)
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
/// Nothing is asked of git about a branch, a commit or a diff, because every
/// such question would fail and each failure is a red banner on the refresh
/// timer. Two things still happen. The folder's files are listed as new, since
/// with no history behind them that is what they are, so the file pane and the
/// diff beside it work as they would in a fresh checkout. And the scan for
/// repositories inside the folder runs, which is what makes a clone landing in
/// it appear in the workspace pane.
fn no_repo_snapshot(workspace_root: Option<String>, scope: RefreshScope) -> RefreshSnapshot {
    let mut errors = Vec::new();
    // Without a folder there is nothing to look in; the scan would fall back
    // to asking git where it is, which is the question that just failed.
    let (files, nested_repositories) = match workspace_root.as_deref() {
        Some(root) => (
            crate::git::new_file_entries(Path::new(root)),
            (scope == RefreshScope::Workspace)
                .then(|| scan_nested_repositories(Some(root), &mut errors))
                .flatten(),
        ),
        None => (Vec::new(), Some(Vec::new())),
    };
    let (commit_author, decorative_animations) = author_and_animations();
    RefreshSnapshot {
        scope,
        repo_root: None,
        workspace_root,
        files: Some(files),
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
        commit_author,
        decorative_animations,
        errors,
    }
}

/// What the first frame needs before the first refresh lands: which checkout
/// this is and the branch it is on. Two quick git calls; the lists — branches,
/// files, commits — come with the refresh started straight after, rather than
/// being read here and then all over again by it.
pub(super) fn prime_roots(state: &mut AppState) {
    state.repo_root = crate::git::repo_root().ok();
    if state.workspace_root.is_none() {
        state.workspace_root = state.repo_root.clone();
    }
    if state.repo_root.is_some() {
        state.branch = crate::git::head_branch().ok();
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

pub(super) fn git_metadata_path_should_refresh(path: &[&str]) -> bool {
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
) -> Result<(
    RecommendedWatcher,
    Receiver<notify::Result<notify::Event>>,
    WatchRoots,
)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("start file watcher")?;

    watcher
        .watch(dir, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", dir.display()))?;

    let dir_canonical = canonical_path(dir);
    let git_dirs = git_metadata_dirs(dir);
    for git_dir in &git_dirs {
        let git_dir_canonical = canonical_path(git_dir);
        if git_dir_canonical.starts_with(&dir_canonical) {
            continue;
        }
        watcher
            .watch(git_dir, RecursiveMode::Recursive)
            .with_context(|| format!("watch {}", git_dir.display()))?;
    }

    Ok((watcher, rx, WatchRoots::new(dir, git_dirs)))
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

/// The git directory `cwd` uses and the one its repository shares, resolved,
/// from one `rev-parse`. They are the same directory except in a linked
/// worktree.
fn git_metadata_dirs(cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(out) =
        crate::git::git_command_in_dir(cwd, &["rev-parse", "--git-dir", "--git-common-dir"])
            .output()
    else {
        return dirs;
    };
    if !out.status.success() {
        return dirs;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        push_unique_path(&mut dirs, canonical_path(&path));
    }
    dirs
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
    use crate::state::RefreshScope;
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

    /// Nothing to complain about: a folder that is no checkout must not put an
    /// error banner up on every refresh. Its files are still there to work on,
    /// and with no history behind them they are all new.
    #[test]
    fn refreshing_a_plain_directory_lists_its_files_and_reports_no_errors() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("notes.txt"), "mine\n").expect("write file");
        let workspace = tmp.path().to_string_lossy().into_owned();

        let snapshot = crate::git::with_repo(tmp.path(), || {
            build_refresh_snapshot(Some(workspace.clone()), RefreshScope::Workspace)
        });

        assert!(snapshot.errors.is_empty(), "errors: {:?}", snapshot.errors);
        assert_eq!(snapshot.repo_root, None);
        assert_eq!(snapshot.workspace_root, Some(workspace));
        assert_eq!(snapshot.branch, None);
        assert!(snapshot.commits.unwrap_or_default().is_empty());
        let files = snapshot.files.unwrap_or_default();
        assert!(
            files.iter().any(|file| file.path == "notes.txt"),
            "got {files:?}"
        );
    }

    /// The folder was opened to clone into, so the clone has to show up.
    #[test]
    fn refreshing_a_plain_directory_finds_a_repository_cloned_into_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        init_repo_at(&tmp.path().join("cloned"));
        let workspace = tmp.path().to_string_lossy().into_owned();

        let snapshot = crate::git::with_repo(tmp.path(), || {
            build_refresh_snapshot(Some(workspace), RefreshScope::Workspace)
        });

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
