//! Telling a program about the wheel.
//!
//! A program that asks to be told about the mouse expects to handle the wheel
//! itself, and one drawing on the alternate screen has no scrollback for lg to
//! move on its behalf — vt100 gives the alternate grid none at all. So the
//! notch is encoded the way xterm would and handed over, which is what a
//! terminal emulator does with it.

use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// Wheel up and wheel down as xterm numbers them: buttons 4 and 5 with the
/// 64 bit set, reported as a press with no matching release.
const WHEEL_UP: u8 = 64;
const WHEEL_DOWN: u8 = 65;

/// The bytes for one wheel notch at a cell, or `None` when the program has not
/// asked about the mouse and the wheel is lg's to act on.
///
/// `column` and `row` are 1-based and relative to the program's own screen.
pub fn encode_wheel(
    up: bool,
    column: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }
    let button = if up { WHEEL_UP } else { WHEEL_DOWN };
    let column = column.max(1);
    let row = row.max(1);
    Some(match encoding {
        MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{column};{row}M").into_bytes(),
        // The older encodings carry each coordinate in one byte, so anything
        // past the 223rd cell cannot be expressed and is reported at the edge.
        _ => vec![
            0x1b,
            b'[',
            b'M',
            32u8.saturating_add(button),
            offset_byte(column),
            offset_byte(row),
        ],
    })
}

fn offset_byte(value: u16) -> u8 {
    u8::try_from(value.saturating_add(32)).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_never_asked_gets_nothing() {
        assert!(
            encode_wheel(
                true,
                1,
                1,
                MouseProtocolMode::None,
                MouseProtocolEncoding::Sgr
            )
            .is_none(),
            "the wheel stays lg's to act on"
        );
    }

    #[test]
    fn sgr_reports_the_wheel_as_a_press_at_the_cell() {
        let up = encode_wheel(
            true,
            12,
            root_row(),
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        )
        .expect("encoded");
        assert_eq!(String::from_utf8(up).unwrap(), "\x1b[<64;12;5M");

        let down = encode_wheel(
            false,
            12,
            root_row(),
            MouseProtocolMode::AnyMotion,
            MouseProtocolEncoding::Sgr,
        )
        .expect("encoded");
        assert_eq!(String::from_utf8(down).unwrap(), "\x1b[<65;12;5M");
    }

    fn root_row() -> u16 {
        5
    }

    #[test]
    fn the_default_encoding_offsets_every_field_by_32() {
        let bytes = encode_wheel(
            true,
            1,
            1,
            MouseProtocolMode::Press,
            MouseProtocolEncoding::Default,
        )
        .expect("encoded");
        assert_eq!(bytes, vec![0x1b, b'[', b'M', 96, 33, 33]);
    }

    #[test]
    fn a_cell_past_what_one_byte_holds_is_reported_at_the_edge() {
        let bytes = encode_wheel(
            false,
            400,
            1,
            MouseProtocolMode::Press,
            MouseProtocolEncoding::Default,
        )
        .expect("encoded");
        assert_eq!(bytes[3], 97, "wheel down");
        assert_eq!(bytes[4], u8::MAX, "clamped rather than wrapped");
    }

    #[test]
    fn a_cell_at_the_origin_is_never_reported_as_zero() {
        let bytes = encode_wheel(
            true,
            0,
            0,
            MouseProtocolMode::Press,
            MouseProtocolEncoding::Default,
        )
        .expect("encoded");
        assert_eq!((bytes[4], bytes[5]), (33, 33), "1-based, not 0-based");
    }
}
