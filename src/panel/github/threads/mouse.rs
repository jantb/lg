//! The mouse over a pull request's conversations: the wheel moves through the
//! list or scrolls whatever it is over, a click selects a conversation, gives
//! the chat the keyboard or takes it back, and a tab in the header leaves the
//! conversations for that tab.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::state::AppState;

use super::super::Mode;
use super::super::browse::show_tab;
use super::super::draw::{self, HeaderClick};
use super::chat::{self, set_focus};
use super::current;

/// How far a turn of the wheel scrolls a conversation or the chat.
const WHEEL_LINES: u16 = 3;

pub(in crate::panel::github) fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) {
    let session = chat::session(state);
    let Some(panes) = draw::thread_panes(area, session.is_some()) else {
        return;
    };
    let at = Position::new(m.column, m.row);
    let in_chat = panes.chat.is_some_and(|chat| chat.contains(at));
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(HeaderClick::Tab(tab)) = draw::header_click(state, area, m.column, m.row) {
                leave_for(state, tab);
                return;
            }
            set_focus(state, in_chat);
            if in_chat
                && let Some(screen) = panes.chat.map(chat_screen)
                && screen.contains(at)
            {
                // A drag from here selects the chat's own text, and no further.
                state.selection = crate::ui::TextSelection::within(screen, m.column, m.row);
            } else if panes.conversation.contains(at) {
                state.selection =
                    crate::ui::TextSelection::within(panes.conversation, m.column, m.row);
            }
            if panes.list.contains(at) {
                click_list(state, panes.list, m.row);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(selection) = state.selection.as_mut().filter(|s| s.dragging) {
                selection.extend(m.column, m.row);
            }
        }
        // A drag let go is copied; a plain click selects nothing.
        MouseEventKind::Up(MouseButton::Left) => {
            state.selection = state
                .selection
                .take()
                .and_then(crate::ui::TextSelection::release);
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            state.selection = None;
            let down = m.kind == MouseEventKind::ScrollDown;
            if in_chat {
                let screen = panes.chat.map(chat_screen).unwrap_or_default();
                if let Some(session) = session.and_then(|id| state.sessions.get_mut(id)) {
                    // Claude takes the wheel itself when it asked for the mouse.
                    let (column, row) = (
                        m.column.saturating_sub(screen.x),
                        m.row.saturating_sub(screen.y),
                    );
                    if !session.send_wheel(!down, column, row) {
                        session.scroll(!down, WHEEL_LINES as usize);
                    }
                }
                return;
            }
            let Some(view) = current(state) else {
                return;
            };
            if panes.list.contains(at) {
                let idx = if down {
                    view.idx + 1
                } else {
                    view.idx.saturating_sub(1)
                };
                view.select(idx);
            } else if panes.conversation.contains(at) {
                view.scroll = if down {
                    view.scroll.saturating_add(WHEEL_LINES)
                } else {
                    view.scroll.saturating_sub(WHEEL_LINES)
                };
            }
        }
        _ => {}
    }
}

/// The chat's terminal: its pane below the title row.
fn chat_screen(pane: Rect) -> Rect {
    Rect {
        y: pane.y + 1,
        height: pane.height.saturating_sub(1),
        ..pane
    }
}

/// Select the conversation drawn on `row`, scrolled the way the list draws it.
fn click_list(state: &mut AppState, list: Rect, row: u16) {
    let Some(view) = current(state) else {
        return;
    };
    let shown = list.height as usize;
    let offset = view.idx.saturating_sub(shown.saturating_sub(1));
    let idx = offset + (row - list.y) as usize;
    if idx < view.visible().len() && idx != view.idx {
        view.select(idx);
    }
}

/// A reply half written is left behind with the conversations; the chat
/// carries on and is there on coming back.
fn leave_for(state: &mut AppState, tab: super::super::Tab) {
    set_focus(state, false);
    state.github.mode = Mode::Browse;
    show_tab(state, tab);
}
