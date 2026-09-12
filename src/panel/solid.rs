//! A tiny ray-marcher for drawing a mascot in three dimensions as shaded
//! text. A figure is a handful of signed-distance primitives, each with a
//! colour; each cell of the picture fires one ray at the figure, and where it
//! lands the surface's slope towards the light picks both how bright the
//! colour is and how dense the character. The figure stands facing the
//! reader, shifting its stance every few seconds, and its eyes follow the
//! reader as it turns.

use ratatui::style::Color;

mod figures;
pub use figures::{duke, ferris, gopher, kodee, monitor, robot, snake};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    fn add(self, o: V3) -> V3 {
        v(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn sub(self, o: V3) -> V3 {
        v(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn scale(self, k: f32) -> V3 {
        v(self.x * k, self.y * k, self.z * k)
    }
    fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    fn norm(self) -> V3 {
        let l = self.len();
        if l > 0.0 { self.scale(1.0 / l) } else { self }
    }
    fn abs(self) -> V3 {
        v(self.x.abs(), self.y.abs(), self.z.abs())
    }
    fn max0(self) -> V3 {
        v(self.x.max(0.0), self.y.max(0.0), self.z.max(0.0))
    }
    fn rot_y(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        v(self.x * c + self.z * s, self.y, -self.x * s + self.z * c)
    }
    fn rot_z(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        v(self.x * c - self.y * s, self.x * s + self.y * c, self.z)
    }
}

/// One solid piece of a figure.
#[derive(Debug, Clone, Copy)]
pub enum Shape {
    Sphere {
        c: V3,
        r: f32,
    },
    /// A box with rounded edges: centre, half extents, edge radius.
    RoundBox {
        c: V3,
        h: V3,
        r: f32,
    },
    /// A line segment thickened to `r`.
    Capsule {
        a: V3,
        b: V3,
        r: f32,
    },
    /// A triangular prism pointing up: centre of base, half width of base,
    /// height, and half depth.
    Prism {
        c: V3,
        w: f32,
        h: f32,
        d: f32,
    },
    /// A sphere stretched to different radii along each axis.
    Ellipsoid {
        c: V3,
        r: V3,
    },
    /// A ring lying flat: centre, ring radius, tube radius.
    Torus {
        c: V3,
        big: f32,
        small: f32,
    },
}

impl Shape {
    fn distance(&self, p: V3) -> f32 {
        match *self {
            Shape::Sphere { c, r } => p.sub(c).len() - r,
            Shape::RoundBox { c, h, r } => {
                let q = p.sub(c).abs().sub(h);
                q.max0().len() + q.x.max(q.y).max(q.z).min(0.0) - r
            }
            Shape::Capsule { a, b, r } => {
                let pa = p.sub(a);
                let ba = b.sub(a);
                let t = (pa.dot(ba) / ba.dot(ba)).clamp(0.0, 1.0);
                pa.sub(ba.scale(t)).len() - r
            }
            Shape::Prism { c, w, h, d } => {
                // A triangle in x/y extruded along z: inside when below the
                // two slanted sides and above the base.
                let q = p.sub(c);
                let slope = h / w;
                let side = (q.x.abs() * slope + q.y - h) / (1.0 + slope * slope).sqrt();
                let base = -q.y;
                let tri = side.max(base);
                let dz = q.z.abs() - d;
                tri.max(dz)
            }
            Shape::Ellipsoid { c, r } => {
                let q = p.sub(c);
                let k0 = v(q.x / r.x, q.y / r.y, q.z / r.z).len();
                let k1 = v(q.x / (r.x * r.x), q.y / (r.y * r.y), q.z / (r.z * r.z)).len();
                if k1 > 0.0 {
                    k0 * (k0 - 1.0) / k1
                } else {
                    -r.x.min(r.y).min(r.z)
                }
            }
            Shape::Torus { c, big, small } => {
                let q = p.sub(c);
                let ring = (q.x * q.x + q.z * q.z).sqrt() - big;
                (ring * ring + q.y * q.y).sqrt() - small
            }
        }
    }
}

/// A shape and what it is made of.
#[derive(Debug, Clone, Copy)]
pub struct Part {
    pub shape: Shape,
    pub color: Color,
    /// A character to draw this part in whatever the light, for things that
    /// should read as a flat panel or a mark rather than a shaded body.
    pub glyph: Option<char>,
    /// Text laid over the part's front face, for a screen with something on
    /// it. Where set, it picks the character instead of `glyph`.
    pub screen: Option<Screen>,
}

/// Lines of text on the front (+z) face of a box, typed out one character at
/// a time: `typed` characters are on the screen so far, and a cursor blinks
/// where the next one goes. The face is centred on `c` with half extents `h`.
#[derive(Debug, Clone, Copy)]
pub struct Screen {
    pub c: V3,
    pub h: V3,
    pub lines: &'static [&'static str],
    pub typed: usize,
    pub cursor_on: bool,
}

impl Screen {
    /// Characters in the text, counting one for each line break.
    pub fn len(lines: &[&str]) -> usize {
        lines.iter().map(|l| l.chars().count() + 1).sum()
    }

    /// The character showing at the point `p` on the face: the text is
    /// stretched to fill the face, so one text cell may be several picture
    /// cells or none.
    fn glyph_at(&self, p: V3) -> char {
        let rows = self.lines.len().max(1);
        let cols = self
            .lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(1)
            .max(1);
        let u = ((p.x - self.c.x + self.h.x) / (2.0 * self.h.x)).clamp(0.0, 0.999);
        let v = ((self.c.y + self.h.y - p.y) / (2.0 * self.h.y)).clamp(0.0, 0.999);
        let col = (u * cols as f32) as usize;
        let row = (v * rows as f32) as usize;
        let before: usize = self.lines[..row]
            .iter()
            .map(|l| l.chars().count() + 1)
            .sum();
        let index = before + col;
        let line_len = self.lines[row].chars().count();
        if index < self.typed {
            if col < line_len {
                self.lines[row].chars().nth(col).unwrap_or(' ')
            } else {
                ' '
            }
        } else if index == self.typed && col <= line_len && self.cursor_on {
            '\u{258c}'
        } else {
            ' '
        }
    }
}

/// The characters a surface is drawn in, dimmest first.
const RAMP: &[char] = &['.', ':', '-', '=', '+', '*', '#', '%', '@'];

/// How a figure sits in its box at one moment: turned `yaw` about the
/// vertical, leaning `tilt` radians to the side, raised `bob` units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub yaw: f32,
    pub tilt: f32,
    pub bob: f32,
    /// Units the figure has paced to the right of the middle.
    pub sway: f32,
    /// Units of air above the figure for it to hop into.
    pub headroom: f32,
}

/// Seconds between hops.
const HOP_PERIOD: f32 = 4.0;
/// How long a hop is in the air.
const HOP_TIME: f32 = 0.7;
/// Seconds a stance is held before the figure shifts to the next, and how
/// long the shift takes.
const SHIFT_PERIOD: f32 = 5.0;
const SHIFT_TIME: f32 = 0.9;
/// Seconds between blinks, when in the period the eyes close, and for how
/// long. The first blink comes well after the figure appears, so its eyes
/// are open when it does.
const BLINK_PERIOD: f32 = 3.7;
const BLINK_AT: f32 = 1.0;
const BLINK_TIME: f32 = 0.14;

/// A small deterministic scatter in 0..1 for stance `n`.
fn noise(n: i32) -> f32 {
    let mut h = (n as u32).wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    (h % 10_000) as f32 / 10_000.0
}

/// The yaw the figure holds at time `t`: a stance within `turn` radians of
/// facing the reader, eased to a new one every few seconds. It never turns
/// far enough to look away.
fn stance(t: f32, turn: f32) -> f32 {
    let n = (t / SHIFT_PERIOD).floor();
    let into = t - n * SHIFT_PERIOD;
    let at = |n: i32| (noise(n) * 2.0 - 1.0) * turn;
    let (from, to) = (at(n as i32), at(n as i32 + 1));
    let k = (into / SHIFT_TIME).clamp(0.0, 1.0);
    let ease = k * k * (3.0 - 2.0 * k);
    from + (to - from) * ease
}

/// The figure's pose at time `t`. A live mascot faces the reader, shifting
/// its stance within `turn` radians of straight on, leaning into each turn,
/// pacing from side to side, breathing up and down, and every few seconds
/// hopping. An object such as a monitor only turns a little on the spot: a
/// monitor that hops is unsettling.
pub fn pose(t: f32, turn: f32, alive: bool) -> Pose {
    if !alive {
        return Pose {
            yaw: stance(t, turn * 0.6) + 0.05 * (t * 0.6).sin(),
            tilt: 0.0,
            bob: 0.0,
            sway: 0.0,
            headroom: 0.0,
        };
    }
    let stance = stance(t, turn);
    let yaw = stance + 0.05 * (t * 1.3).sin();
    let tilt = 0.12 * (t * 1.9).sin() + 0.2 * stance;
    let breathe = 0.06 * (t * 2.6).sin();
    let into_hop = t % HOP_PERIOD;
    let hop = if into_hop < HOP_TIME {
        // A parabola: up and back down to where it started.
        let k = into_hop / HOP_TIME;
        1.4 * k * (1.0 - k)
    } else {
        0.0
    };
    Pose {
        yaw,
        tilt: tilt + hop * 0.3,
        bob: breathe + hop,
        sway: 0.7 * (t * 0.45).sin(),
        headroom: 0.4,
    }
}

/// Where a figure's eyes are looking, and whether they are open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gaze {
    /// The direction the pupils point, in the figure's own frame.
    pub look: V3,
    pub blink: bool,
}

impl Gaze {
    /// Straight ahead, eyes open.
    pub fn forward() -> Self {
        Self {
            look: v(0.0, 0.0, 1.0),
            blink: false,
        }
    }

    /// Looking back at a reader the figure has turned `yaw` radians away
    /// from, so the eyes stay on them whichever way it stands; the glance
    /// drifts a little so the eyes are alive rather than fixed, and every
    /// few seconds they blink.
    pub fn at(t: f32, yaw: f32) -> Self {
        let drift = yaw + 0.18 * (t * 0.9).sin();
        let look = v(drift.sin(), 0.12 * (t * 0.7).sin(), drift.cos()).norm();
        let into = t.rem_euclid(BLINK_PERIOD);
        Self {
            look,
            blink: (BLINK_AT..BLINK_AT + BLINK_TIME).contains(&into),
        }
    }
}

/// Render `parts` turned by `angle` about the vertical into a grid `cols` by
/// `rows`, where each cell is twice as tall as it is wide. `None` where the
/// ray hits nothing.
pub fn render(
    parts: &[Part],
    angle: f32,
    cols: usize,
    rows: usize,
) -> Vec<Vec<Option<(char, Color)>>> {
    render_posed(
        parts,
        Pose {
            yaw: angle,
            tilt: 0.0,
            bob: 0.0,
            sway: 0.0,
            headroom: 0.0,
        },
        cols,
        rows,
    )
}

/// Render `parts` in `pose` into a grid `cols` by `rows`.
pub fn render_posed(
    parts: &[Part],
    pose: Pose,
    cols: usize,
    rows: usize,
) -> Vec<Vec<Option<(char, Color)>>> {
    // The figure lives in a box about 3.6 units wide and 3.6 high; the view
    // is scaled so that box, plus any room for a hop, just fits the grid.
    let extent_y = 3.6 + pose.headroom;
    let extent_x = extent_y * cols as f32 / (rows as f32 * 2.0);
    let angle = pose.yaw;
    // Moving and tilting the figure is moving and tilting the camera the
    // other way, which keeps the distance fields where they are.
    let to_figure = |p: V3| p.rot_z(-pose.tilt).rot_y(angle);
    let light = v(-0.5, 0.8, 1.0).norm();
    let mut out = vec![vec![None; cols]; rows];
    let dist = |p: V3| -> (f32, usize) {
        let mut best = (f32::MAX, 0);
        for (i, part) in parts.iter().enumerate() {
            let d = part.shape.distance(p);
            if d < best.0 {
                best = (d, i);
            }
        }
        best
    };
    for (row, line) in out.iter_mut().enumerate() {
        for (col, cell) in line.iter_mut().enumerate() {
            let sx = ((col as f32 + 0.5) / cols as f32 - 0.5) * extent_x;
            let sy = (0.5 - (row as f32 + 0.5) / rows as f32) * extent_y;
            // Orthographic rays from the front, the figure rotated to meet them.
            let mut p = to_figure(v(sx - pose.sway, sy - pose.bob + pose.headroom / 2.0, 3.0));
            let dir = to_figure(v(0.0, 0.0, -1.0));
            let mut hit = None;
            for _ in 0..64 {
                let (d, i) = dist(p);
                if d < 0.004 {
                    hit = Some(i);
                    break;
                }
                if d > 8.0 {
                    break;
                }
                p = p.add(dir.scale(d));
            }
            let Some(i) = hit else { continue };
            let part = parts[i];
            let e = 0.01;
            let n = v(
                dist(p.add(v(e, 0.0, 0.0))).0 - dist(p.sub(v(e, 0.0, 0.0))).0,
                dist(p.add(v(0.0, e, 0.0))).0 - dist(p.sub(v(0.0, e, 0.0))).0,
                dist(p.add(v(0.0, 0.0, e))).0 - dist(p.sub(v(0.0, 0.0, e))).0,
            )
            .norm();
            let light_here = to_figure(light);
            let diffuse = n.dot(light_here).max(0.0);
            let shade = 0.35 + 0.65 * diffuse;
            let ch = part
                .screen
                .map(|s| s.glyph_at(p))
                .or(part.glyph)
                .unwrap_or_else(|| {
                    RAMP[((diffuse * (RAMP.len() - 1) as f32).round() as usize).min(RAMP.len() - 1)]
                });
            *cell = Some((
                ch,
                dim(
                    part.color,
                    if part.glyph.is_some() || part.screen.is_some() {
                        0.6 + 0.4 * shade
                    } else {
                        shade
                    },
                ),
            ));
        }
    }
    out
}

fn dim(color: Color, k: f32) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * k) as u8,
            (g as f32 * k) as u8,
            (b as f32 * k) as u8,
        ),
        other => other,
    }
}

fn part(shape: Shape, color: Color) -> Part {
    Part {
        shape,
        color,
        glyph: None,
        screen: None,
    }
}

fn mark(shape: Shape, color: Color, glyph: char) -> Part {
    Part {
        shape,
        color,
        glyph: Some(glyph),
        screen: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(parts: &[Part], angle: f32) -> String {
        render(parts, angle, 40, 20)
            .iter()
            .map(|row| {
                row.iter()
                    .map(|c| c.map_or(' ', |(ch, _)| ch))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn count_glyph(grid: &[Vec<Option<(char, Color)>>], glyph: char) -> usize {
        grid.iter()
            .flatten()
            .filter(|c| matches!(c, Some((g, _)) if *g == glyph))
            .count()
    }

    #[test]
    fn kodee_faces_the_viewer_with_two_eyes_and_turns() {
        let kodee = |t| kodee(t, Gaze::forward());
        let front = render(&kodee(0.0), 0.0, 40, 20);
        assert!(
            count_glyph(&front, 'O') >= 6,
            "both eyes show from the front:\n{}",
            picture(&kodee(0.0), 0.0)
        );
        assert_ne!(
            picture(&kodee(0.0), 0.0),
            picture(&kodee(0.0), 1.0),
            "turning changes the picture"
        );
        let back = render(&kodee(0.0), std::f32::consts::PI, 40, 20);
        assert_eq!(
            count_glyph(&back, 'O'),
            0,
            "no eyes from behind:\n{}",
            picture(&kodee(0.0), std::f32::consts::PI)
        );
    }

    #[test]
    fn the_figure_keeps_facing_the_reader_and_its_eyes_follow_and_blink() {
        // Over a long wait the stance wanders but never turns the back.
        for step in 0..600 {
            let t = step as f32 * 0.1;
            let pose = pose(t, 0.55, true);
            assert!(
                pose.yaw.abs() < 0.8,
                "yaw {} at {t}s faces the reader",
                pose.yaw
            );
        }
        // The pupils turn with the figure so they stay on the reader: turned
        // to one side, the eyes still show from the front. Nothing else on
        // Ferris is that dark a blue, so such a cell is a pupil.
        let pupils = |grid: &[Vec<Option<(char, Color)>>]| {
            grid.iter()
                .flatten()
                .filter(
                    |c| matches!(c, Some((_, Color::Rgb(r, g, b))) if *r < 30 && *g < 30 && b >= r),
                )
                .count()
        };
        let turned = pose(7.0, 0.55, true);
        let grid = render_posed(&ferris(7.0, Gaze::at(7.0, turned.yaw)), turned, 44, 20);
        assert!(pupils(&grid) > 0, "pupils show while turned");
        // And every few seconds they close.
        let blinks = (0..400)
            .filter(|i| Gaze::at(*i as f32 * 0.01, 0.0).blink)
            .count();
        assert!(blinks > 0, "a blink comes within four seconds");
        assert!(!Gaze::at(0.0, 0.0).blink, "the eyes are open to begin with");
        let shut = render(&ferris(1.05, Gaze::at(1.05, 0.0)), 0.0, 44, 20);
        assert_eq!(pupils(&shut), 0, "no pupils while blinking");
        assert_eq!(count_glyph(&shut, 'O'), 0, "the eyes are shut");
    }

    #[test]
    fn every_figure_renders_something_and_moves_with_time() {
        let figures: [fn(f32) -> Vec<Part>; 7] = [
            |t| kodee(t, Gaze::forward()),
            |t| ferris(t, Gaze::forward()),
            |t| gopher(t, Gaze::forward()),
            duke,
            |t| snake(t, Gaze::forward()),
            |t| monitor(t, Color::Rgb(247, 223, 30), &["const x = 1;", "run(x);"]),
            |t| robot(t, Gaze::forward()),
        ];
        for figure in figures {
            let grid = render(&figure(0.0), 0.3, 44, 20);
            assert_eq!(grid.len(), 20);
            assert!(grid.iter().all(|row| row.len() == 44));
            let filled = grid.iter().flatten().filter(|c| c.is_some()).count();
            assert!(
                filled > 100,
                "the figure fills a good part of its grid: {filled}"
            );
            assert_ne!(
                picture(&figure(0.0), 0.3),
                picture(&figure(1.0), 0.3),
                "the figure moves with time:\n{}",
                picture(&figure(0.0), 0.3)
            );
        }
    }
}
