//! The review text, merge, close and new pull request forms, and their keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::github::{MergeMethod, NewPullRequest, PullRequest};
use crate::panel::text_input::TextInput;
use crate::state::{AppState, GitHubAction};

use super::browse::{dispatch, select_pr};
use super::lookup::{is_current_branch, selected_pr};
use super::{Compose, CreateField, CreateForm, MergeForm, MergeRow, Mode};

pub(super) fn compose(state: &mut AppState, kind: Compose) {
    state.github.mode = Mode::Compose {
        kind,
        text: TextInput::default(),
    };
}

pub(super) fn start_merge(state: &mut AppState, pr: &PullRequest) {
    let (methods, method) = match &state.github.repo {
        Some(repo) if !repo.merge_methods.is_empty() => {
            (repo.merge_methods.clone(), repo.preferred_merge)
        }
        _ => (MergeMethod::ALL.to_vec(), MergeMethod::Merge),
    };
    state.github.mode = Mode::Merge(MergeForm {
        number: pr.number,
        method,
        methods,
        delete_branch: true,
        auto: false,
        row: MergeRow::Method,
    });
}

/// Open the new pull request form for the branch checked out here, filled in
/// from what the branch carries.
pub(super) fn start_create(state: &mut AppState) {
    let Some(branch) = state.branch.clone() else {
        state.set_status("check a branch out to open a pull request from it", false);
        return;
    };
    if let Some(idx) = state
        .github
        .prs
        .iter()
        .position(|pr| pr.is_open() && is_current_branch(state, pr))
    {
        let number = state.github.prs[idx].number;
        select_pr(state, idx);
        state.set_status(format!("#{number} is already open for {branch}"), false);
        return;
    }
    let base = state
        .github
        .repo
        .as_ref()
        .map(|repo| repo.default_branch.clone())
        .unwrap_or_else(|| "main".to_string());
    if branch == base {
        state.set_status(
            format!("{branch} is the base branch; open pull requests from a feature branch"),
            false,
        );
        return;
    }
    let remote = crate::preferences::remote();
    let commits = crate::git::commit_messages_since(&format!("{remote}/{base}"))
        .or_else(|_| crate::git::commit_messages_since(&base))
        .unwrap_or_default();
    let (title, body) = crate::github::fill_from_commits(&branch, &commits);
    state.github.mode = Mode::Create(CreateForm {
        branch,
        title: title.into(),
        base: base.into(),
        body: body.into(),
        draft: false,
        field: CreateField::Title,
    });
}

pub(super) fn compose_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Mode::Compose { kind, text } = &mut state.github.mode else {
        return;
    };
    let kind = *kind;
    match key.code {
        KeyCode::Esc => state.github.mode = Mode::Browse,
        KeyCode::Char('u') if ctrl => text.clear(),
        KeyCode::Enter => {
            let body = text.trim().to_string();
            if kind.needs_text() && body.is_empty() {
                state.set_status("write something first; Esc cancels", false);
                return;
            }
            let Some(number) = selected_pr(state).map(|pr| pr.number) else {
                state.github.mode = Mode::Browse;
                return;
            };
            state.github.mode = Mode::Browse;
            let action = match kind {
                Compose::Approve => GitHubAction::Review {
                    number,
                    verdict: crate::github::Verdict::Approve,
                    body,
                },
                Compose::RequestChanges => GitHubAction::Review {
                    number,
                    verdict: crate::github::Verdict::RequestChanges,
                    body,
                },
                Compose::Comment => GitHubAction::Comment { number, body },
            };
            dispatch(state, action);
        }
        _ => {
            text.handle_key(key);
        }
    }
}

pub(super) fn merge_key(state: &mut AppState, key: KeyEvent) {
    let Mode::Merge(form) = &mut state.github.mode else {
        return;
    };
    let row_idx = MergeRow::ALL
        .iter()
        .position(|row| *row == form.row)
        .unwrap_or(0);
    let rows = MergeRow::ALL.len();
    match key.code {
        KeyCode::Esc => state.github.mode = Mode::Browse,
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
            form.row = MergeRow::ALL[(row_idx + 1) % rows];
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
            form.row = MergeRow::ALL[(row_idx + rows - 1) % rows];
        }
        KeyCode::Char(' ' | 'h' | 'l') | KeyCode::Left | KeyCode::Right => {
            let forward = !matches!(key.code, KeyCode::Char('h') | KeyCode::Left);
            match form.row {
                MergeRow::Method => {
                    let len = form.methods.len().max(1);
                    let idx = form
                        .methods
                        .iter()
                        .position(|method| *method == form.method)
                        .unwrap_or(0);
                    let next = if forward {
                        (idx + 1) % len
                    } else {
                        (idx + len - 1) % len
                    };
                    if let Some(method) = form.methods.get(next) {
                        form.method = *method;
                    }
                }
                MergeRow::DeleteBranch => form.delete_branch = !form.delete_branch,
                MergeRow::Auto => form.auto = !form.auto,
            }
        }
        KeyCode::Enter => {
            let action = GitHubAction::Merge {
                number: form.number,
                method: form.method,
                delete_branch: form.delete_branch,
                auto: form.auto,
            };
            state.github.mode = Mode::Browse;
            dispatch(state, action);
        }
        _ => {}
    }
}

pub(super) fn confirm_close_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y' | 'Y') => {
            state.github.mode = Mode::Browse;
            if let Some(number) = selected_pr(state).map(|pr| pr.number) {
                dispatch(state, GitHubAction::Close { number });
            }
        }
        KeyCode::Char('n' | 'N') | KeyCode::Esc => state.github.mode = Mode::Browse,
        _ => {}
    }
}

pub(super) fn create_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Mode::Create(form) = &mut state.github.mode else {
        return;
    };
    match key.code {
        KeyCode::Esc => state.github.mode = Mode::Browse,
        KeyCode::Char('s') if ctrl => submit_create(state),
        KeyCode::Tab | KeyCode::Down => form.field = form.field.next(true),
        KeyCode::BackTab | KeyCode::Up => form.field = form.field.next(false),
        KeyCode::Enter => match form.field {
            CreateField::Body => form.body.insert_char('\n'),
            CreateField::Draft => form.draft = !form.draft,
            CreateField::Title | CreateField::Base => form.field = form.field.next(true),
        },
        KeyCode::Char(' ') if form.field == CreateField::Draft => form.draft = !form.draft,
        KeyCode::Char('u') if ctrl => {
            if let Some(text) = create_text(form) {
                text.clear();
            }
        }
        _ => {
            if let Some(text) = create_text(form) {
                text.handle_key(key);
            }
        }
    }
}

/// The text the focused field of the new pull request form edits.
fn create_text(form: &mut CreateForm) -> Option<&mut TextInput> {
    match form.field {
        CreateField::Title => Some(&mut form.title),
        CreateField::Base => Some(&mut form.base),
        CreateField::Body => Some(&mut form.body),
        CreateField::Draft => None,
    }
}

fn submit_create(state: &mut AppState) {
    let Mode::Create(form) = &mut state.github.mode else {
        return;
    };
    if form.title.trim().is_empty() {
        form.field = CreateField::Title;
        state.set_status("a pull request needs a title", true);
        return;
    }
    if form.base.trim().is_empty() {
        form.field = CreateField::Base;
        state.set_status("a pull request needs a base branch", true);
        return;
    }
    let pr = NewPullRequest {
        branch: form.branch.clone(),
        base: form.base.trim().to_string(),
        title: form.title.trim().to_string(),
        body: form.body.trim_end().to_string(),
        draft: form.draft,
    };
    state.github.mode = Mode::Browse;
    dispatch(state, GitHubAction::Create(pr));
}
