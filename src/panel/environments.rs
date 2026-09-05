//! The workspace pane: every checkout, its sessions, and the deployment status.

use ratatui::{Frame, layout::Rect, style::Color};

use crate::{
    app,
    session::SessionKind,
    state::{AppState, BranchView},
};

use super::scroll;

mod actions;
mod draw;
mod tree;

pub(crate) use actions::{
    activate_selected_repository_row, close_nested_repo_detail, reload_nested_repo_detail,
    selected_checkout_label, selected_linked_worktree, selected_session,
    start_session_for_selection,
};
pub(crate) use draw::{nested_repo_scroll_offset, sync_scroll_offset};
pub(crate) use tree::{nested_repo_tree_len, select_nested_repo_tree_row};

use actions::{
    bring_selected_worktree_home, close_selected_session, land_selected_worktree,
    load_nested_repo_detail, open_new_worktree_form, remove_selected_worktree,
    selected_repository_project_path, show_session_row, sync_selected_worktree,
};
use draw::render_nested_repositories;
use tree::{NestedRepoTreeRow, move_selection, selected_tree_row};

/// Borders plus the branch line plus the `main` row. Deploy branches add one
/// row each, and a checkout does not have to have both.
const DEPLOYMENT_STATUS_BASE_HEIGHT: u16 = 4;
const MIN_REPOSITORY_TREE_WITH_DEPLOYMENT: u16 = 6;
const ACTIVE_REPOSITORY_BG: Color = Color::Rgb(24, 54, 34);

/// Draw the checkouts and what is running in them.
///
/// Always the tree, even for a lone project with nothing nested under it. The
/// tree is the only place a session can be started or returned to, and a
/// checkout on its own needs that as much as a workspace of ten does — it used
/// to be replaced here by the deployment box, which left the one project case
/// with no way to reach a session at all. The deployment box is still shown,
/// underneath, whenever the pane is tall enough for both.
pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    render_nested_repositories(state, area, frame, focused);
}

pub fn handle_key(
    state: &mut AppState,
    key: ratatui::crossterm::event::KeyEvent,
) -> anyhow::Result<bool> {
    use ratatui::crossterm::event::KeyCode;

    if state.nested_repositories.is_empty()
        && state.workspace_root.is_none()
        && state.repo_root.is_none()
    {
        // Nothing to steer. The keys are still this pane's, so swallow them
        // rather than telling the user they do not exist.
        return Ok(true);
    }
    state.clamp();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => move_selection(state, true, 1),
        KeyCode::Char('k') | KeyCode::Up => move_selection(state, false, 1),
        KeyCode::Enter => match selected_tree_row(state) {
            Some(NestedRepoTreeRow::Session { id }) => show_session_row(state, id),
            Some(
                NestedRepoTreeRow::Root
                | NestedRepoTreeRow::Repo { .. }
                | NestedRepoTreeRow::Worktree { .. },
            ) => {
                activate_selected_repository_row(state);
            }
            Some(NestedRepoTreeRow::Branch {
                repo_idx,
                branch_idx,
            }) => {
                if let (Some(repo), Some(branch)) = (
                    state.nested_repositories.get(repo_idx),
                    state.nested_repo_branches.get(branch_idx),
                ) {
                    app::checkout_nested_branch_async(
                        state,
                        repo.path.clone(),
                        branch.name.clone(),
                    );
                }
            }
            Some(NestedRepoTreeRow::Remote {
                repo_idx,
                branch_idx,
            }) => {
                if let Some(repo) = state.nested_repositories.get(repo_idx) {
                    let branch = state
                        .visible_nested_repo_remote_branches()
                        .nth(branch_idx)
                        .map(|branch| branch.name.clone());
                    if let Some(branch) = branch {
                        app::checkout_nested_remote_branch_async(state, repo.path.clone(), branch);
                    }
                }
            }
            None => {}
        },
        KeyCode::Char('o') => {
            if let Some(path) = selected_repository_project_path(state) {
                state.pending_action = Some(crate::state::PendingAction::OpenProjectAt(path));
            }
        }
        KeyCode::Char('n') => open_new_worktree_form(state),
        KeyCode::Char('s') => state.open_agent_picker(true),
        KeyCode::Char('S') => state.open_agent_picker(false),
        KeyCode::Char('t') => start_session_for_selection(state, SessionKind::Terminal, true),
        KeyCode::Char('T') => start_session_for_selection(state, SessionKind::Terminal, false),
        KeyCode::Char('D') => remove_selected_worktree(state),
        KeyCode::Char('x') => close_selected_session(state),
        KeyCode::Char('m') => land_selected_worktree(state),
        KeyCode::Char('M') => sync_selected_worktree(state),
        KeyCode::Char('b') => bring_selected_worktree_home(state),
        KeyCode::Char('r') if state.nested_repo_detail_path.is_some() => {
            state.nested_repo_branch_view = match state.nested_repo_branch_view {
                BranchView::Local => BranchView::Remote,
                BranchView::Remote => BranchView::Local,
            };
            if let Some(path) = state.nested_repo_detail_path.clone() {
                let _ = load_nested_repo_detail(state, &path);
            }
            state.clamp();
        }
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('h') => {
            close_nested_repo_detail(state);
        }
        _ => return Ok(false),
    }
    Ok(true)
}
