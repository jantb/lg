//! The diff itself scrolling up past the reader.

use super::rain::RAIN_GLYPHS;
use super::*;

/// Rows per second the diff scrolls.
const DIFF_SPEED: f32 = 4.0;

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
fn draw_glyph_diff(canvas: &mut Canvas, width: usize, ground: usize, t: f32) {
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
