//! Code raining down the sky.

use super::*;

/// The glyphs raining down the background.
pub(super) const RAIN_GLYPHS: &[char] = &[
    '{', '}', '(', ')', '[', ']', ';', '=', '<', '>', '+', '-', '*', '/', '&', '|', '!', '?', ':',
    '.', '0', '1', 'f', 'n', 'x',
];
/// How many columns out of this many carry rain.
const RAIN_DENSITY: u64 = 3;
const RAIN_TRAIL: usize = 7;
/// Rows a drop falls per second, at the slower of its two speeds.
const RAIN_SPEED: f32 = 9.0;

/// Code falling down the background above the ground: a third of the
/// columns carry a drop, each at its own offset, a bright head with a
/// fading tail of glyphs behind it.
pub(super) fn draw_rain(canvas: &mut Canvas, width: usize, ground: usize, t: f32) {
    if ground == 0 {
        return;
    }
    let span = ground + RAIN_TRAIL;
    for x in 0..width {
        let seed = hash(x, 7);
        if !seed.is_multiple_of(RAIN_DENSITY) {
            continue;
        }
        // Some drops fall at one speed, some at half again, so the rain has
        // depth; the head's position is continuous, its cell the floor of it.
        let speed = RAIN_SPEED * (1.0 + 0.5 * ((seed >> 8) % 2) as f32);
        let fall = t * speed + ((seed >> 16) % 1_000) as f32;
        let head = fall as usize % span;
        let generation = fall as usize / span;
        for back in 0..RAIN_TRAIL {
            let Some(y) = head.checked_sub(back) else {
                break;
            };
            if y >= ground {
                continue;
            }
            let glyph = RAIN_GLYPHS[(hash(x, y + generation) % RAIN_GLYPHS.len() as u64) as usize];
            // Brightness falls off along the tail and flickers a little.
            let fade = 1.0 - back as f32 / RAIN_TRAIL as f32;
            let flicker = 0.85 + 0.15 * (t * 17.0 + x as f32).sin();
            let color = if back == 0 {
                Color::Rgb(170, 255, 200)
            } else {
                dim(Color::Rgb(70, 200, 120), fade * flicker)
            };
            let mut style = Style::default().fg(color);
            if back == 0 {
                style = style.add_modifier(Modifier::BOLD);
            }
            canvas.put(x, y, glyph, style);
        }
    }
}
