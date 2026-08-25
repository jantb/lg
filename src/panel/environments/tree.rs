//! The rows the workspace pane shows, and which one is selected.

use crate::{
    git::{Worktree, same_dir},
    state::{AppState, BranchView},
};

/// Whether this worktree is the checkout every other panel is showing.
pub(super) fn worktree_selected(state: &AppState, worktree: &Worktree) -> bool {
    state.repo_root.as_deref().is_some_and(|root| {
        same_dir(
            std::path::Path::new(root),
            std::path::Path::new(&worktree.path),
        )
    })
}

pub(crate) fn nested_repo_tree_len(state: &AppState) -> usize {
    nested_repo_tree_rows(state).len()
}

pub(crate) fn select_nested_repo_tree_row(state: &mut AppState, idx: usize) {
    state.nested_repo_tree_idx = idx;
    match selected_tree_row(state) {
        Some(
            NestedRepoTreeRow::Repo { repo_idx }
            | NestedRepoTreeRow::Branch { repo_idx, .. }
            | NestedRepoTreeRow::Remote { repo_idx, .. },
        ) => state.nested_repositories_idx = repo_idx,
        Some(
            NestedRepoTreeRow::Root
            | NestedRepoTreeRow::Worktree { .. }
            | NestedRepoTreeRow::Session { .. },
        )
        | None => {}
    }
}

pub(super) fn move_selection(state: &mut AppState, down: bool, amount: usize) {
    let len = nested_repo_tree_rows(state).len();
    move_index(&mut state.nested_repo_tree_idx, len, down, amount);
    match selected_tree_row(state) {
        Some(
            NestedRepoTreeRow::Repo { repo_idx }
            | NestedRepoTreeRow::Branch { repo_idx, .. }
            | NestedRepoTreeRow::Remote { repo_idx, .. },
        ) => state.nested_repositories_idx = repo_idx,
        Some(
            NestedRepoTreeRow::Root
            | NestedRepoTreeRow::Worktree { .. }
            | NestedRepoTreeRow::Session { .. },
        )
        | None => {}
    }
}

fn move_index(idx: &mut usize, len: usize, down: bool, amount: usize) {
    if len == 0 {
        *idx = 0;
    } else if down {
        *idx = idx.saturating_add(amount).min(len - 1);
    } else {
        *idx = idx.saturating_sub(amount);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NestedRepoTreeRow {
    Root,
    /// A running (or finished) session, listed under the checkout it runs in.
    Session {
        id: crate::session::SessionId,
    },
    /// A checkout of the active repository, listed under the row that stands
    /// for that repository.
    Worktree {
        wt_idx: usize,
    },
    Repo {
        repo_idx: usize,
    },
    Branch {
        repo_idx: usize,
        branch_idx: usize,
    },
    Remote {
        repo_idx: usize,
        branch_idx: usize,
    },
}

pub(super) fn nested_repo_tree_rows(state: &AppState) -> Vec<NestedRepoTreeRow> {
    let worktrees = worktree_rows(state);
    let anchor = worktree_anchor(state);
    let worktree_rows_for = |repo_idx: Option<usize>| {
        let mine = anchor == repo_idx;
        worktrees
            .iter()
            .filter(move |_| mine)
            .map(|wt_idx| NestedRepoTreeRow::Worktree { wt_idx: *wt_idx })
    };

    let mut rows = Vec::new();
    push_checkout(&mut rows, state, NestedRepoTreeRow::Root);
    for row in worktree_rows_for(None) {
        push_checkout(&mut rows, state, row);
    }
    for (repo_idx, repo) in state.nested_repositories.iter().enumerate() {
        push_checkout(&mut rows, state, NestedRepoTreeRow::Repo { repo_idx });
        for row in worktree_rows_for(Some(repo_idx)) {
            push_checkout(&mut rows, state, row);
        }
        if state.nested_repo_detail_path.as_deref() == Some(repo.path.as_str()) {
            match state.nested_repo_branch_view {
                BranchView::Local => {
                    rows.extend(state.nested_repo_branches.iter().enumerate().map(
                        |(branch_idx, _)| NestedRepoTreeRow::Branch {
                            repo_idx,
                            branch_idx,
                        },
                    ))
                }
                BranchView::Remote => {
                    rows.extend(state.visible_nested_repo_remote_branches().enumerate().map(
                        |(branch_idx, _)| NestedRepoTreeRow::Remote {
                            repo_idx,
                            branch_idx,
                        },
                    ))
                }
            }
        }
    }
    rows
}

/// Add a checkout row, followed by the session running in it, if any.
fn push_checkout(rows: &mut Vec<NestedRepoTreeRow>, state: &AppState, row: NestedRepoTreeRow) {
    rows.push(row);
    if let Some(dir) = row_checkout_dir(state, row)
        && let Some(id) = state.sessions.for_dir(std::path::Path::new(&dir))
    {
        rows.push(NestedRepoTreeRow::Session { id });
    }
}

/// Where a row's checkout lives, for rows that stand for one.
pub(super) fn row_checkout_dir(state: &AppState, row: NestedRepoTreeRow) -> Option<String> {
    match row {
        NestedRepoTreeRow::Root => state
            .workspace_root
            .clone()
            .or_else(|| state.repo_root.clone()),
        NestedRepoTreeRow::Worktree { wt_idx } => state
            .worktrees
            .get(wt_idx)
            .map(|worktree| worktree.path.clone()),
        NestedRepoTreeRow::Repo { repo_idx } => {
            let root = state.workspace_root.as_ref().or(state.repo_root.as_ref())?;
            let repo = state.nested_repositories.get(repo_idx)?;
            Some(
                std::path::Path::new(root)
                    .join(&repo.path)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
        NestedRepoTreeRow::Session { .. }
        | NestedRepoTreeRow::Branch { .. }
        | NestedRepoTreeRow::Remote { .. } => None,
    }
}

/// Worktrees that need a row of their own. The tree already shows the
/// workspace checkout and every nested repository, so a worktree sitting at one
/// of those paths is left out rather than listed twice.
fn worktree_rows(state: &AppState) -> Vec<usize> {
    if state.worktrees.len() < 2 {
        return Vec::new();
    }
    state
        .worktrees
        .iter()
        .enumerate()
        .filter(|(_, worktree)| !worktree_has_another_row(state, worktree))
        .map(|(idx, _)| idx)
        .collect()
}

fn worktree_has_another_row(state: &AppState, worktree: &Worktree) -> bool {
    let path = std::path::Path::new(&worktree.path);
    let Some(root) = state.workspace_root.as_deref() else {
        return false;
    };
    let root = std::path::Path::new(root);
    if same_dir(root, path) {
        return true;
    }
    state
        .nested_repositories
        .iter()
        .any(|repo| same_dir(&root.join(&repo.path), path))
}

/// The row the worktrees hang under: the nested repository they belong to, or
/// `None` for the workspace checkout itself. The main worktree is what ties a
/// set of worktrees to a repository.
fn worktree_anchor(state: &AppState) -> Option<usize> {
    let main = state.worktrees.iter().find(|worktree| worktree.is_main)?;
    let root = std::path::Path::new(state.workspace_root.as_deref()?);
    state
        .nested_repositories
        .iter()
        .position(|repo| same_dir(&root.join(&repo.path), std::path::Path::new(&main.path)))
}

pub(super) fn selected_tree_row(state: &AppState) -> Option<NestedRepoTreeRow> {
    nested_repo_tree_rows(state)
        .get(state.nested_repo_tree_idx)
        .copied()
}

pub(super) fn tree_idx_for_repo_path(state: &AppState, path: &str) -> Option<usize> {
    nested_repo_tree_rows(state)
        .iter()
        .position(|row| matches!(row, NestedRepoTreeRow::Repo { repo_idx } if state.nested_repositories.get(*repo_idx).is_some_and(|repo| repo.path == path)))
}

/// Whether the checkout lg is pointed at is the workspace root itself, rather
/// than one of the repositories inside it. True when there is no workspace to
/// distinguish it from.
pub(super) fn root_repo_selected(state: &AppState) -> bool {
    match (state.workspace_root.as_deref(), state.repo_root.as_deref()) {
        (Some(workspace), Some(repo)) => {
            same_dir(std::path::Path::new(workspace), std::path::Path::new(repo))
        }
        _ => true,
    }
}

pub(super) fn nested_repo_selected(state: &AppState, repo_path: &str) -> bool {
    let (Some(workspace), Some(repo_root)) =
        (state.workspace_root.as_deref(), state.repo_root.as_deref())
    else {
        return false;
    };
    // Resolved through symlinks: a workspace of symlinked repositories reports a
    // repo_root outside the workspace, which no prefix of it matches.
    same_dir(
        &std::path::Path::new(workspace).join(repo_path),
        std::path::Path::new(repo_root),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A workspace directory holding a symlink to a repository that really
    /// lives elsewhere, which is how these workspaces are normally laid out.
    fn workspace_with_symlinked_repo() -> (tempfile::TempDir, String, String) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workspace = tmp.path().join("workspace");
        let real = tmp.path().join("elsewhere").join("lg");
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::create_dir_all(&real).expect("repo");
        std::os::unix::fs::symlink(&real, workspace.join("lg")).expect("symlink");
        (
            tmp,
            workspace.to_string_lossy().into_owned(),
            real.to_string_lossy().into_owned(),
        )
    }

    #[test]
    fn a_symlinked_repository_counts_as_the_active_one() {
        let (_tmp, workspace, real) = workspace_with_symlinked_repo();
        let mut state = AppState::new();
        state.workspace_root = Some(workspace);
        // git reports the resolved path, which sits outside the workspace.
        state.repo_root = Some(real);

        assert!(
            nested_repo_selected(&state, "lg"),
            "the row for the checked-out repository must show as active"
        );
        assert!(
            !root_repo_selected(&state),
            "the workspace root is not what is checked out"
        );
    }

    #[test]
    fn another_repository_in_the_same_workspace_is_not_active() {
        let (_tmp, workspace, real) = workspace_with_symlinked_repo();
        let mut state = AppState::new();
        state.workspace_root = Some(workspace);
        state.repo_root = Some(real);

        assert!(!nested_repo_selected(&state, "melt"));
    }

    #[test]
    fn the_workspace_root_itself_is_reported_as_the_root_row() {
        let (_tmp, workspace, _real) = workspace_with_symlinked_repo();
        let mut state = AppState::new();
        state.workspace_root = Some(workspace.clone());
        state.repo_root = Some(workspace);

        assert!(root_repo_selected(&state));
    }
}
