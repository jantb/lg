//! Keys and the mouse in the flow modal.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Constraint, Rect};

use crate::state::{AppState, Modal, clamp_index};
use crate::{app, ui};

use super::available_actions;
use super::draw::{branch_name_char, confirm_detail, menu_areas, modal_area};

/// The wheel moves through the actions and a click selects one; a click on
/// the one already selected goes on with it, as Enter does — to its prompt or
/// its confirmation, never straight to running it.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> Result<()> {
    if state.workflow_job.is_some()
        || state.flow_confirm.is_some()
        || state.flow_input.is_some()
        || !state.branch_actions_available()
    {
        return Ok(());
    }
    if let Some(down) = super::super::pointer::wheel(m) {
        return handle_key(
            state,
            KeyEvent::from(if down { KeyCode::Down } else { KeyCode::Up }),
        );
    }
    if !super::super::pointer::left_click(m) {
        return Ok(());
    }
    let inner = ui::modal_inner(modal_area(area));
    let (chunks, _) = ui::modal_row_areas(inner, &[Constraint::Length(1), Constraint::Min(0)]);
    let (list, _) = menu_areas(chunks[1]);
    let len = available_actions(state).len();
    if let Some(at) =
        super::super::pointer::list_row(list, state.flow_list.scroll, len, m.column, m.row)
    {
        if Some(at) == clamp_index(state.flow_list.idx, len) {
            return handle_key(state, KeyEvent::from(KeyCode::Enter));
        }
        state.flow_list.idx = at;
    }
    Ok(())
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    if state.workflow_job.is_some() {
        return Ok(());
    }

    if !state.branch_actions_available() {
        state.flow_confirm = None;
        state.flow_input = None;
        state.flow_text.clear();
        if key.code == KeyCode::Esc {
            state.modal = Modal::None;
        }
        return Ok(());
    }

    if let Some(action) = state.flow_input {
        match key.code {
            KeyCode::Esc => {
                state.flow_input = None;
                state.flow_text.clear();
            }
            KeyCode::Enter => {
                let name = state.flow_text.trim().to_owned();
                if name.is_empty() {
                    state.set_status("branch name cannot be empty", true);
                } else {
                    state.flow_input = None;
                    state.flow_text.clear();
                    app::run_flow_action(state, action, Some(name));
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.flow_text.insert_char(branch_name_char(c));
            }
            _ => {
                state.flow_text.handle_key(key);
            }
        }
        return Ok(());
    }

    if let Some(action) = state.flow_confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                state.flow_confirm = None;
                app::run_flow_action(state, action, None);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.flow_confirm = None;
            }
            _ => {}
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let actions = available_actions(state);
            state.flow_list.idx = clamp_index(state.flow_list.idx, actions.len()).unwrap_or(0);
            state.flow_list.idx = state
                .flow_list
                .idx
                .saturating_add(1)
                .min(actions.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let actions = available_actions(state);
            state.flow_list.idx = clamp_index(state.flow_list.idx, actions.len()).unwrap_or(0);
            state.flow_list.idx = state.flow_list.idx.saturating_sub(1);
        }
        KeyCode::Enter => {
            let actions = available_actions(state);
            let Some(action) = actions
                .get(state.flow_list.idx.min(actions.len().saturating_sub(1)))
                .copied()
            else {
                return Ok(());
            };
            if action.needs_input() {
                state.flow_input = Some(action);
                state.flow_text.clear();
            } else if action.needs_confirmation() {
                state.flow_confirm_detail = confirm_detail(state, action);
                state.flow_confirm = Some(action);
            } else {
                app::run_flow_action(state, action, None);
            }
        }
        _ => {}
    }
    Ok(())
}
