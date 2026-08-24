//! Keeping the running sessions fed: keys in, output out, size in step.

use ratatui::crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyEvent, KeyModifiers,
};
use ratatui::crossterm::execute;

use crate::state::AppState;
use crate::term;

use super::App;

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

/// Read every session's output and settle any that ended.
pub(crate) fn pump(state: &mut AppState) -> SessionEnded {
    state.sessions.pump();
    match state.session_view() {
        Some(id)
            if state
                .sessions
                .get(id)
                .is_some_and(|session| !session.is_running()) =>
        {
            SessionEnded(state.session_capture)
        }
        _ => SessionEnded(false),
    }
}

/// Whether the session on screen ended while it still held the keyboard.
pub(crate) struct SessionEnded(bool);

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
        self.sync_bracketed_paste();
    }

    /// Bracketed paste is only switched on while a session holds the keyboard,
    /// so lg's own text fields keep receiving pastes as ordinary typing. Called
    /// once per loop so it also covers captures started from panel keys.
    pub(super) fn sync_bracketed_paste(&mut self) {
        let wanted = self.state.session_capture;
        if wanted == self.bracketed_paste {
            return;
        }
        self.bracketed_paste = wanted;
        let out = self.terminal.backend_mut();
        let _ = if wanted {
            execute!(out, EnableBracketedPaste)
        } else {
            execute!(out, DisableBracketedPaste)
        };
    }

    /// Show the next or previous session and hand it the keyboard.
    pub(super) fn cycle_session(&mut self, forward: bool) {
        if cycle(&mut self.state, forward) {
            self.sync_bracketed_paste();
        } else {
            self.state.set_status("no sessions running", false);
        }
    }

    pub(super) fn drain_sessions(&mut self) {
        let SessionEnded(had_keyboard) = pump(&mut self.state);
        if had_keyboard {
            self.set_session_capture(false);
            self.state
                .set_status("session ended \u{2014} x closes it", false);
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
