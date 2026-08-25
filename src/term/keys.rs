//! Turn lg's key events into the bytes a terminal program expects.
//!
//! Only what a full-screen program actually reads is encoded: control
//! characters, the escape sequences for navigation and function keys, and the
//! modifier forms of those. A key with no terminal meaning encodes to nothing
//! rather than to a stray byte.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode one key press. `application_cursor` follows the program's own mode:
/// full-screen programs usually switch the arrow keys to `ESC O A` form.
pub fn encode_key(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    let bytes = match key.code {
        KeyCode::Char(c) => {
            let mut bytes = if ctrl {
                vec![control_byte(c)?]
            } else {
                c.to_string().into_bytes()
            };
            if alt {
                bytes.insert(0, 0x1b);
            }
            bytes
        }
        // Shift+Enter and Alt+Enter are how a prompt gets a newline instead of
        // being submitted, and both are spelled as an escape before the
        // carriage return. The `CSI 13;2u` form says the same thing, but only
        // to a program that asked for the kitty keyboard protocol — and lg's
        // own terminal never answers that query, so nothing running inside it
        // ever asks. Sent there, it would read as nothing at all.
        KeyCode::Enter if shift || alt => vec![0x1b, b'\r'],
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if alt => vec![0x1b, 0x7f],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => arrow(b'A', &key.modifiers, application_cursor),
        KeyCode::Down => arrow(b'B', &key.modifiers, application_cursor),
        KeyCode::Right => arrow(b'C', &key.modifiers, application_cursor),
        KeyCode::Left => arrow(b'D', &key.modifiers, application_cursor),
        KeyCode::Home => arrow(b'H', &key.modifiers, application_cursor),
        KeyCode::End => arrow(b'F', &key.modifiers, application_cursor),
        KeyCode::Insert => tilde(2, &key.modifiers),
        KeyCode::Delete => tilde(3, &key.modifiers),
        KeyCode::PageUp => tilde(5, &key.modifiers),
        KeyCode::PageDown => tilde(6, &key.modifiers),
        KeyCode::F(n) => function_key(n, &key.modifiers)?,
        _ => return None,
    };
    Some(bytes)
}

/// Wrap pasted text so the program can tell it from typing, which is what stops
/// a pasted newline from submitting a half-pasted prompt.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }
    let mut bytes = b"\x1b[200~".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

/// The byte Ctrl+key sends: `Ctrl+a` is 1, up to `Ctrl+z` at 26, plus the
/// handful of punctuation forms that have one.
fn control_byte(c: char) -> Option<u8> {
    let byte = match c.to_ascii_lowercase() {
        'a'..='z' => (c.to_ascii_lowercase() as u8) - b'a' + 1,
        '@' | ' ' => 0,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' | '?' => 31,
        _ => return None,
    };
    Some(byte)
}

/// `1` plus a bit per modifier, which is how xterm spells them in a sequence.
fn modifier_code(modifiers: &KeyModifiers) -> u8 {
    let mut code = 1;
    if modifiers.contains(KeyModifiers::SHIFT) {
        code += 1;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        code += 2;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        code += 4;
    }
    code
}

fn arrow(final_byte: u8, modifiers: &KeyModifiers, application_cursor: bool) -> Vec<u8> {
    let code = modifier_code(modifiers);
    if code > 1 {
        // Modified arrows are always CSI form, even in application mode.
        return format!("\x1b[1;{code}{}", final_byte as char).into_bytes();
    }
    if application_cursor {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde(number: u8, modifiers: &KeyModifiers) -> Vec<u8> {
    let code = modifier_code(modifiers);
    if code > 1 {
        format!("\x1b[{number};{code}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

fn function_key(n: u8, modifiers: &KeyModifiers) -> Option<Vec<u8>> {
    let code = modifier_code(modifiers);
    // F1-F4 have their own short form; the rest are numbered.
    let bytes = match n {
        1..=4 => {
            let final_byte = b'P' + (n - 1);
            if code > 1 {
                format!("\x1b[1;{code}{}", final_byte as char).into_bytes()
            } else {
                vec![0x1b, b'O', final_byte]
            }
        }
        5..=15 => {
            let number = match n {
                5 => 15,
                6..=10 => 17 + (n - 6),
                11 => 23,
                12 => 24,
                13 => 25,
                14 => 26,
                15 => 28,
                _ => return None,
            };
            return Some(tilde(number, modifiers));
        }
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn encode(key: KeyEvent) -> Vec<u8> {
        encode_key(key, false).expect("encodable key")
    }

    #[test]
    fn plain_characters_go_through_as_utf8() {
        assert_eq!(encode(plain(KeyCode::Char('a'))), b"a");
        assert_eq!(encode(plain(KeyCode::Char('\u{e5}'))), "å".as_bytes());
    }

    #[test]
    fn control_keys_become_control_bytes() {
        assert_eq!(encode(with(KeyCode::Char('c'), KeyModifiers::CONTROL)), [3]);
        assert_eq!(encode(with(KeyCode::Char('a'), KeyModifiers::CONTROL)), [1]);
        assert_eq!(encode(with(KeyCode::Char(' '), KeyModifiers::CONTROL)), [0]);
        assert_eq!(
            encode(with(KeyCode::Char(']'), KeyModifiers::CONTROL)),
            [29]
        );
    }

    #[test]
    fn alt_prefixes_an_escape() {
        assert_eq!(
            encode(with(KeyCode::Char('b'), KeyModifiers::ALT)),
            b"\x1bb"
        );
        assert_eq!(encode(with(KeyCode::Enter, KeyModifiers::ALT)), b"\x1b\r");
    }

    #[test]
    fn the_basics_match_what_a_terminal_sends() {
        assert_eq!(encode(plain(KeyCode::Enter)), b"\r");
        assert_eq!(encode(plain(KeyCode::Tab)), b"\t");
        assert_eq!(encode(plain(KeyCode::BackTab)), b"\x1b[Z");
        assert_eq!(encode(plain(KeyCode::Backspace)), [0x7f]);
        assert_eq!(encode(plain(KeyCode::Esc)), [0x1b]);
    }

    #[test]
    fn shift_enter_asks_for_a_newline_rather_than_submitting() {
        assert_eq!(encode(with(KeyCode::Enter, KeyModifiers::SHIFT)), b"\x1b\r");
        assert_eq!(encode(plain(KeyCode::Enter)), b"\r", "plain Enter submits");
    }

    #[test]
    fn arrows_follow_the_programs_cursor_mode() {
        assert_eq!(encode(plain(KeyCode::Up)), b"\x1b[A");
        assert_eq!(
            encode_key(plain(KeyCode::Up), true).unwrap(),
            b"\x1bOA",
            "application cursor mode uses the O form"
        );
    }

    #[test]
    fn modified_arrows_stay_in_csi_form_even_in_application_mode() {
        let key = with(KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(encode_key(key, true).unwrap(), b"\x1b[1;5D");
        let key = with(KeyCode::Right, KeyModifiers::SHIFT | KeyModifiers::ALT);
        assert_eq!(encode(key), b"\x1b[1;4C");
    }

    #[test]
    fn navigation_and_function_keys_are_numbered_sequences() {
        assert_eq!(encode(plain(KeyCode::Delete)), b"\x1b[3~");
        assert_eq!(encode(plain(KeyCode::PageDown)), b"\x1b[6~");
        assert_eq!(encode(plain(KeyCode::F(1))), b"\x1bOP");
        assert_eq!(encode(plain(KeyCode::F(5))), b"\x1b[15~");
        assert_eq!(encode(plain(KeyCode::F(12))), b"\x1b[24~");
        assert_eq!(
            encode(with(KeyCode::F(5), KeyModifiers::CONTROL)),
            b"\x1b[15;5~"
        );
    }

    #[test]
    fn keys_with_no_terminal_meaning_encode_to_nothing() {
        assert!(encode_key(plain(KeyCode::CapsLock), false).is_none());
        assert!(encode_key(plain(KeyCode::F(30)), false).is_none());
    }

    #[test]
    fn pasted_text_is_bracketed_when_the_program_asked_for_it() {
        assert_eq!(encode_paste("a\nb", false), b"a\nb");
        assert_eq!(encode_paste("a\nb", true), b"\x1b[200~a\nb\x1b[201~");
    }
}
