//! The action menu for the selected row of the repository tree.
//!
//! The tree nests repositories, worktrees, sessions and branches, and each
//! kind of row has its own things that can be done to it. Rather than a
//! footer of twenty letters, `Space` lists what applies to the row under the
//! cursor. The everyday keys — start an agent, a terminal, a worktree — keep
//! their letters and the menu says so; the rest live here alone.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::{
    session::SessionKind,
    state::{AppState, BranchView, Modal},
    ui,
};

use super::actions::{
    bring_selected_worktree_home, close_selected_session, init_available, init_selected_checkout,
    land_selected_worktree, open_new_worktree_form, remove_selected_worktree, selected_checkout,
    selected_linked_worktree, selected_repository_project_path, start_session_for_selection,
};
use super::tree::{NestedRepoTreeRow, selected_tree_row};

/// Everything the menu can do. Each entry names the row kinds it applies to
/// in [`available`], so a worktree-only action never shows up on a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoAction {
    /// What Enter does on this row: expand, check out, or show the session.
    Activate,
    StartAgent,
    Terminal,
    TerminalNoSandbox,
    NewWorktree,
    CloseSession,
    LandWorktree,
    SyncWorktree,
    BranchHome,
    RemoveWorktree,
    OpenIde,
    ToggleRemotes,
    GitInit,
    Environments,
}

impl RepoAction {
    /// The letter that also does this from the tree, for the actions common
    /// enough to keep one.
    pub fn shortcut(self) -> Option<&'static str> {
        Some(match self {
            Self::Activate => "Enter",
            Self::StartAgent => "s",
            Self::Terminal => "t",
            Self::NewWorktree => "n",
            Self::CloseSession => "x",
            Self::OpenIde => "o",
            Self::ToggleRemotes => "r",
            Self::Environments => "E",
            _ => return None,
        })
    }
}

/// What the menu offers for the selected row, in the order it lists them.
pub fn available(state: &AppState) -> Vec<RepoAction> {
    let Some(row) = selected_tree_row(state) else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    let checkout = selected_checkout(state).is_some();
    match row {
        NestedRepoTreeRow::Session { .. } => {
            actions.extend([RepoAction::Activate, RepoAction::CloseSession]);
        }
        NestedRepoTreeRow::Branch { .. } | NestedRepoTreeRow::Remote { .. } => {
            actions.push(RepoAction::Activate);
        }
        NestedRepoTreeRow::Root
        | NestedRepoTreeRow::Repo { .. }
        | NestedRepoTreeRow::Worktree { .. } => {}
    }
    if checkout {
        actions.extend([
            RepoAction::StartAgent,
            RepoAction::Terminal,
            RepoAction::TerminalNoSandbox,
            RepoAction::NewWorktree,
        ]);
    }
    if selected_linked_worktree(state).is_some() {
        actions.extend([
            RepoAction::LandWorktree,
            RepoAction::SyncWorktree,
            RepoAction::BranchHome,
            RepoAction::RemoveWorktree,
        ]);
    }
    if selected_repository_project_path(state).is_some() {
        actions.push(RepoAction::OpenIde);
    }
    if state.nested_repo_detail_path.is_some() {
        actions.push(RepoAction::ToggleRemotes);
    }
    if init_available(state) {
        actions.push(RepoAction::GitInit);
    }
    actions.push(RepoAction::Environments);
    actions
}

/// The words for each entry, given the row it is about.
fn label(state: &AppState, action: RepoAction) -> String {
    match action {
        RepoAction::Activate => match selected_tree_row(state) {
            Some(NestedRepoTreeRow::Session { .. }) => "Show session".into(),
            Some(NestedRepoTreeRow::Branch { .. } | NestedRepoTreeRow::Remote { .. }) => {
                "Check out this branch".into()
            }
            _ => "Expand".into(),
        },
        RepoAction::StartAgent => "Start an agent here…".into(),
        RepoAction::Terminal => "Open a terminal here (Terrarium sandbox)".into(),
        RepoAction::TerminalNoSandbox => "Open a terminal here, no sandbox".into(),
        RepoAction::NewWorktree => "New worktree…".into(),
        RepoAction::CloseSession => "Close session".into(),
        RepoAction::LandWorktree => "Land worktree: merge into main and clean up".into(),
        RepoAction::SyncWorktree => "Sync main into this worktree".into(),
        RepoAction::BranchHome => "Move its branch to the main checkout".into(),
        RepoAction::RemoveWorktree => "Remove worktree".into(),
        RepoAction::OpenIde => "Open in IDE".into(),
        RepoAction::ToggleRemotes => match state.nested_repo_branch_view {
            BranchView::Local => "Show remote branches".into(),
            BranchView::Remote => "Show local branches".into(),
        },
        RepoAction::GitInit => "Make this folder a Git repository".into(),
        RepoAction::Environments => "Environments and promotion…".into(),
    }
}

pub fn open(state: &mut AppState) {
    if available(state).is_empty() {
        state.set_status("select a repository, worktree or session first", false);
        return;
    }
    state.repo_menu_idx = 0;
    state.modal = Modal::RepoActions;
}

/// Do `action` to the selected row, the same as its letter would.
pub fn run(state: &mut AppState, action: RepoAction) {
    match action {
        RepoAction::Activate => {
            let _ = super::handle_key(state, KeyEvent::from(KeyCode::Enter));
        }
        RepoAction::StartAgent => state.open_agent_picker(true),
        RepoAction::Terminal => start_session_for_selection(state, SessionKind::Terminal, true),
        RepoAction::TerminalNoSandbox => {
            start_session_for_selection(state, SessionKind::Terminal, false)
        }
        RepoAction::NewWorktree => open_new_worktree_form(state),
        RepoAction::CloseSession => close_selected_session(state),
        RepoAction::LandWorktree => land_selected_worktree(state),
        RepoAction::SyncWorktree => sync_selected_worktree(state),
        RepoAction::BranchHome => bring_selected_worktree_home(state),
        RepoAction::RemoveWorktree => remove_selected_worktree(state),
        RepoAction::OpenIde => {
            if let Some(path) = selected_repository_project_path(state) {
                state.pending_action = Some(crate::state::PendingAction::OpenProjectAt(path));
            }
        }
        RepoAction::ToggleRemotes => super::toggle_remote_branches(state),
        RepoAction::GitInit => init_selected_checkout(state),
        RepoAction::Environments => super::super::deployment::open(state),
    }
}

use super::actions::sync_selected_worktree;

fn title(state: &AppState) -> String {
    let what = match selected_tree_row(state) {
        Some(NestedRepoTreeRow::Session { id }) => state
            .sessions
            .get(id)
            .map(|session| format!("session {}", session.label)),
        Some(NestedRepoTreeRow::Worktree { .. }) => {
            selected_checkout(state).map(|(_, label)| format!("worktree {label}"))
        }
        Some(NestedRepoTreeRow::Branch { .. } | NestedRepoTreeRow::Remote { .. }) => {
            Some("branch".to_string())
        }
        _ => selected_checkout(state).map(|(_, label)| format!("repository {label}")),
    };
    match what {
        Some(what) => format!("Actions for {what}"),
        None => "Actions".to_string(),
    }
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let actions = available(state);
    let height = actions.len() as u16 + 4;
    let modal = ui::centered(area, 66.min(area.width), height.min(area.height));
    let inner = ui::modal_frame(frame, modal, &title(state));
    let (chunks, dividers) =
        ui::modal_rows(frame, inner, &[Constraint::Min(1), Constraint::Length(1)]);

    let items: Vec<ListItem> = actions
        .iter()
        .map(|action| {
            let shortcut = action.shortcut().unwrap_or("");
            ListItem::new(Line::from(vec![
                Span::styled(format!("{shortcut:<6}"), Style::default().fg(Color::Yellow)),
                Span::raw(label(state, *action)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(crate::ui::palette::selection())
        .highlight_symbol("\u{203a} ");
    let mut list_state = super::scroll::list_state(
        Some(state.repo_menu_idx.min(actions.len().saturating_sub(1))),
        0,
    );
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" run  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" close  "),
            Span::styled(
                "the letters shown also work from the tree",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
        ])),
        chunks[1],
    );
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let actions = available(state);
    let last = actions.len().saturating_sub(1);
    state.repo_menu_idx = state.repo_menu_idx.min(last);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.repo_menu_idx = (state.repo_menu_idx + 1).min(last)
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.repo_menu_idx = state.repo_menu_idx.saturating_sub(1)
        }
        KeyCode::Enter => {
            if let Some(action) = actions.get(state.repo_menu_idx).copied() {
                state.modal = Modal::None;
                run(state, action);
            }
        }
        KeyCode::Esc | KeyCode::Char(' ') => state.modal = Modal::None,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Worktree;
    use crate::state::PendingAction;

    fn worktree(path: &str, branch: &str, is_main: bool) -> Worktree {
        Worktree {
            path: path.into(),
            branch: Some(branch.into()),
            head: "0123456789abcdef0123456789abcdef01234567".into(),
            is_main,
            bare: false,
            locked: None,
            prunable: None,
            has_changes: false,
            unmerged: Some(0),
        }
    }

    fn workspace() -> AppState {
        let mut state = AppState::new();
        state.workspace_root = Some("/workspace".into());
        state.repo_root = Some("/workspace".into());
        state.worktrees = vec![
            worktree("/workspace", "main", true),
            worktree("/workspace.worktrees/feat-x", "feat/x", false),
        ];
        state
    }

    /// The worktree-only actions are offered on a worktree row and nowhere
    /// else, so the menu never lists something the row cannot do.
    #[test]
    fn the_menu_offers_what_the_selected_row_can_do() {
        let mut state = workspace();

        assert!(!available(&state).contains(&RepoAction::LandWorktree));
        assert!(available(&state).contains(&RepoAction::StartAgent));

        state.nested_repo_tree_idx = 1;
        let offered = available(&state);
        assert!(offered.contains(&RepoAction::LandWorktree), "{offered:?}");
        assert!(offered.contains(&RepoAction::RemoveWorktree), "{offered:?}");
    }

    /// Choosing an entry does the same as its letter used to: landing a
    /// worktree parks the merge behind the usual confirmation.
    #[test]
    fn enter_runs_the_highlighted_action_and_closes_the_menu() {
        let mut state = workspace();
        state.nested_repo_tree_idx = 1;
        open(&mut state);
        assert_eq!(state.modal, Modal::RepoActions);

        let land = available(&state)
            .iter()
            .position(|action| *action == RepoAction::LandWorktree)
            .expect("land is offered");
        for _ in 0..land {
            handle_key(&mut state, KeyEvent::from(KeyCode::Char('j'))).unwrap();
        }
        handle_key(&mut state, KeyEvent::from(KeyCode::Enter)).unwrap();

        assert_ne!(state.modal, Modal::RepoActions, "the menu is done with");
        assert_eq!(
            state.confirm.as_ref().map(|c| c.action.clone()),
            Some(PendingAction::LandWorktree {
                path: "/workspace.worktrees/feat-x".into(),
                branch: "feat/x".into(),
            })
        );
    }
}
