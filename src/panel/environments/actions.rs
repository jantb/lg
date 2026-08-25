//! What the workspace keys do to the selected row.

use crate::{
    git::Worktree,
    state::{AppState, BranchView},
};

use super::tree::{
    NestedRepoTreeRow, selected_tree_row, tree_idx_for_repo_path, worktree_selected,
};

pub(crate) fn activate_selected_repository_row(state: &mut AppState) -> bool {
    match selected_tree_row(state) {
        Some(NestedRepoTreeRow::Worktree { wt_idx }) => activate_worktree_row(state, wt_idx),
        Some(NestedRepoTreeRow::Root) => {
            state.pending_action = Some(crate::state::PendingAction::SwitchRepository {
                target: crate::state::RepoTarget::Workspace,
            });
            true
        }
        Some(NestedRepoTreeRow::Repo { repo_idx }) => {
            let Some(path) = state
                .nested_repositories
                .get(repo_idx)
                .map(|repo| repo.path.clone())
            else {
                return false;
            };
            state.pending_action = Some(crate::state::PendingAction::SwitchRepository {
                target: crate::state::RepoTarget::Nested(path.clone()),
            });
            if state.nested_repo_detail_path.as_deref() == Some(path.as_str()) {
                state.nested_repo_tree_idx = tree_idx_for_repo_path(state, &path).unwrap_or(0);
            } else {
                open_nested_repo_detail(state, path);
            }
            true
        }
        _ => false,
    }
}

/// Start a claude session in the selected checkout, or show the one already
/// running there. One session per checkout, so this is also how you get back to
/// a session you left.
pub(super) fn start_session_for_selection(state: &mut AppState, sandboxed: bool) {
    let Some((path, label)) = selected_checkout(state) else {
        state.set_status("select a repository or worktree first", false);
        return;
    };
    if let Some(id) = state.sessions.for_dir(std::path::Path::new(&path)) {
        state.show_session(id);
        state.session_capture = true;
        return;
    }
    state.pending_action = Some(crate::state::PendingAction::StartSession {
        path,
        label,
        sandboxed,
    });
}

/// The checkout a row stands for: where it is, and what to call it.
fn selected_checkout(state: &AppState) -> Option<(String, String)> {
    match selected_tree_row(state)? {
        NestedRepoTreeRow::Session { id } => {
            let session = state.sessions.get(id)?;
            Some((
                session.cwd.to_string_lossy().into_owned(),
                session.label.clone(),
            ))
        }
        NestedRepoTreeRow::Worktree { wt_idx } => {
            let worktree = state.worktrees.get(wt_idx)?;
            Some((worktree.path.clone(), worktree.label()))
        }
        NestedRepoTreeRow::Root => {
            let path = state
                .workspace_root
                .clone()
                .or_else(|| state.repo_root.clone())?;
            let label = state
                .branch
                .clone()
                .unwrap_or_else(|| dir_name(&path).to_string());
            Some((path, label))
        }
        NestedRepoTreeRow::Repo { repo_idx }
        | NestedRepoTreeRow::Branch { repo_idx, .. }
        | NestedRepoTreeRow::Remote { repo_idx, .. } => {
            let root = state.workspace_root.as_ref().or(state.repo_root.as_ref())?;
            let repo = state.nested_repositories.get(repo_idx)?;
            let path = std::path::Path::new(root)
                .join(&repo.path)
                .to_string_lossy()
                .into_owned();
            let label = repo
                .branch
                .clone()
                .unwrap_or_else(|| dir_name(&repo.path).to_string());
            Some((path, label))
        }
    }
}

fn dir_name(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

/// Open the new-worktree form for the active repository.
pub(super) fn open_new_worktree_form(state: &mut AppState) {
    if state.repo_root.is_none() && state.worktrees.is_empty() {
        state.set_status("no repository to add a worktree to", true);
        return;
    }
    state.open_worktree_modal(crate::git::preferred_base_ref());
}

/// Remove the selected worktree, or forget one whose directory is already
/// gone. The checkout lg is showing cannot be removed from under itself, and
/// neither can the main worktree — git refuses that too.
pub(super) fn remove_selected_worktree(state: &mut AppState) {
    let Some(NestedRepoTreeRow::Worktree { wt_idx }) = selected_tree_row(state) else {
        state.set_status("select a worktree row to remove it", false);
        return;
    };
    let Some(worktree) = state.worktrees.get(wt_idx) else {
        return;
    };
    if worktree.is_missing() {
        state.pending_action = Some(crate::state::PendingAction::PruneWorktrees);
        return;
    }
    if worktree.is_main {
        state.set_status("the main checkout cannot be removed", true);
        return;
    }
    if worktree_selected(state, worktree) {
        state.set_status("switch to another checkout first", true);
        return;
    }
    if worktree.locked.is_some() {
        state.set_status(format!("{} is locked", worktree.label()), true);
        return;
    }

    let label = worktree.label();
    let path = worktree.path.clone();
    let dirty = worktree.has_changes;
    if let Some(reason) = session_in_the_way(state, &path, &label) {
        state.set_status(reason, true);
        return;
    }
    let detail = if dirty {
        format!("{path} has uncommitted changes, which will be discarded.")
    } else {
        path.clone()
    };
    state.confirm_action(
        "Remove Worktree",
        format!("Remove the worktree for {label}?"),
        detail,
        crate::state::PendingAction::RemoveWorktree { path, force: dirty },
    );
}

/// The session the repository tree has selected, if the selection is one.
pub(crate) fn selected_session(state: &AppState) -> Option<crate::session::SessionId> {
    match selected_tree_row(state)? {
        NestedRepoTreeRow::Session { id } => Some(id),
        _ => None,
    }
}

/// Stop the selected session and forget it. A session that ends on its own is
/// dropped as soon as that is noticed, so this is for the ones still running:
/// closing one from the row it is shown on, rather than only from inside the
/// session pane.
pub(super) fn close_selected_session(state: &mut AppState) {
    let Some(NestedRepoTreeRow::Session { id }) = selected_tree_row(state) else {
        state.set_status("select a session row to close it", false);
        return;
    };
    let label = state
        .sessions
        .get(id)
        .map(|session| session.label.clone())
        .unwrap_or_default();
    state.sessions.close(id);
    if state.session_view().is_none() {
        state.show_diff();
    }
    state.clamp();
    state.set_status(format!("closed the session in {label}"), false);
}

/// Merge the selected worktree's branch into main and clean up after it. The
/// merge itself runs in whichever checkout holds main, so it works from here
/// no matter which checkout lg is currently showing.
pub(super) fn land_selected_worktree(state: &mut AppState) {
    let Some((path, branch)) = handover_candidate(state, "land") else {
        return;
    };
    let remote = format!("{}/{branch}", crate::config::DEFAULT_PUSH_REMOTE);
    state.confirm_action(
        "Land Worktree",
        format!("Merge {branch} into main and clean up?"),
        [
            format!("merge {branch} into main"),
            "push main".to_string(),
            format!("remove {path}"),
            format!("delete {branch} and {remote}"),
        ]
        .join("\n"),
        crate::state::PendingAction::LandWorktree { path, branch },
    );
}

/// Merge main into the selected worktree's branch, in the worktree. This is
/// what unblocks a land that main has moved out from under: the merge conflict
/// belongs to the branch, not to main.
pub(super) fn sync_selected_worktree(state: &mut AppState) {
    let Some((path, branch)) = handover_candidate(state, "sync") else {
        return;
    };
    state.confirm_reversible_action(
        "Sync Worktree",
        format!("Merge main into {branch}?"),
        [
            format!("merge main into {branch} in {path}"),
            "main is not touched".to_string(),
            "conflicts are resolved here, then landing is a fast-forward".to_string(),
        ]
        .join("\n"),
        crate::state::PendingAction::SyncWorktree { path, branch },
    );
}

/// Move the selected worktree's branch back to the main checkout, so the work
/// carries on there. Nothing is merged and the branch stays as it is.
pub(super) fn bring_selected_worktree_home(state: &mut AppState) {
    let Some((path, branch)) = handover_candidate(state, "bring home") else {
        return;
    };
    state.confirm_reversible_action(
        "Bring Branch Home",
        format!("Move {branch} to the main checkout?"),
        [
            format!("remove {path}"),
            format!("check {branch} out in the main checkout"),
            "nothing is merged; the branch keeps living".to_string(),
        ]
        .join("\n"),
        crate::state::PendingAction::BringWorktreeHome { path, branch },
    );
}

/// The linked worktree the repository tree has selected, if the selection is
/// one at all. The footer offers the handover keys only where they apply.
pub(crate) fn selected_linked_worktree(state: &AppState) -> Option<&Worktree> {
    let NestedRepoTreeRow::Worktree { wt_idx } = selected_tree_row(state)? else {
        return None;
    };
    state
        .worktrees
        .get(wt_idx)
        .filter(|worktree| !worktree.is_main)
}

/// The worktree and branch a handover would move, once the reasons it cannot
/// happen are out of the way. `verb` names the action in those refusals.
fn handover_candidate(state: &mut AppState, verb: &str) -> Option<(String, String)> {
    let Some(worktree) = selected_linked_worktree(state) else {
        state.set_status(format!("select a worktree row to {verb} it"), false);
        return None;
    };
    let label = worktree.label();
    let path = worktree.path.clone();
    let branch = worktree.branch.clone();
    let (missing, locked, dirty) = (
        worktree.is_missing(),
        worktree.locked.is_some(),
        worktree.has_changes,
    );

    if missing {
        state.set_status(format!("{label} is missing on disk; prune it"), true);
        return None;
    }
    if locked {
        state.set_status(format!("{label} is locked"), true);
        return None;
    }
    if dirty {
        state.set_status(
            format!("commit or discard the changes in {label} first"),
            true,
        );
        return None;
    }
    if let Some(reason) = session_in_the_way(state, &path, &label) {
        state.set_status(reason, true);
        return None;
    }
    let Some(branch) = branch else {
        state.set_status("a detached worktree has no branch to hand over", true);
        return None;
    };
    Some((path, branch))
}

/// A live claude session keeps its checkout in use: removing the directory
/// would pull it out from under a running process. The refusal names the keys,
/// because a session holding the keyboard is exactly when they are hardest to
/// go looking for.
fn session_in_the_way(state: &AppState, path: &str, label: &str) -> Option<String> {
    state
        .sessions
        .for_dir(std::path::Path::new(path))
        .map(|_| format!("close the claude session in {label} first \u{2014} Ctrl-] then x"))
}

/// Switch to the selected worktree. A worktree git still knows about but whose
/// directory is gone cannot be entered, so say that instead of failing later.
pub(super) fn activate_worktree_row(state: &mut AppState, wt_idx: usize) -> bool {
    let Some(worktree) = state.worktrees.get(wt_idx) else {
        return false;
    };
    if worktree.is_missing() {
        let label = worktree.label();
        state.set_status(format!("{label} is missing on disk; prune it"), true);
        return false;
    }
    state.pending_action = Some(crate::state::PendingAction::SwitchRepository {
        target: crate::state::RepoTarget::Path(std::path::PathBuf::from(&worktree.path)),
    });
    true
}

pub(super) fn selected_repository_project_path(state: &AppState) -> Option<String> {
    let root = state.workspace_root.as_ref().or(state.repo_root.as_ref())?;
    match selected_tree_row(state)? {
        NestedRepoTreeRow::Root => Some(root.clone()),
        NestedRepoTreeRow::Session { id } => state
            .sessions
            .get(id)
            .map(|session| session.cwd.to_string_lossy().into_owned()),
        NestedRepoTreeRow::Worktree { wt_idx } => state
            .worktrees
            .get(wt_idx)
            .map(|worktree| worktree.path.clone()),
        NestedRepoTreeRow::Repo { repo_idx }
        | NestedRepoTreeRow::Branch { repo_idx, .. }
        | NestedRepoTreeRow::Remote { repo_idx, .. } => state
            .nested_repositories
            .get(repo_idx)
            .map(|repo| std::path::Path::new(root).join(&repo.path))
            .map(|path| path.to_string_lossy().into_owned()),
    }
}

pub(crate) fn open_nested_repo_detail(state: &mut AppState, path: String) {
    match load_nested_repo_detail(state, &path) {
        Ok(()) => {
            state.nested_repo_detail_path = Some(path.clone());
            state.nested_repo_branch_view = BranchView::Local;
            state.nested_repo_branches_idx = state
                .nested_repo_branches
                .iter()
                .position(|branch| branch.is_current)
                .unwrap_or(0);
            state.nested_repo_remote_branches_idx = 0;
            state.nested_repo_tree_idx = tree_idx_for_repo_path(state, &path).unwrap_or(0);
            state.set_status(format!("opened {path} branches"), false);
        }
        Err(err) => state.set_status(format!("load nested branches failed: {err}"), true),
    }
}

pub(crate) fn reload_nested_repo_detail(state: &mut AppState) {
    if let Some(path) = state.nested_repo_detail_path.clone()
        && let Err(err) = load_nested_repo_detail(state, &path)
    {
        state.set_status(format!("load nested branches failed: {err}"), true);
    }
}

pub(super) fn load_nested_repo_detail(state: &mut AppState, path: &str) -> anyhow::Result<()> {
    if let Some(root) = state.workspace_root.as_deref() {
        let root = std::path::Path::new(root);
        state.nested_repo_branches = crate::git::nested_repo_branches_at(root, path)?;
        state.nested_repo_remote_branches = crate::git::nested_repo_remote_branches_at(root, path)?;
    } else {
        state.nested_repo_branches = crate::git::nested_repo_branches(path)?;
        state.nested_repo_remote_branches = crate::git::nested_repo_remote_branches(path)?;
    }
    state.clamp();
    Ok(())
}

pub(crate) fn close_nested_repo_detail(state: &mut AppState) {
    if let Some(path) = state.nested_repo_detail_path.take() {
        state.nested_repo_branches.clear();
        state.nested_repo_remote_branches.clear();
        state.nested_repo_branch_view = BranchView::Local;
        state.nested_repo_branches_idx = 0;
        state.nested_repo_remote_branches_idx = 0;
        state.nested_repo_tree_idx = tree_idx_for_repo_path(state, &path).unwrap_or(0);
    }
}

/// Show a session in the main pane and give it the keyboard.
pub(super) fn show_session_row(state: &mut AppState, id: crate::session::SessionId) {
    if state.sessions.get(id).is_none() {
        return;
    }
    state.show_session(id);
    state.session_capture = true;
}
