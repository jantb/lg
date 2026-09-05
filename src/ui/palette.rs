//! The colours lg draws its chrome with, and the motion it allows itself.
//!
//! The panels used to pick from the sixteen ANSI names at each call site, which
//! left focus, selection, and idle chrome all leaning on the same grey. The
//! accent lives here instead so the focused frame, the selected row, and the
//! footer's section name are visibly one thing, and so the idle chrome can sit
//! further back than any named grey allows.
//!
//! Motion follows one rule: something moves only while something is happening.
//! A running job pulses the frame it runs in, a fresh status line settles from
//! bright to normal, a new error flashes, and a session waiting on a question
//! blinks. An idle screen holds still.

use ratatui::style::{Color, Modifier, Style};

/// The one accent: focused frame, section names, selection marker.
pub const ACCENT: Color = Color::Rgb(86, 204, 226);
/// The accent at its brightest, the peak of a pulse.
pub const ACCENT_BRIGHT: Color = Color::Rgb(170, 240, 255);
/// The accent at rest, the trough of a pulse.
pub const ACCENT_DIM: Color = Color::Rgb(44, 122, 142);

/// Border of a panel that does not have focus. Darker than `DarkGray` so the
/// focused frame stands out against it.
pub const FRAME_IDLE: Color = Color::Rgb(72, 78, 90);
/// Title text of a panel that does not have focus.
pub const TEXT_IDLE: Color = Color::Rgb(150, 156, 168);

/// The selected row's background: tinted with the accent rather than a plain
/// grey so the row reads as chosen, but deep enough that the row's own colours
/// (authors, graph pipes, branch names) survive on top of it.
pub const SELECTION_BG: Color = Color::Rgb(24, 64, 86);

/// One pulse, trough to peak and back. Indexed by the animation tick, so at
/// `ANIMATION_STEP_MS` a full pulse takes just under a second.
const PULSE: [Color; 8] = [
    ACCENT_DIM,
    ACCENT_DIM,
    ACCENT,
    ACCENT_BRIGHT,
    ACCENT_BRIGHT,
    ACCENT,
    ACCENT_DIM,
    ACCENT_DIM,
];

/// The accent shade for this tick of a pulse.
pub fn pulse(tick: usize) -> Color {
    PULSE[tick % PULSE.len()]
}

/// Whether a blink is in its "on" half. Slower than the pulse: a blink is an
/// interruption, and one that flickers is an irritation.
pub fn blink_on(tick: usize) -> bool {
    (tick / 4) % 2 == 0
}

/// How long a fresh status message stays bright before settling.
pub const STATUS_SETTLE_MS: i64 = 400;
/// How long a new error flashes before holding steady.
pub const ERROR_FLASH_MS: i64 = 2_000;
/// One half of an error flash.
const ERROR_FLASH_HALF_MS: i64 = 250;

/// How a status message of this age should look: `is_error` decides the hue,
/// the age decides how much attention it still asks for.
///
/// A success arrives bright and settles to its resting green, so the eye
/// catches the change without being held. An error flashes for a couple of
/// seconds and then holds, because it stays on screen for half a minute and
/// nobody should have to watch it blink for that long.
pub fn status_style(age_ms: i64, is_error: bool) -> Style {
    if is_error {
        let flashing = age_ms < ERROR_FLASH_MS;
        let on = !flashing || (age_ms / ERROR_FLASH_HALF_MS) % 2 == 0;
        if on {
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(140, 40, 40))
        }
    } else if age_ms < STATUS_SETTLE_MS {
        Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    }
}

/// Whether a status message this old is still animating and so still wants
/// redrawing at the animation clock's rate.
pub fn status_animating(age_ms: i64, is_error: bool) -> bool {
    if is_error {
        age_ms < ERROR_FLASH_MS
    } else {
        age_ms < STATUS_SETTLE_MS
    }
}

/// The selected row in a list. The `›` marker takes this style too, so it picks
/// up the tint along with the row.
pub fn selection() -> Style {
    Style::default()
        .bg(SELECTION_BG)
        .add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pulse_returns_to_where_it_started() {
        assert_eq!(pulse(0), pulse(PULSE.len()));
        assert!(
            (0..PULSE.len()).any(|tick| pulse(tick) != pulse(0)),
            "a pulse that never changes is not a pulse"
        );
    }

    #[test]
    fn a_fresh_error_flashes_and_an_old_one_holds() {
        let on = status_style(0, true);
        let off = status_style(ERROR_FLASH_HALF_MS, true);
        assert_ne!(on, off, "a new error must be seen to change");

        let held = status_style(ERROR_FLASH_MS, true);
        for age in (ERROR_FLASH_MS..ERROR_FLASH_MS + 3_000).step_by(100) {
            assert_eq!(status_style(age, true), held, "an old error must not blink");
        }
        assert!(!status_animating(ERROR_FLASH_MS, true));
        assert!(status_animating(0, true));
    }

    #[test]
    fn a_success_arrives_bright_and_settles() {
        assert_ne!(
            status_style(0, false),
            status_style(STATUS_SETTLE_MS, false)
        );
        assert_eq!(
            status_style(STATUS_SETTLE_MS, false),
            status_style(STATUS_SETTLE_MS + 10_000, false)
        );
        assert!(status_animating(0, false));
        assert!(!status_animating(STATUS_SETTLE_MS, false));
    }

    #[test]
    fn a_blink_has_an_off_half() {
        assert!((0..16).any(|tick| !blink_on(tick)));
        assert!((0..16).any(blink_on));
    }
}
