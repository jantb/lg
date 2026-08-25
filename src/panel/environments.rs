use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::{
    app,
    config::BRANCH_MAIN,
    git::{Branch, NestedRepo, ReleaseEnv, ReleaseTargetStatus, RemoteBranch, Worktree, same_dir},
    state::{AppState, BranchView, SPINNER_FRAMES, clamp_index},
    ui,
};

use super::scroll;

/// Borders plus the branch line plus the `main` row. Deploy branches add one
/// row each, and a checkout does not have to have both.
const DEPLOYMENT_STATUS_BASE_HEIGHT: u16 = 4;
const MIN_REPOSITORY_TREE_WITH_DEPLOYMENT: u16 = 6;
const ACTIVE_REPOSITORY_BG: Color = Color::Rgb(24, 54, 34);

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    // Workspace mode is built around this tree, so it is always drawn there —
    // even for a lone checkout, which is where a session gets started from.
    if !state.git_panes_visible()
        || !state.nested_repositories.is_empty()
        || !root_repo_selected(state)
        || state.has_linked_worktrees()
    {
        render_nested_repositories(state, area, frame, focused);
        return;
    }

    if !state.flow_available() {
        frame.render_widget(Paragraph::new(""), area);
        return;
    }

    render_deployment_status(state, area, frame);
}

/// Height the deployment box needs in this checkout: only the deploy branches
/// that exist get a row.
fn deployment_status_height(state: &AppState) -> u16 {
    DEPLOYMENT_STATUS_BASE_HEIGHT + u16::try_from(release_envs(state).len()).unwrap_or(0)
}

/// The deploy environments this checkout has, in promotion order.
fn release_envs(state: &AppState) -> Vec<(ReleaseEnv, String)> {
    [ReleaseEnv::Dev, ReleaseEnv::Test]
        .into_iter()
        .filter_map(|env| {
            state
                .release_branch(env)
                .map(|branch| (env, branch.to_string()))
        })
        .collect()
}

fn render_deployment_status(state: &AppState, area: Rect, frame: &mut Frame) {
    let block = ui::bordered("Deployment Status");
    let mut lines = Vec::new();

    match state.branch.as_deref() {
        Some(branch) => lines.push(Line::from(vec![
            Span::styled("branch ", Style::default().fg(Color::DarkGray)),
            Span::styled(branch.to_string(), Style::default().fg(Color::Green)),
        ])),
        None => lines.push(Line::from(Span::styled(
            "detached HEAD",
            Style::default().fg(Color::Red),
        ))),
    }

    lines.push(env_line(
        BRANCH_MAIN,
        state.current_branch_releases.main.as_ref(),
        Color::Magenta,
        state.animation_tick,
        release_status_loading(state),
    ));
    for (env, branch) in release_envs(state) {
        let (status, color) = match env {
            ReleaseEnv::Dev => (state.current_branch_releases.develop.as_ref(), Color::Cyan),
            ReleaseEnv::Test => (state.current_branch_releases.test.as_ref(), Color::Yellow),
        };
        lines.push(env_line(
            &branch,
            status,
            color,
            state.animation_tick,
            release_status_loading(state),
        ));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_nested_repositories(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let deployment_height = deployment_status_height(state);
    let show_deployment = state.flow_available()
        && area.height >= deployment_height + MIN_REPOSITORY_TREE_WITH_DEPLOYMENT;
    let (tree_area, deployment_area) = if show_deployment {
        let chunks = Layout::vertical([
            Constraint::Min(MIN_REPOSITORY_TREE_WITH_DEPLOYMENT),
            Constraint::Length(deployment_height),
        ])
        .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };
    let rows = nested_repo_tree_rows(state);
    let len = rows.len();
    let selected_idx = clamp_index(state.nested_repo_tree_idx, len);
    let title = repositories_title(state);
    let block = ui::framed_with_activity(
        1,
        &title,
        focused,
        selected_idx.map(|idx| (idx + 1, len)),
        state.animation_tick,
        state.activity_label().is_some(),
    );
    let row_width = tree_area.width.saturating_sub(4) as usize;
    let items = rows
        .iter()
        .map(|row| match row {
            NestedRepoTreeRow::Root => {
                repository_list_item(root_repo_line(state, row_width), root_repo_selected(state))
            }
            NestedRepoTreeRow::Session { id } => match state.sessions.get(*id) {
                Some(session) => {
                    let shown = state.session_view() == Some(*id);
                    repository_list_item(session_line(session, row_width, shown), shown)
                }
                None => ListItem::new(Line::from("")),
            },
            NestedRepoTreeRow::Worktree { wt_idx } => {
                let worktree = &state.worktrees[*wt_idx];
                let active = worktree_selected(state, worktree);
                repository_list_item(worktree_line(worktree, row_width, active), active)
            }
            NestedRepoTreeRow::Repo { repo_idx } => {
                let repo = &state.nested_repositories[*repo_idx];
                let expanded = state
                    .nested_repositories
                    .get(*repo_idx)
                    .is_some_and(|repo| {
                        state.nested_repo_detail_path.as_deref() == Some(&repo.path)
                    });
                let active = nested_repo_selected(state, &repo.path);
                repository_list_item(nested_repo_line(repo, row_width, expanded, active), active)
            }
            NestedRepoTreeRow::Branch { branch_idx, .. } => ListItem::new(nested_branch_line(
                &state.nested_repo_branches[*branch_idx],
                row_width,
            )),
            NestedRepoTreeRow::Remote { branch_idx, .. } => {
                ListItem::new(nested_remote_branch_line(
                    state
                        .visible_nested_repo_remote_branches()
                        .nth(*branch_idx)
                        .expect("visible remote row index"),
                    row_width,
                ))
            }
        })
        .collect::<Vec<_>>();
    let offset = nested_repo_scroll_offset(state, tree_area);
    let mut list_state = scroll::list_state(focused.then_some(selected_idx).flatten(), offset);
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");
    frame.render_stateful_widget(list, tree_area, &mut list_state);
    if let Some(area) = deployment_area {
        render_deployment_status(state, area, frame);
    }
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    let len = nested_repo_tree_rows(state).len();
    let selected_idx = clamp_index(state.nested_repo_tree_idx, len);
    state.nested_repositories_scroll_offset = scroll::selection_scroll_offset(
        selected_idx,
        len,
        scroll::list_viewport_height(area.height),
        state.nested_repositories_scroll_offset,
    );
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
        KeyCode::Char('s') => start_session_for_selection(state, true),
        KeyCode::Char('S') => start_session_for_selection(state, false),
        KeyCode::Char('D') => remove_selected_worktree(state),
        KeyCode::Char('x') => close_selected_session(state),
        KeyCode::Char('m') => land_selected_worktree(state),
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
fn start_session_for_selection(state: &mut AppState, sandboxed: bool) {
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
fn open_new_worktree_form(state: &mut AppState) {
    if state.repo_root.is_none() && state.worktrees.is_empty() {
        state.set_status("no repository to add a worktree to", true);
        return;
    }
    state.open_worktree_modal(crate::git::preferred_base_ref());
}

/// Remove the selected worktree, or forget one whose directory is already
/// gone. The checkout lg is showing cannot be removed from under itself, and
/// neither can the main worktree — git refuses that too.
fn remove_selected_worktree(state: &mut AppState) {
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

/// Close the selected session and forget it. A session that has ended keeps
/// its last screen so the reason can be read, which leaves a row behind; this
/// is how that row is cleared from where it is actually seen, rather than only
/// from inside the session pane.
fn close_selected_session(state: &mut AppState) {
    let Some(NestedRepoTreeRow::Session { id }) = selected_tree_row(state) else {
        state.set_status("select a session row to close it", false);
        return;
    };
    let label = state
        .sessions
        .get(id)
        .map(|session| session.label.clone())
        .unwrap_or_default();
    let ended = state
        .sessions
        .get(id)
        .is_some_and(|session| !session.is_running());
    state.sessions.close(id);
    if state.session_view().is_none() {
        state.show_diff();
    }
    state.clamp();
    state.set_status(
        if ended {
            format!("cleared the finished session in {label}")
        } else {
            format!("closed the session in {label}")
        },
        false,
    );
}

/// Merge the selected worktree's branch into main and clean up after it. The
/// merge itself runs in whichever checkout holds main, so it works from here
/// no matter which checkout lg is currently showing.
fn land_selected_worktree(state: &mut AppState) {
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

/// Move the selected worktree's branch back to the main checkout, so the work
/// carries on there. Nothing is merged and the branch stays as it is.
fn bring_selected_worktree_home(state: &mut AppState) {
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
fn activate_worktree_row(state: &mut AppState, wt_idx: usize) -> bool {
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

/// Whether this worktree is the checkout every other panel is showing.
fn worktree_selected(state: &AppState, worktree: &Worktree) -> bool {
    state.repo_root.as_deref().is_some_and(|root| {
        same_dir(
            std::path::Path::new(root),
            std::path::Path::new(&worktree.path),
        )
    })
}

fn selected_repository_project_path(state: &AppState) -> Option<String> {
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

fn load_nested_repo_detail(state: &mut AppState, path: &str) -> anyhow::Result<()> {
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

pub(crate) fn nested_repo_scroll_offset(state: &AppState, area: Rect) -> usize {
    let len = nested_repo_tree_rows(state).len();
    scroll::selection_scroll_offset(
        clamp_index(state.nested_repo_tree_idx, len),
        len,
        scroll::list_viewport_height(area.height),
        state.nested_repositories_scroll_offset,
    )
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

fn move_selection(state: &mut AppState, down: bool, amount: usize) {
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
enum NestedRepoTreeRow {
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

fn nested_repo_tree_rows(state: &AppState) -> Vec<NestedRepoTreeRow> {
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
fn row_checkout_dir(state: &AppState, row: NestedRepoTreeRow) -> Option<String> {
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

fn selected_tree_row(state: &AppState) -> Option<NestedRepoTreeRow> {
    nested_repo_tree_rows(state)
        .get(state.nested_repo_tree_idx)
        .copied()
}

fn tree_idx_for_repo_path(state: &AppState, path: &str) -> Option<usize> {
    nested_repo_tree_rows(state)
        .iter()
        .position(|row| matches!(row, NestedRepoTreeRow::Repo { repo_idx } if state.nested_repositories.get(*repo_idx).is_some_and(|repo| repo.path == path)))
}

/// Whether the checkout lg is pointed at is the workspace root itself, rather
/// than one of the repositories inside it. True when there is no workspace to
/// distinguish it from.
fn root_repo_selected(state: &AppState) -> bool {
    match (state.workspace_root.as_deref(), state.repo_root.as_deref()) {
        (Some(workspace), Some(repo)) => {
            same_dir(std::path::Path::new(workspace), std::path::Path::new(repo))
        }
        _ => true,
    }
}

fn nested_repo_selected(state: &AppState, repo_path: &str) -> bool {
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

fn repository_list_item(line: Line<'static>, selected: bool) -> ListItem<'static> {
    let item = ListItem::new(line);
    if selected {
        item.style(Style::default().bg(ACTIVE_REPOSITORY_BG))
    } else {
        item
    }
}

/// Names the checkout every other panel is showing, so the active repository
/// is readable from the panel frame and not only from the highlighted row.
fn repositories_title(state: &AppState) -> String {
    let kind = if state.git_panes_visible() {
        "Repositories"
    } else {
        "Checkouts"
    };
    match active_repo_name(state) {
        Some(name) => format!("{kind} \u{2022} {name}"),
        None => kind.to_string(),
    }
}

fn active_repo_name(state: &AppState) -> Option<String> {
    let root = state.repo_root.as_deref()?;
    std::path::Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn nested_repo_line(
    repo: &NestedRepo,
    row_width: usize,
    expanded: bool,
    active: bool,
) -> Line<'static> {
    let branch = repo
        .branch
        .clone()
        .or_else(|| {
            repo.detached_at
                .as_ref()
                .map(|sha| format!("detached@{sha}"))
        })
        .unwrap_or_else(|| "unknown".to_string());
    let marker_width = 4 + if repo.has_changes { 2 } else { 0 };
    let branch_width = branch.chars().count().saturating_add(1);
    let max_path_width = row_width
        .saturating_sub(marker_width)
        .saturating_sub(branch_width);

    let mut spans = Vec::new();
    spans.push(Span::styled(
        if expanded { "\u{25be} " } else { "\u{25b8} " },
        Style::default().fg(Color::LightMagenta),
    ));
    spans.push(Span::styled(
        if active { "* " } else { "  " },
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ));
    if repo.has_changes {
        spans.push(Span::styled(
            "! ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        truncate_chars(&repo.path, max_path_width),
        if active {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        },
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        truncate_chars(&branch, row_width.saturating_sub(marker_width + 1)),
        Style::default()
            .fg(if repo.branch.is_some() {
                Color::Green
            } else {
                Color::LightMagenta
            })
            .add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

/// Show a session in the main pane and give it the keyboard.
fn show_session_row(state: &mut AppState, id: crate::session::SessionId) {
    if state.sessions.get(id).is_none() {
        return;
    }
    state.show_session(id);
    state.session_capture = true;
}

/// The colour of a running session's dot: green when it is ready for a command,
/// yellow while it works, red when it is blocked on a question.
fn activity_color(activity: crate::session::SessionActivity) -> Color {
    match activity {
        crate::session::SessionActivity::Idle => Color::Green,
        crate::session::SessionActivity::Working => Color::Yellow,
        crate::session::SessionActivity::NeedsInput => Color::Red,
    }
}

/// What to add after "claude" for an activity, or nothing when it is simply
/// ready — the row is narrow, and "ready" is the state that needs no words.
fn activity_word(activity: crate::session::SessionActivity) -> Option<&'static str> {
    match activity {
        crate::session::SessionActivity::Idle => None,
        crate::session::SessionActivity::Working => Some("working"),
        crate::session::SessionActivity::NeedsInput => Some("needs input"),
    }
}

/// A session under its checkout: whether it is running, what it is doing, and
/// whether it has said something since it was last looked at.
fn session_line(session: &crate::session::Session, row_width: usize, shown: bool) -> Line<'static> {
    let (glyph, glyph_color) = match &session.status {
        crate::session::SessionStatus::Ended(_) => ("\u{25cb} ", Color::DarkGray),
        crate::session::SessionStatus::Running => ("\u{25cf} ", activity_color(session.activity())),
    };
    // The dot says what it is doing; the word repeats it for anyone the colour
    // alone does not reach, and the caret still marks output nobody has read.
    let state_text = match &session.status {
        crate::session::SessionStatus::Ended(notice) => format!("claude {notice}"),
        crate::session::SessionStatus::Running => {
            let mut text = "claude".to_string();
            if let Some(word) = activity_word(session.activity()) {
                text.push_str(" \u{b7} ");
                text.push_str(word);
            }
            if session.attention {
                text.push_str(" \u{25b4}");
            }
            text
        }
    };
    let mut spans = vec![
        Span::styled("    ", Style::default()),
        Span::styled(glyph, Style::default().fg(glyph_color)),
    ];
    spans.push(Span::styled(
        truncate_chars(&state_text, row_width.saturating_sub(6)),
        if shown {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        },
    ));
    if session.sandboxed && row_width > state_text.chars().count() + 16 {
        spans.push(Span::styled(
            " sandboxed",
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

/// One checkout of the active repository: what is checked out there, which
/// directory it is, and whether it can be entered at all.
fn worktree_line(worktree: &Worktree, row_width: usize, active: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled("  \u{2387} ", Style::default().fg(Color::LightMagenta)),
        Span::styled(
            if active { "* " } else { "  " },
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if worktree.has_changes {
        spans.push(Span::styled(
            "! ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let note = worktree_note(worktree);
    let landing = landing_marker(worktree);
    let used = 4
        + if worktree.has_changes { 2 } else { 0 }
        + landing
            .as_ref()
            .map_or(0, |(text, _)| text.chars().count() + 1);
    let note_width = note.as_ref().map_or(0, |note| note.chars().count() + 1);
    let label = worktree.label();
    let label_width = row_width
        .saturating_sub(used)
        .saturating_sub(note_width)
        .saturating_sub(worktree.dir_name().chars().count() + 1);

    spans.push(Span::styled(
        truncate_chars(&label, label_width.max(4)),
        Style::default()
            .fg(if worktree.branch.is_some() {
                Color::Green
            } else {
                Color::LightMagenta
            })
            .add_modifier(Modifier::BOLD),
    ));
    if let Some((text, color)) = landing {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(text, Style::default().fg(color)));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        truncate_chars(&worktree.dir_name(), row_width.saturating_sub(used + 1)),
        Style::default().fg(Color::DarkGray),
    ));
    if let Some(note) = note {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(note, Style::default().fg(Color::Red)));
    }
    Line::from(spans)
}

/// What landing this worktree would move, so `m` is legible before it is
/// pressed: how many commits main is missing, or that it is already merged and
/// only the cleanup is left.
fn landing_marker(worktree: &Worktree) -> Option<(String, Color)> {
    match worktree.unmerged? {
        0 => Some(("merged".to_string(), Color::DarkGray)),
        n => Some((format!("\u{2191}{n}"), Color::Cyan)),
    }
}

/// Why a worktree cannot simply be entered, when that is the case.
fn worktree_note(worktree: &Worktree) -> Option<String> {
    if worktree.is_missing() {
        return Some("missing".to_string());
    }
    if worktree.locked.is_some() {
        return Some("locked".to_string());
    }
    None
}

fn root_repo_line(state: &AppState, row_width: usize) -> Line<'static> {
    let label = state
        .workspace_root
        .as_deref()
        .and_then(|root| std::path::Path::new(root).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let marker = if root_repo_selected(state) {
        "* "
    } else {
        "  "
    };
    Line::from(vec![
        Span::styled("\u{25b8} ", Style::default().fg(Color::LightMagenta)),
        Span::styled(
            format!(
                "{marker}{}",
                truncate_chars(label, row_width.saturating_sub(4))
            ),
            Style::default()
                .fg(if root_repo_selected(state) {
                    Color::Green
                } else {
                    Color::Gray
                })
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn nested_branch_line(branch: &Branch, row_width: usize) -> Line<'static> {
    let prefix = if branch.is_current { "  * " } else { "    " };
    let mut spans = vec![Span::styled(
        format!(
            "{prefix}{}",
            truncate_chars(&branch.name, row_width.saturating_sub(4))
        ),
        if branch.is_current {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        },
    )];
    if branch.ahead > 0 {
        spans.push(Span::styled(
            format!(" \u{2191}{}", branch.ahead),
            Style::default().fg(Color::Green),
        ));
    }
    if branch.behind > 0 {
        spans.push(Span::styled(
            format!(" \u{2193}{}", branch.behind),
            Style::default().fg(Color::Yellow),
        ));
    }
    Line::from(spans)
}

fn nested_remote_branch_line(branch: &RemoteBranch, row_width: usize) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(
            "    {}",
            truncate_chars(&branch.name, row_width.saturating_sub(4))
        ),
        Style::default().fg(Color::LightMagenta),
    )])
}

fn env_line(
    label: &str,
    status: Option<&ReleaseTargetStatus>,
    color: Color,
    tick: usize,
    loading: bool,
) -> Line<'static> {
    let marker = match status {
        Some(s) if s.missing_commits == 0 => "[x]",
        Some(_) => "[~]",
        None if loading => "[~]",
        None => "[ ]",
    };
    let mut spans = vec![
        Span::styled(marker, Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            label.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    match status {
        Some(s) => {
            let released_at = if s.released_at.is_empty() {
                "not merged".to_string()
            } else {
                s.released_at.clone()
            };
            spans.push(Span::styled(released_at, Style::default().fg(Color::Gray)));
            if s.missing_commits > 0 {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("+{}", s.missing_commits),
                    Style::default().fg(Color::Red),
                ));
            }
        }
        None if loading => {
            let pulse = SPINNER_FRAMES[tick % SPINNER_FRAMES.len()];
            spans.push(Span::styled(
                format!("{pulse} checking"),
                Style::default().fg(Color::Gray),
            ));
        }
        None => {
            let pulse = if tick % 2 == 0 {
                SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
            } else {
                "-"
            };
            spans.push(Span::styled(
                format!("{pulse} not merged"),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    Line::from(spans)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() && max_chars > 0 {
        out.pop();
        out.push('\u{2026}');
    }
    out
}

fn release_status_loading(state: &AppState) -> bool {
    state
        .release_status_job
        .as_ref()
        .is_some_and(|job| Some(job.branch.as_str()) == state.branch.as_deref())
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
