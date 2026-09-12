//! The backdrops behind the mascot: rain, night sky, moon and the scrolling diff.

use super::*;

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

/// A night sky: stars that brighten and fade at their own pace, the moon
/// itself, and now and then a shooting star crossing from the top left.
pub(super) fn draw_night(canvas: &mut Canvas, width: usize, ground: usize, ms: u64) {
    let t = ms as f32 / 1000.0;
    for y in 0..ground {
        for x in 0..width {
            let seed = hash(x, y);
            if !seed.is_multiple_of(STAR_DENSITY) {
                continue;
            }
            // Each star twinkles on its own period and phase; the glyph
            // follows the brightness so a star swells as it brightens.
            let period = 1.5 + ((seed >> 8) % 100) as f32 / 40.0;
            let phase = ((seed >> 16) % 628) as f32 / 100.0;
            let bright = 0.5 + 0.5 * (t * std::f32::consts::TAU / period + phase).sin();
            let glyph = match bright {
                b if b > 0.9 => '\u{2726}',
                b if b > 0.7 => '*',
                b if b > 0.4 => '+',
                _ => '\u{b7}',
            };
            let color = mix(Color::Rgb(90, 90, 140), Color::Rgb(255, 255, 225), bright);
            canvas.put(x, y, glyph, Style::default().fg(color));
        }
    }
    draw_moon(canvas, width, ground, t);
    // The shooting star: a bright head with a fading tail, crossing two cells
    // right and one down every fifty milliseconds for the first part of the
    // period.
    let in_period = ms % SHOOTING_STAR_PERIOD_MS;
    let step = (in_period / 50) as usize;
    let start_x = ((ms / SHOOTING_STAR_PERIOD_MS) as usize * 37) % width.max(1);
    if step < ground.min(width / 2) {
        for back in 0..5 {
            let Some(s) = step.checked_sub(back) else {
                break;
            };
            let x = start_x + s * 2;
            let y = s;
            let (glyph, color) = match back {
                0 => ('\u{2726}', Color::Rgb(255, 255, 255)),
                1 => ('*', Color::Rgb(230, 230, 255)),
                _ => (
                    '\u{b7}',
                    dim(Color::Rgb(170, 170, 220), 1.0 - back as f32 / 6.0),
                ),
            };
            if y < ground {
                canvas.put(x, y, glyph, Style::default().fg(color));
            }
        }
    }
}

/// The moon, hung to the right of the sky and glowing gently. It is drawn
/// as a sphere rather than a picture of one: every cell of the disc gets
/// the light its own bit of surface reflects, so the moon is round at
/// whatever size the sky allows and dark where the maria are. Too shallow
/// a sky leaves it moonless.
pub(super) fn draw_moon(canvas: &mut Canvas, width: usize, ground: usize, t: f32) {
    let rows = ground.saturating_sub(MOON_Y + 1).min(MOON_MAX_ROWS);
    if rows < MOON_MIN_ROWS {
        return;
    }
    // A cell is twice as tall as it is wide, so a round disc is twice as
    // many cells across as it is rows down.
    let cols = rows * 2;
    if cols + MOON_SKY > width {
        return;
    }
    let moon_x = width.saturating_sub(cols + MOON_MARGIN);
    let glow = 0.94 + 0.06 * (t * 0.7).sin();
    for row in 0..rows {
        for col in 0..cols {
            // Cell middles, on the unit disc with y pointing up.
            let nx = (col as f32 + 0.5) * 2.0 / cols as f32 - 1.0;
            let ny = 1.0 - (row as f32 + 0.5) * 2.0 / rows as f32;
            let r2 = nx * nx + ny * ny;
            if r2 > 1.0 {
                continue;
            }
            let bright = moon_light(nx, ny, r2, col, row);
            let step = (bright * MOON_RAMP.len() as f32) as usize;
            let glyph = MOON_RAMP[step.min(MOON_RAMP.len() - 1)];
            // The glow rides on the colour alone: pulsing the glyphs too
            // would set the whole surface crawling.
            let shade = mix(Color::Rgb(64, 64, 82), Color::Rgb(255, 250, 226), bright);
            let mut style = Style::default().fg(dim(shade, glow));
            if bright > 0.72 {
                style = style.add_modifier(Modifier::BOLD);
            }
            // Every cell of the disc is painted, so the moon is opaque and
            // no star shows through it.
            canvas.put(moon_x + col, MOON_Y + row, glyph, style);
        }
    }
}

/// The light coming off the point of the moon's surface seen at `(nx, ny)`
/// on the disc. A full moon is lit from behind the reader, so what shapes
/// its face is not shadow but albedo: bright highlands, dark maria and
/// brighter ray craters, over a disc that dims a little towards the limb,
/// with a fine grain so the surface does not read as polished.
pub(super) fn moon_light(nx: f32, ny: f32, r2: f32, col: usize, row: usize) -> f32 {
    let mut albedo = MOON_HIGHLAND;
    for (mx, my, mr, depth) in MOON_MARIA.iter().copied() {
        albedo -= depth * moon_patch(nx, ny, mx, my, mr);
    }
    for (cx, cy, cr, lift) in MOON_CRATERS.iter().copied() {
        albedo += lift * moon_patch(nx, ny, cx, cy, cr);
    }
    // How far round the ball this point sits, which is what dims the limb.
    let z = (1.0 - r2).max(0.0).sqrt();
    let grain = (hash(col, row) % 100) as f32 / 100.0 - 0.5;
    let shade = MOON_LIMB + (1.0 - MOON_LIMB) * z.sqrt();
    (albedo * shade + grain * 0.04).clamp(0.0, 1.0)
}

/// How much of a patch centred on `(px, py)` with radius `pr` covers the
/// point `(x, y)`: all of it in the middle, none at the rim, eased between
/// so the plains have soft shores.
pub(super) fn moon_patch(x: f32, y: f32, px: f32, py: f32, pr: f32) -> f32 {
    let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt() / pr;
    if d >= 1.0 {
        return 0.0;
    }
    let e = 1.0 - d;
    e * e * (3.0 - 2.0 * e)
}

/// The diff scrolling up behind everything: the very lines the model is
/// reading, highlighted the way the diff pane highlights them and in the
/// colours a diff is read in. The change itself is what the wait is about, so
/// the backdrop shows it rather than a stand-in; it loops round, so a wait
/// long enough sees all of it go past. Only a diff too large to have been
/// kept falls back to abstract glyphs.
pub(super) fn draw_diff(canvas: &mut Canvas, feed: &Feed, width: usize, ground: usize, t: f32) {
    if feed.lines.is_empty() {
        draw_glyph_diff(canvas, width, ground, t);
        return;
    }
    let offset = (t * DIFF_SPEED) as usize;
    for y in 0..ground {
        let line = &feed.lines[(y + offset) % feed.lines.len()];
        // Lines nearer the top have been read and fade; the newest arrive
        // bright at the bottom.
        let depth = 0.55 + 0.45 * (y as f32 / ground.max(1) as f32);
        for (x, &(c, color)) in line.iter().take(width).enumerate() {
            if c != ' ' {
                canvas.put(x, y, c, Style::default().fg(dim(color, depth)));
            }
        }
    }
}

/// The stand-in for a diff there is nothing left of: lines of the right
/// shape in the right colours, drawn as runs of glyphs.
pub(super) fn draw_glyph_diff(canvas: &mut Canvas, width: usize, ground: usize, t: f32) {
    let offset = (t * DIFF_SPEED) as usize;
    for y in 0..ground {
        let line = y + offset;
        let seed = hash(line, 3);
        let (sign, color) = match seed % 7 {
            0 | 1 => ('+', DIFF_ADDED),
            2 => ('-', DIFF_REMOVED),
            3 => continue,
            _ => (' ', DIFF_CONTEXT),
        };
        // Lines nearer the top have been read and fade; the newest arrive
        // bright at the bottom.
        let depth = 0.55 + 0.45 * (y as f32 / ground.max(1) as f32);
        let indent = 1 + (seed >> 8) as usize % 4 * 4;
        let len = 6 + (seed >> 16) as usize % (width * 7 / 10).max(1);
        let style = Style::default().fg(dim(color, depth));
        canvas.put(0, y, sign, style);
        let mut x = indent + 1;
        let end = (x + len).min(width.saturating_sub(1));
        while x < end {
            let word = 2 + (hash(line, x) % 6) as usize;
            for _ in 0..word {
                if x >= end {
                    break;
                }
                let glyph = RAIN_GLYPHS[(hash(line, x + 99) % RAIN_GLYPHS.len() as u64) as usize];
                canvas.put(x, y, glyph, style);
                x += 1;
            }
            x += 1;
        }
    }
}
