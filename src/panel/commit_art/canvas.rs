//! A character canvas the scene is painted on, and the colour arithmetic it uses.

use super::*;

/// A grid of styled cells the scene is painted onto, then read out as rows.
pub(super) struct Canvas {
    pub(super) cells: Vec<Vec<Cell>>,
}

impl Canvas {
    pub(super) fn new(width: usize, height: usize) -> Self {
        Self {
            cells: vec![vec![(' ', Style::default()); width]; height],
        }
    }

    pub(super) fn put(&mut self, x: usize, y: usize, c: char, style: Style) {
        if let Some(cell) = self.cells.get_mut(y).and_then(|row| row.get_mut(x)) {
            *cell = (c, style);
        }
    }

    pub(super) fn text(&mut self, x: usize, y: usize, text: &str, style: Style) {
        for (i, c) in text.chars().enumerate() {
            self.put(x + i, y, c, style);
        }
    }

    pub(super) fn width(&self) -> usize {
        self.cells.first().map_or(0, Vec::len)
    }

    /// Copy another canvas onto this one with its top left at row `y`.
    pub(super) fn blit(&mut self, other: &Canvas, y: usize) {
        for (dy, row) in other.cells.iter().enumerate() {
            for (x, &(c, style)) in row.iter().enumerate() {
                self.put(x, y + dy, c, style);
            }
        }
    }

    pub(super) fn lines(self) -> Vec<Line<'static>> {
        self.cells
            .into_iter()
            .map(|row| {
                // Runs of one style become one span; a span per cell would
                // be thousands of allocations a frame for nothing.
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut run = String::new();
                let mut run_style = Style::default();
                for (c, style) in row {
                    if style != run_style && !run.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut run), run_style));
                    }
                    run_style = style;
                    run.push(c);
                }
                if !run.is_empty() {
                    spans.push(Span::styled(run, run_style));
                }
                Line::from(spans)
            })
            .collect()
    }
}

/// `c` as it can go in a canvas cell, which holds one character in one
/// column: itself when that is what it takes, and a dot when it is not.
/// Combining marks, the wide scripts and emoji would otherwise leave the row
/// they land in a column short or a column over.
pub(super) fn cell(c: char) -> char {
    if c.is_ascii_graphic() || c == ' ' {
        return c;
    }
    let mut buf = [0u8; 4];
    if Span::raw(&*c.encode_utf8(&mut buf)).width() == 1 {
        c
    } else {
        '\u{b7}'
    }
}

/// A small, fast, deterministic hash for scattering stars, rain and glyphs.
pub(super) fn hash(a: usize, b: usize) -> u64 {
    let mut h = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (b as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 29)
}

/// A colour scaled towards black by `k` in 0..=1.
pub(super) fn dim(color: Color, k: f32) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * k) as u8,
            (g as f32 * k) as u8,
            (b as f32 * k) as u8,
        ),
        other => other,
    }
}

/// The colour `k` of the way from `a` to `b`.
pub(super) fn mix(a: Color, b: Color, k: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) => {
            let k = k.clamp(0.0, 1.0);
            let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
            Color::Rgb(lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
        }
        _ => a,
    }
}

/// A fully saturated colour at `hue` turns round the wheel.
pub(super) fn hue(hue: f32) -> Color {
    let h = (hue.rem_euclid(1.0)) * 6.0;
    let x = (1.0 - ((h % 2.0) - 1.0).abs()) * 255.0;
    let x = x as u8;
    match h as u8 {
        0 => Color::Rgb(255, x, 60),
        1 => Color::Rgb(x, 255, 60),
        2 => Color::Rgb(60, 255, x),
        3 => Color::Rgb(60, x, 255),
        4 => Color::Rgb(x, 60, 255),
        _ => Color::Rgb(255, 60, x),
    }
}
