//! The backdrops behind the mascot, one to a file: code raining down, a
//! night sky, the diff scrolling past, a round of GLTron, a game of Snake
//! and the trench run. A seed picks one and it stays for the whole wait.

use super::*;

mod diff;
mod grid;
mod night;
mod rain;
mod snake;
mod trench;
mod tron;

/// The backgrounds a wait can be set against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Backdrop {
    /// Code glyphs raining down.
    Rain,
    /// Stars twinkling under a moon, with the odd shooting star.
    Night,
    /// The diff itself scrolling up past the reader.
    Diff,
    /// Light cycles riding a round of GLTron.
    Tron,
    /// A snake hunting food and growing.
    Snake,
    /// The trench run: the Death Star, which goes up when the first word
    /// of the message flies in.
    Trench,
}

pub(super) const BACKDROPS: [Backdrop; 6] = [
    Backdrop::Rain,
    Backdrop::Night,
    Backdrop::Diff,
    Backdrop::Tron,
    Backdrop::Snake,
    Backdrop::Trench,
];

impl Backdrop {
    /// The backdrop `seed` picks.
    pub(super) fn pick(seed: usize) -> Self {
        BACKDROPS[seed % BACKDROPS.len()]
    }

    /// Paint the sky, `width` cells across and `ground` rows deep, as it
    /// stands for `show`.
    pub(super) fn draw(self, canvas: &mut Canvas, show: Show<'_>, width: usize, ground: usize) {
        let Show {
            seed,
            ms,
            feed,
            boom,
            ..
        } = show;
        let t = ms as f32 / 1000.0;
        let mut put = |x: usize, y: usize, c: char, style: Style| canvas.put(x, y, c, style);
        match self {
            Backdrop::Rain => rain::draw_rain(canvas, width, ground, t),
            Backdrop::Night => night::draw_night(canvas, width, ground, ms),
            Backdrop::Diff => diff::draw_diff(canvas, feed, width, ground, t),
            Backdrop::Tron => tron::frame(seed, width, ground, ms, &mut put),
            Backdrop::Snake => snake::frame(seed, width, ground, ms, &mut put),
            Backdrop::Trench => trench::frame(seed, width, ground, ms, boom, &mut put),
        }
    }
}
