//! The mouse over the modal: tabs and filters in the header, rows in the
//! lists.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::state::AppState;

use super::browse::{next_filter, select_pr, show_filter, show_tab, toggle_drafts};
use super::lookup::visible_repos;
use super::{Mode, Tab, draw, inbox};

/// The wheel moves through whichever list is on screen; a click on the header
/// picks a tab, steps the filter or shows and hides drafts, and a click in the
/// list selects what it lands on.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) {
    if state.github.mode != Mode::Browse {
        return;
    }
    let down = match m.kind {
        MouseEventKind::ScrollDown => true,
        MouseEventKind::ScrollUp => false,
        MouseEventKind::Down(MouseButton::Left) => {
            match draw::header_click(state, area, m.column, m.row) {
                Some(draw::HeaderClick::Tab(tab)) => show_tab(state, tab),
                Some(draw::HeaderClick::Filter(filter)) => show_filter(state, filter),
                Some(draw::HeaderClick::NextFilter) => next_filter(state),
                Some(draw::HeaderClick::Drafts) => toggle_drafts(state),
                None => {
                    if let Some(idx) = draw::list_click(state, area, m.column, m.row) {
                        match state.github.tab {
                            Tab::Inbox => inbox::select(state, idx),
                            Tab::PullRequests => select_pr(state, idx),
                            Tab::Repositories => state.github.repo_idx = idx,
                        }
                    }
                }
            }
            return;
        }
        _ => return,
    };
    match state.github.tab {
        Tab::Inbox if down => inbox::select(state, state.github.inbox.idx + 1),
        Tab::Inbox => inbox::select(state, state.github.inbox.idx.saturating_sub(1)),
        Tab::PullRequests if down => select_pr(state, state.github.pr_idx + 1),
        Tab::PullRequests => select_pr(state, state.github.pr_idx.saturating_sub(1)),
        Tab::Repositories => {
            let last = visible_repos(&state.github).len().saturating_sub(1);
            state.github.repo_idx = if down {
                (state.github.repo_idx + 1).min(last)
            } else {
                state.github.repo_idx.saturating_sub(1)
            };
        }
    }
}
