//! A night sky: twinkling stars, the moon and the odd shooting star.

use super::*;

/// One cell in this many is a star.
const STAR_DENSITY: u64 = 14;
/// Milliseconds between shooting stars.
const SHOOTING_STAR_PERIOD_MS: u64 = 9_000;
/// The moon is not a drawing but a lit sphere worked out cell by cell:
/// these glyphs, faint to dense, stand for the light coming off it.
const MOON_RAMP: &[char] = &['.', ':', '-', '=', '+', '*', '#', '%', '@'];
/// Rows the disc spans, at most and at least. Under the minimum a sphere
/// has too few cells to read as round, so the sky is left moonless.
const MOON_MAX_ROWS: usize = 18;
const MOON_MIN_ROWS: usize = 8;
/// How much light the bright highlands throw back, and how much of that is
/// left at the limb: enough dimming to round the ball off, not so much that
/// its edge blurs into the sky.
const MOON_HIGHLAND: f32 = 0.95;
const MOON_LIMB: f32 = 0.78;
/// The near side's dark plains, as middle, radius and depth on the unit
/// disc with x to the right and y up. The moon shows us one face, so these
/// are placed where they are seen rather than turned into view.
const MOON_MARIA: &[(f32, f32, f32, f32)] = &[
    (-0.55, 0.10, 0.34, 0.46),  // Oceanus Procellarum
    (-0.28, 0.42, 0.26, 0.50),  // Mare Imbrium
    (0.10, 0.36, 0.18, 0.48),   // Mare Serenitatis
    (0.28, 0.17, 0.20, 0.48),   // Mare Tranquillitatis
    (0.61, 0.29, 0.10, 0.44),   // Mare Crisium
    (0.44, -0.01, 0.14, 0.42),  // Mare Fecunditatis
    (0.32, -0.17, 0.10, 0.38),  // Mare Nectaris
    (-0.02, 0.17, 0.09, 0.34),  // Mare Vaporum
    (-0.16, -0.34, 0.15, 0.36), // Mare Nubium
    (-0.37, -0.31, 0.11, 0.38), // Mare Humorum
];
/// The ray craters, laid out the same way: Tycho in the south with its
/// splash, and Copernicus above it.
const MOON_CRATERS: &[(f32, f32, f32, f32)] = &[
    (-0.11, -0.61, 0.11, 0.08), // Tycho
    (-0.27, 0.16, 0.07, 0.10),  // Copernicus
];
/// Where the moon would rather hang: this far in from the right edge, so it
/// is over the sky left of where the network sits below and the picture is
/// not all weight on one side. This is a preference, not a fit: a sky with
/// no room for it puts the moon as far right as it goes and no further.
const MOON_MARGIN: usize = (TREE_LEVELS - 1) * MAX_LEVEL_STEP + 8;
/// The fit, on the other hand: this much sky beside the moon or none at
/// all, since a moon with the sky crowded around it is worse than a clear
/// night.
const MOON_SKY: usize = 32;
/// The row the moon hangs from.
const MOON_Y: usize = 1;

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
fn draw_moon(canvas: &mut Canvas, width: usize, ground: usize, t: f32) {
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
fn moon_light(nx: f32, ny: f32, r2: f32, col: usize, row: usize) -> f32 {
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
fn moon_patch(x: f32, y: f32, px: f32, py: f32, pr: f32) -> f32 {
    let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt() / pr;
    if d >= 1.0 {
        return 0.0;
    }
    let e = 1.0 - d;
    e * e * (3.0 - 2.0 * e)
}
