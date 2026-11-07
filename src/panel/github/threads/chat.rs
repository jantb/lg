//! The Claude chat beside a pull request's conversations: each one is handed
//! to Claude, which reads the code it is on and says whether the reviewer's
//! point holds, and the two talk it through until they agree on what to do
//! before it is answered and resolved.
//!
//! It is the pull request's code Claude reads, so the chat opens only where
//! the pull request's branch is checked out. It outlives leaving the
//! conversations: going back to them finds it where it was.

use ratatui::crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::github::ReviewThread;
use crate::session::SessionId;
use crate::state::{AppState, Modal, PendingAction};

use super::super::Mode;
use super::current;

/// The chat started from a pull request's conversations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadsChat {
    /// The pull request whose conversations it is about.
    pub number: u64,
    pub id: SessionId,
    /// The conversation it was handed last, by id, and where that one is.
    pub thread: String,
    pub location: String,
    /// It has the keyboard: every key but Ctrl-] goes to Claude.
    pub focused: bool,
}

/// What Claude is told about one conversation.
pub fn brief(thread: &ReviewThread) -> String {
    let mut out = format!("Conversation on {}", thread.location());
    if thread.resolved {
        out.push_str(" (resolved)");
    }
    if thread.outdated {
        out.push_str(" (outdated: the code it was left on has changed since)");
    }
    out.push_str(":\n");
    if !thread.diff_hunk.trim().is_empty() {
        out.push_str("Diff it was left on:\n");
        out.push_str(thread.diff_hunk.trim_end());
        out.push('\n');
    }
    for comment in &thread.comments {
        out.push_str(&format!(
            "\n{} wrote:\n{}\n",
            comment.author,
            comment.body.trim_end()
        ));
    }
    out
}

/// The chat for the conversations on screen, while its session runs.
pub fn session(state: &AppState) -> Option<SessionId> {
    let Mode::Threads(view) = &state.github.mode else {
        return None;
    };
    let chat = state.github.threads_chat.as_ref()?;
    (chat.number == view.number && state.sessions.get(chat.id).is_some()).then_some(chat.id)
}

pub fn chat_on_screen(state: &AppState) -> Option<SessionId> {
    (state.modal == Modal::GitHub)
        .then(|| session(state))
        .flatten()
}

/// Then every key, Ctrl-C and Esc included, goes to Claude.
pub fn chat_has_keyboard(state: &AppState) -> bool {
    chat_on_screen(state).is_some()
        && state
            .github
            .threads_chat
            .as_ref()
            .is_some_and(|chat| chat.focused)
}

/// Keep the session as big as the pane it is drawn in.
pub fn fit_chat(state: &mut AppState, area: Rect) {
    let Some(id) = chat_on_screen(state) else {
        return;
    };
    let Some(screen) = super::super::draw::thread_chat_screen(area) else {
        return;
    };
    if let Some(session) = state.sessions.get_mut(id) {
        session.resize(screen.height, screen.width);
    }
}

fn set_focus(state: &mut AppState, focused: bool) {
    if let Some(chat) = state.github.threads_chat.as_mut() {
        chat.focused = focused;
    }
}

/// `C`: hand Claude the selected conversation, starting the chat first if
/// there is none, and give it the keyboard. A conversation it already has is
/// not handed again; the keyboard just goes back to it.
pub(super) fn ask(state: &mut AppState) {
    let Some(view) = current(state) else {
        return;
    };
    let Some(thread) = view.selected() else {
        state.set_status("no conversation selected", false);
        return;
    };
    let (number, head) = (view.number, view.head.clone());
    let (id, location, text) = (thread.id.clone(), thread.location(), brief(thread));
    if state.branch.as_deref() != Some(head.as_str()) {
        state.set_status(
            format!(
                "#{number} is not checked out here, so Claude cannot read its code \u{2014} Esc, then Enter on it checks out {head}"
            ),
            false,
        );
        return;
    }
    let Some(session_id) = session(state) else {
        state.pending_action = Some(PendingAction::ThreadsChat);
        return;
    };
    let handed = state
        .github
        .threads_chat
        .as_ref()
        .is_some_and(|chat| chat.thread == id);
    if !handed {
        if let Some(session) = state.sessions.get_mut(session_id) {
            let message = crate::llm::build_threads_chat_next(&text);
            let bracketed = session.bracketed_paste();
            session.send(&crate::term::encode_paste(&message, bracketed));
            session.send(b"\r");
        }
        if let Some(chat) = state.github.threads_chat.as_mut() {
            chat.thread = id;
            chat.location = location.clone();
        }
    }
    set_focus(state, true);
    let said = if handed {
        "in the chat"
    } else {
        "Claude is reading the code"
    };
    state.set_status(
        format!("{said} on {location} \u{2014} Ctrl-] back to the conversations"),
        false,
    );
}

/// A key while the chat has the keyboard.
pub(super) fn key(state: &mut AppState, key: KeyEvent) {
    if crate::app::session::is_release_key(&key) {
        set_focus(state, false);
        return;
    }
    match session(state).and_then(|id| state.sessions.get_mut(id)) {
        Some(session) => {
            if let Some(bytes) = crate::term::encode_key(key, session.application_cursor()) {
                session.send(&bytes);
            }
        }
        None => set_focus(state, false),
    }
}

/// Text pasted while the chat has the keyboard goes to Claude.
pub(super) fn paste(state: &mut AppState, text: &str) -> bool {
    if !chat_has_keyboard(state) {
        return false;
    }
    if let Some(session) = session(state).and_then(|id| state.sessions.get_mut(id)) {
        let bracketed = session.bracketed_paste();
        session.send(&crate::term::encode_paste(text, bracketed));
    }
    true
}

/// A click in the chat gives it the keyboard, one elsewhere takes it back; the
/// wheel over it scrolls what Claude has said.
pub(in crate::panel::github) fn mouse(state: &mut AppState, area: Rect, m: &MouseEvent) {
    let Some(id) = session(state) else {
        return;
    };
    let Some(screen) = super::super::draw::thread_chat_area(area) else {
        return;
    };
    let inside = screen.contains(Position::new(m.column, m.row));
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => set_focus(state, inside),
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if inside => {
            if let Some(session) = state.sessions.get_mut(id) {
                session.scroll(m.kind == MouseEventKind::ScrollUp, 3);
            }
        }
        _ => {}
    }
}
