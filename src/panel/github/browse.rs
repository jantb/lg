//! The keys while browsing: switching tabs, filters and owners, and acting on
//! the selected pull request or repository.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::github::PrFilter;
use crate::panel::text_input::Edit;
use crate::state::{AppState, GitHubAction, Modal, PendingAction};

use super::forms::{
    compose, compose_key, confirm_close_key, create_key, merge_key, start_create, start_merge,
};
use super::load::{load_owners, load_pull_requests, load_repositories, step_owner};
use super::lookup::{
    clone_destination, clone_request, selected_pr, visible_repos, worktree_path_for,
};
use super::{Compose, CreateField, Mode, Tab, inbox};

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match state.github.mode {
        Mode::Browse => match state.github.tab {
            Tab::Inbox => inbox::handle_key(state, key),
            Tab::PullRequests => browse_pull_requests(state, key),
            Tab::Repositories => browse_repositories(state, key),
        },
        Mode::Compose { .. } => compose_key(state, key),
        Mode::Merge(_) => merge_key(state, key),
        Mode::ConfirmClose => confirm_close_key(state, key),
        Mode::Create(_) => create_key(state, key),
    }
    Ok(())
}

pub(super) fn switch_tab(state: &mut AppState, forward: bool) {
    show_tab(state, state.github.tab.next(forward));
}

/// Move to `tab`, reading its list if there is none yet.
pub(super) fn show_tab(state: &mut AppState, tab: Tab) {
    state.github.tab = tab;
    match tab {
        Tab::Inbox
            if state.github.inbox.loading.is_none() && state.github.inbox.items().is_empty() =>
        {
            inbox::reload(state)
        }
        Tab::PullRequests if state.github.prs_loading.is_none() && state.github.prs.is_empty() => {
            load_pull_requests(state)
        }
        Tab::Repositories
            if state.github.repos_loading.is_none() && state.github.repos.is_empty() =>
        {
            load_repositories(state)
        }
        _ => {}
    }
}

pub(super) fn select_pr(state: &mut AppState, idx: usize) {
    state.github.pr_idx = idx.min(state.github.prs.len().saturating_sub(1));
    state.github.detail_scroll = 0;
}

pub(super) fn dispatch(state: &mut AppState, action: GitHubAction) {
    state.pending_action = Some(PendingAction::GitHub(action));
}

fn browse_pull_requests(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
            return;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            switch_tab(state, key.code == KeyCode::Tab);
            return;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            select_pr(state, state.github.pr_idx + 1);
            return;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            select_pr(state, state.github.pr_idx.saturating_sub(1));
            return;
        }
        KeyCode::Char('g') | KeyCode::Home => {
            select_pr(state, 0);
            return;
        }
        KeyCode::Char('G') | KeyCode::End => {
            select_pr(state, usize::MAX);
            return;
        }
        KeyCode::Char('d') if ctrl => {
            state.github.detail_scroll = state.github.detail_scroll.saturating_add(5);
            return;
        }
        KeyCode::Char('u') if ctrl => {
            state.github.detail_scroll = state.github.detail_scroll.saturating_sub(5);
            return;
        }
        KeyCode::Char('r') => {
            load_pull_requests(state);
            return;
        }
        KeyCode::Char('s') => {
            next_filter(state);
            return;
        }
        KeyCode::Char('h') => {
            toggle_drafts(state);
            return;
        }
        KeyCode::Char('n') => {
            start_create(state);
            return;
        }
        KeyCode::Char('o') => {
            let url = selected_pr(state)
                .map(|pr| pr.url.clone())
                .or_else(|| state.github.repo.as_ref().map(|repo| repo.url.clone()))
                .filter(|url| !url.is_empty());
            match url {
                Some(url) => dispatch(state, GitHubAction::OpenInBrowser { url }),
                None => state.set_status("nothing to open yet", false),
            }
            return;
        }
        _ => {}
    }

    let Some(pr) = selected_pr(state).cloned() else {
        if matches!(key.code, KeyCode::Char(_) | KeyCode::Enter) {
            state.set_status("no pull request selected", false);
        }
        return;
    };
    let number = pr.number;
    // Reviewing, merging and drafting are for pull requests still open;
    // saying why is better than a gh error about it.
    let still_open = |state: &mut AppState| {
        if pr.is_open() {
            true
        } else {
            state.set_status(format!("#{number} is {}", pr.state.label()), false);
            false
        }
    };
    match key.code {
        KeyCode::Enter => {
            state.modal = Modal::None;
            dispatch(state, GitHubAction::Checkout { number });
        }
        KeyCode::Char('w') => match worktree_path_for(state, &pr) {
            Some(path) => {
                state.modal = Modal::None;
                dispatch(state, GitHubAction::CheckoutWorktree { number, path });
            }
            None => state.set_status("no main worktree to put a new one next to", true),
        },
        KeyCode::Char('v') => crate::panel::guided::open_pull_request(state, &pr),
        KeyCode::Char('a') if still_open(state) => compose(state, Compose::Approve),
        KeyCode::Char('x') if still_open(state) => compose(state, Compose::RequestChanges),
        KeyCode::Char('c') => compose(state, Compose::Comment),
        KeyCode::Char('m') if still_open(state) => start_merge(state, &pr),
        KeyCode::Char('D') if still_open(state) => dispatch(
            state,
            GitHubAction::SetDraft {
                number,
                draft: !pr.draft,
            },
        ),
        KeyCode::Char('X') => match pr.state {
            crate::github::PrState::Open => state.github.mode = Mode::ConfirmClose,
            crate::github::PrState::Closed => dispatch(state, GitHubAction::Reopen { number }),
            crate::github::PrState::Merged => {
                state.set_status(format!("#{number} is merged and cannot be reopened"), false)
            }
        },
        _ => {}
    }
}

fn browse_repositories(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let visible = visible_repos(&state.github).len();
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Tab | KeyCode::BackTab => switch_tab(state, key.code == KeyCode::Tab),
        KeyCode::Down => {
            state.github.repo_idx = (state.github.repo_idx + 1).min(visible.saturating_sub(1))
        }
        KeyCode::Up => state.github.repo_idx = state.github.repo_idx.saturating_sub(1),
        // With a search typed, the arrows move through it; with none, there
        // is nothing to move through, and they step between owners.
        KeyCode::Right | KeyCode::Left | KeyCode::Home | KeyCode::End
            if !state.github.query.is_empty() =>
        {
            state.github.query.handle_key(key);
        }
        KeyCode::Right => step_owner(state, true),
        KeyCode::Left => step_owner(state, false),
        KeyCode::Char('r') if ctrl => {
            load_owners(state);
            load_repositories(state);
        }
        KeyCode::Char('u') if ctrl => {
            state.github.query.clear();
            state.github.repo_idx = 0;
        }
        KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_) if !ctrl => {
            if state.github.query.edit_key(key) == Edit::Changed {
                state.github.repo_idx = 0;
            }
        }
        KeyCode::Enter => clone_selected(state),
        _ => {}
    }
}

fn clone_selected(state: &mut AppState) {
    let Some(repo) = clone_request(state) else {
        state.set_status("pick a repository, or type owner/name", false);
        return;
    };
    let Some(dir) = clone_destination(state) else {
        state.set_status(format!("no folder to clone {repo} into"), true);
        return;
    };
    if dir.exists() {
        // Already here: going to it is what cloning it again would have been for.
        if crate::git::repo_root_at(&dir).is_some() {
            state.modal = Modal::None;
            state.pending_action = Some(PendingAction::SwitchRepository {
                target: crate::state::RepoTarget::Path(dir),
            });
        } else {
            state.set_status(
                format!("{} exists and is not a repository", dir.display()),
                true,
            );
        }
        return;
    }
    state.modal = Modal::None;
    dispatch(
        state,
        GitHubAction::Clone {
            repo,
            dir: dir.to_string_lossy().into_owned(),
        },
    );
}

/// Text pasted while the modal takes typing. Returns whether it was taken.
pub fn handle_paste(state: &mut AppState, text: &str) -> bool {
    if state.modal != Modal::GitHub {
        return false;
    }
    let one_line = || text.replace(['\r', '\n'], " ");
    match &mut state.github.mode {
        Mode::Compose { text: typed, .. } => typed.insert_str(&one_line()),
        Mode::Create(form) => match form.field {
            CreateField::Body => form.body.insert_str(&text.replace('\r', "")),
            CreateField::Title => form.title.insert_str(&one_line()),
            CreateField::Base => form.base.insert_str(one_line().trim()),
            CreateField::Draft => return false,
        },
        Mode::Browse if state.github.tab == Tab::Repositories => {
            state.github.query.insert_str(one_line().trim());
            state.github.repo_idx = 0;
        }
        _ => return false,
    }
    true
}

pub(super) fn next_filter(state: &mut AppState) {
    show_filter(state, state.github.filter.next());
}

pub(super) fn show_filter(state: &mut AppState, filter: PrFilter) {
    state.github.filter = filter;
    select_pr(state, 0);
    load_pull_requests(state);
}

pub(super) fn toggle_drafts(state: &mut AppState) {
    state.github.hide_drafts = !state.github.hide_drafts;
    select_pr(state, 0);
    load_pull_requests(state);
}
