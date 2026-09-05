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

/// How long one pulse takes, trough to peak and back.
pub const PULSE_PERIOD_MS: u64 = 1_000;

/// A colour part of the way from `from` to `to`; `t` runs from 0 to 1.
fn lerp(from: (u8, u8, u8), to: (u8, u8, u8), t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
    Color::Rgb(mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}

/// Where in a cycle of `period_ms` the clock is, as 0 at the trough rising
/// smoothly to 1 at the peak and back. Cosine rather than a triangle wave, so
/// the turn at either end is soft.
fn wave(clock_ms: u64, period_ms: u64) -> f64 {
    let phase = (clock_ms % period_ms) as f64 / period_ms as f64;
    0.5 - 0.5 * (phase * std::f64::consts::TAU).cos()
}

const ACCENT_DIM_RGB: (u8, u8, u8) = (44, 122, 142);
const ACCENT_BRIGHT_RGB: (u8, u8, u8) = (170, 240, 255);

/// A colour breathing between `dim` and `bright` once every `period_ms`, read
/// at this point of the animation clock. Continuous in time, so it is as
/// smooth as the frame rate allows.
pub fn breathe(dim: (u8, u8, u8), bright: (u8, u8, u8), clock_ms: u64, period_ms: u64) -> Color {
    lerp(dim, bright, wave(clock_ms, period_ms))
}

/// How bright a glowing thing is drawn at `intensity`, 0 to 1: the resting
/// accent at nothing, through the bright accent, to white at full.
pub fn glow(intensity: f64) -> Color {
    let intensity = intensity.clamp(0.0, 1.0);
    if intensity < 0.5 {
        lerp(ACCENT_DIM_RGB, ACCENT_BRIGHT_RGB, intensity * 2.0)
    } else {
        lerp(ACCENT_BRIGHT_RGB, (255, 255, 255), (intensity - 0.5) * 2.0)
    }
}

/// The accent shade at this point of the animation clock, in milliseconds.
pub fn pulse(clock_ms: u64) -> Color {
    breathe(ACCENT_DIM_RGB, ACCENT_BRIGHT_RGB, clock_ms, PULSE_PERIOD_MS)
}

/// Whether a blink is in its "on" half. Slower than the pulse: a blink is an
/// interruption, and one that flickers is an irritation.
pub fn blink_on(tick: usize) -> bool {
    (tick / 4) % 2 == 0
}

/// How long a fresh status message stays bright before settling.
pub const STATUS_SETTLE_MS: i64 = 600;
/// How long a new error flashes before holding steady.
pub const ERROR_FLASH_MS: i64 = 2_000;
/// One half of an error flash.
const ERROR_FLASH_HALF_MS: i64 = 250;

const SUCCESS_FRESH_RGB: (u8, u8, u8) = (190, 255, 190);
const SUCCESS_REST_RGB: (u8, u8, u8) = (90, 200, 110);
const ERROR_BRIGHT_RGB: (u8, u8, u8) = (255, 95, 95);
const ERROR_DIM_RGB: (u8, u8, u8) = (140, 40, 40);

/// How a status message of this age should look: `is_error` decides the hue,
/// the age decides how much attention it still asks for.
///
/// A success arrives bright and fades to its resting green, so the eye catches
/// the change without being held. An error throbs between bright and dim for a
/// couple of seconds and then holds, because it stays on screen for half a
/// minute and nobody should have to watch it blink for that long. Both are
/// continuous in time rather than stepped, so they look smooth at any frame
/// rate.
pub fn status_style(age_ms: i64, is_error: bool) -> Style {
    let age_ms = age_ms.max(0) as u64;
    if is_error {
        let color = if age_ms < ERROR_FLASH_MS as u64 {
            // Start bright: the wave is at its trough at zero, so the age is
            // offset by half a flash to begin at the peak.
            let period = 2 * ERROR_FLASH_HALF_MS as u64;
            lerp(
                ERROR_DIM_RGB,
                ERROR_BRIGHT_RGB,
                wave(age_ms + period / 2, period),
            )
        } else {
            let (r, g, b) = ERROR_BRIGHT_RGB;
            Color::Rgb(r, g, b)
        };
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        let t = age_ms as f64 / STATUS_SETTLE_MS as f64;
        let style = Style::default().fg(lerp(SUCCESS_FRESH_RGB, SUCCESS_REST_RGB, t));
        if age_ms < STATUS_SETTLE_MS as u64 {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
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
        assert_eq!(pulse(0), pulse(PULSE_PERIOD_MS));
        assert!(
            (0..PULSE_PERIOD_MS).any(|ms| pulse(ms) != pulse(0)),
            "a pulse that never changes is not a pulse"
        );
    }

    #[test]
    fn a_pulse_moves_in_small_steps() {
        // Drawn every few milliseconds, neighbouring frames must differ by at
        // most a shade or two, or the fade reads as a flicker.
        let channels = |c: Color| match c {
            Color::Rgb(r, g, b) => [r, g, b],
            other => panic!("pulse must be a true colour, got {other:?}"),
        };
        for ms in (0..PULSE_PERIOD_MS).step_by(8) {
            let a = channels(pulse(ms));
            let b = channels(pulse(ms + 8));
            for (x, y) in a.iter().zip(b.iter()) {
                assert!(x.abs_diff(*y) <= 4, "jump of {} at {ms}ms", x.abs_diff(*y));
            }
        }
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
