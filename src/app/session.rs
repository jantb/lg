//! Keeping the running sessions fed: keys in, output out, size in step.

use ratatui::crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyEvent, KeyModifiers,
};
use ratatui::crossterm::execute;

use ratatui::backend::Backend;

use crate::state::{AppState, Modal};
use crate::term;

use super::{App, HeadlessApp};

/// The key that hands the keyboard back to lg. Esc has to reach the program
/// being run, so it cannot double as the way out.
///
/// A terminal sends Ctrl-] as the single byte 0x1D, and with no keyboard
/// enhancement negotiated crossterm decodes 0x1C..=0x1F as Ctrl and a digit —
/// so that byte arrives as Ctrl-5, not Ctrl-]. They are one keypress the
/// terminal cannot tell apart, so both have to release; matching only on ']'
/// left the keyboard stuck inside the session with no way out but the mouse.
pub(super) fn is_release_key(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(']') | KeyCode::Char('5'))
}

/// Send a key to the focused session. Returns whether it was consumed —
/// including keys with no terminal meaning, which are swallowed rather than
/// left to trigger an lg action behind the program's back.
pub(crate) fn forward_key(state: &mut AppState, key: KeyEvent) -> bool {
    let Some(id) = state.session_view() else {
        return false;
    };
    let Some(session) = state.sessions.get_mut(id) else {
        return false;
    };
    let application_cursor = session.application_cursor();
    if let Some(bytes) = term::encode_key(key, application_cursor) {
        session.send(&bytes);
    }
    true
}

/// Send pasted text to the focused session, marked as a paste when the program
/// asked for that, so a pasted newline does not submit half a prompt.
pub(crate) fn forward_paste(state: &mut AppState, text: &str) -> bool {
    let Some(id) = state.session_view() else {
        return false;
    };
    let Some(session) = state.sessions.get_mut(id) else {
        return false;
    };
    let bracketed = session.bracketed_paste();
    session.send(&term::encode_paste(text, bracketed));
    true
}

/// Read every session's output and settle any that ended. A session whose
/// program has stopped is dropped by the pump itself, so what is left to do
/// here is clear up after it: the pane goes back to the diff if it was the one
/// on screen, and the tree loses a row.
///
/// Closing the session that was on screen while a flow is still stopped on a
/// conflict puts the conflict back up. A session started from that modal is
/// the work of resolving it, so its ending is the moment to ask again whether
/// the flow can carry on — otherwise the flow is left waiting behind a diff
/// with nothing to say it is there.
///
/// Returns what to say about it, and whether the keyboard has to be taken back
/// off a session that is no longer there.
pub(crate) fn pump(state: &mut AppState) -> Option<SessionEnded> {
    let shown = state.session_view();
    let ended = state.sessions.pump();
    let gone = ended
        .iter()
        .find(|session| Some(session.id) == shown)
        .or_else(|| ended.last())?;
    let was_shown = Some(gone.id) == shown;
    let had_keyboard = was_shown && state.session_capture;
    let mut notice = format!("the session in {} {}", gone.label, gone.notice);
    if was_shown {
        state.show_diff();
        if !state.conflicts.is_empty() {
            state.modal = Modal::Conflict;
            notice.push_str(" \u{2014} press v to continue the flow");
        }
    }
    state.clamp();
    Some(SessionEnded {
        notice,
        had_keyboard,
    })
}

/// A session that ended, ready to be reported.
pub(crate) struct SessionEnded {
    /// What to put in the status bar.
    notice: String,
    /// Whether it held the keyboard when it went.
    had_keyboard: bool,
}

/// Move to the next or previous session, wrapping round.
pub(crate) fn cycle(state: &mut AppState, forward: bool) -> bool {
    let Some(id) = state.sessions.neighbour(forward) else {
        return false;
    };
    state.show_session(id);
    state.session_capture = true;
    true
}

impl App {
    /// Point the keyboard at the focused session, or back at lg.
    pub(super) fn set_session_capture(&mut self, on: bool) {
        self.state.session_capture = on;
        self.sync_session_keyboard();
    }

    /// The terminal is only put in session mode while a session holds the
    /// keyboard, so lg's own text fields keep receiving pastes as ordinary
    /// typing and its panels keep seeing the keys they were written against.
    /// Called once per loop so it also covers captures started from panel keys.
    ///
    /// Session mode is bracketed paste plus spelled-out keys: without the
    /// second, the terminal sends the same bare `\r` for Enter and Shift+Enter,
    /// and a prompt can only ever be submitted, never given a newline.
    pub(super) fn sync_session_keyboard(&mut self) {
        let wanted = self.state.session_capture;
        if wanted == self.session_keyboard {
            return;
        }
        self.session_keyboard = wanted;
        let disambiguate = self.keys_can_disambiguate;
        let out = self.terminal.backend_mut();
        let _ = if wanted {
            execute!(out, EnableBracketedPaste)
        } else {
            execute!(out, DisableBracketedPaste)
        };
        if disambiguate {
            super::set_key_disambiguation(out, wanted);
        }
    }

    /// Show the next or previous session and hand it the keyboard.
    pub(super) fn cycle_session(&mut self, forward: bool) {
        if cycle(&mut self.state, forward) {
            self.sync_session_keyboard();
        } else {
            self.state.set_status("no sessions running", false);
        }
    }

    pub(super) fn drain_sessions(&mut self) {
        let Some(ended) = pump(&mut self.state) else {
            return;
        };
        if ended.had_keyboard {
            self.set_session_capture(false);
        }
        self.state.set_status(ended.notice, false);
    }
}

impl<B: Backend> HeadlessApp<B> {
    /// What the real loop does to the sessions each frame, without a terminal
    /// to keep in step. This is the seam tests drive.
    pub fn drain_sessions(&mut self) {
        if let Some(ended) = pump(&mut self.state) {
            self.state.set_status(ended.notice, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_key_is_recognised_the_way_a_terminal_sends_it() {
        // 0x1D - 0x1C + b'4' == b'5': what crossterm makes of Ctrl-]'s byte.
        assert_eq!((0x1Du8 - 0x1C + b'4') as char, '5');
        for code in [KeyCode::Char(']'), KeyCode::Char('5')] {
            assert!(
                is_release_key(&KeyEvent::new(code, KeyModifiers::CONTROL)),
                "{code:?} is Ctrl-] as some terminal spells it"
            );
        }
    }

    #[test]
    fn the_release_key_needs_control_held() {
        for code in [KeyCode::Char(']'), KeyCode::Char('5')] {
            assert!(
                !is_release_key(&KeyEvent::new(code, KeyModifiers::NONE)),
                "{code:?} on its own belongs to the program"
            );
        }
    }
}
