//! A tiny ray-marcher for drawing a mascot in three dimensions as shaded
//! text. A figure is a handful of signed-distance primitives, each with a
//! colour; each cell of the picture fires one ray at the figure, and where it
//! lands the surface's slope towards the light picks both how bright the
//! colour is and how dense the character. Turning the figure a little every
//! tick makes it rotate on the spot.

use ratatui::style::Color;

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

/// The figure's pose at time `t`. A live mascot turns at `turn_rate` on
/// average but speeds up and slows down as it goes, paces from side to side,
/// sways, breathes up and down, and every few seconds hops. An object such
/// as a monitor just turns on the spot: a monitor that hops is unsettling.
pub fn pose(t: f32, turn_rate: f32, alive: bool) -> Pose {
    if !alive {
        return Pose {
            yaw: t * turn_rate,
            tilt: 0.0,
            bob: 0.0,
            sway: 0.0,
            headroom: 0.0,
        };
    }
    let yaw = t * turn_rate + 0.7 * (t * 0.8).sin();
    let tilt = 0.14 * (t * 1.9).sin();
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

/// Characters typed on a screen per second.
const TYPING_RATE: f32 = 9.0;
/// Seconds the finished text stays up before the screen clears and typing
/// starts over.
const TYPING_PAUSE: f32 = 2.5;

const WHITE: Color = Color::Rgb(250, 250, 250);
const DARK: Color = Color::Rgb(28, 28, 36);
const RED: Color = Color::Rgb(255, 80, 80);

/// Two eyes looking out of the front of a figure at `y`, `x` apart, with
/// their pupils on the surface at depth `z`.
fn eyes(x: f32, y: f32, z: f32, r: f32) -> [Part; 4] {
    [
        mark(Shape::Sphere { c: v(-x, y, z), r }, WHITE, 'O'),
        mark(Shape::Sphere { c: v(x, y, z), r }, WHITE, 'O'),
        mark(
            Shape::Sphere {
                c: v(-x, y, z + r * 0.85),
                r: r * 0.38,
            },
            DARK,
            '#',
        ),
        mark(
            Shape::Sphere {
                c: v(x, y, z + r * 0.85),
                r: r * 0.38,
            },
            DARK,
            '#',
        ),
    ]
}

/// Kodee, the Kotlin mascot: a purple body with two pointed ears, a black
/// screen for a face with two big white eyes, and noodle arms and legs. The
/// right arm waves with `t`.
pub fn kodee(t: f32) -> Vec<Part> {
    let purple = Color::Rgb(125, 82, 255);
    let wave = (t * 2.0).sin();
    let mut parts = vec![
        part(
            Shape::RoundBox {
                c: v(0.0, 0.1, 0.0),
                h: v(0.95, 0.75, 0.3),
                r: 0.2,
            },
            purple,
        ),
        part(
            Shape::Prism {
                c: v(-0.62, 0.9, 0.0),
                w: 0.42,
                h: 0.6,
                d: 0.3,
            },
            purple,
        ),
        part(
            Shape::Prism {
                c: v(0.62, 0.9, 0.0),
                w: 0.42,
                h: 0.6,
                d: 0.3,
            },
            purple,
        ),
        mark(
            Shape::RoundBox {
                c: v(0.0, 0.05, 0.42),
                h: v(0.7, 0.45, 0.04),
                r: 0.06,
            },
            DARK,
            '#',
        ),
        part(
            Shape::Capsule {
                a: v(-1.0, -0.2, 0.0),
                b: v(-1.45, -0.9, 0.15),
                r: 0.11,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(1.0, -0.1, 0.0),
                b: v(1.5, 0.5 + 0.4 * wave, 0.2),
                r: 0.11,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(-0.4, -0.8, 0.0),
                b: v(-0.5, -1.65, 0.1),
                r: 0.12,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(0.4, -0.8, 0.0),
                b: v(0.5, -1.65, 0.1),
                r: 0.12,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(-0.5, -1.65, 0.1),
                b: v(-0.75, -1.65, 0.35),
                r: 0.12,
            },
            purple,
        ),
        part(
            Shape::Capsule {
                a: v(0.5, -1.65, 0.1),
                b: v(0.25, -1.65, 0.35),
                r: 0.12,
            },
            purple,
        ),
    ];
    parts.extend(eyes(0.3, 0.1, 0.5, 0.2));
    parts
}

/// Ferris the crab: a flat orange body with a spiky back, eyes on top,
/// six legs, and two claws that wave with `t`.
pub fn ferris(t: f32) -> Vec<Part> {
    let orange = Color::Rgb(247, 96, 20);
    let wave = (t * 2.0).sin() * 0.35;
    let mut parts = vec![part(
        Shape::Ellipsoid {
            c: v(0.0, -0.3, 0.0),
            r: v(1.35, 0.7, 0.95),
        },
        orange,
    )];
    for i in 0..5 {
        let x = -0.8 + 0.4 * i as f32;
        parts.push(part(
            Shape::Prism {
                c: v(x, 0.2, 0.0),
                w: 0.16,
                h: 0.4,
                d: 0.16,
            },
            orange,
        ));
    }
    for side in [-1.0f32, 1.0] {
        parts.push(part(
            Shape::Capsule {
                a: v(side * 1.2, -0.3, 0.2),
                b: v(side * 1.75, 0.1 + wave, 0.3),
                r: 0.13,
            },
            orange,
        ));
        parts.push(part(
            Shape::Ellipsoid {
                c: v(side * 1.95, 0.2 + wave, 0.3),
                r: v(0.38, 0.3, 0.25),
            },
            orange,
        ));
        for (i, x) in [0.45f32, 0.85, 1.2].into_iter().enumerate() {
            let z = 0.5 - 0.35 * i as f32;
            parts.push(part(
                Shape::Capsule {
                    a: v(side * x, -0.7, z),
                    b: v(side * (x + 0.3), -1.45, z + 0.2),
                    r: 0.1,
                },
                orange,
            ));
        }
    }
    parts.extend(eyes(0.45, -0.05, 0.75, 0.19));
    parts
}

/// The Go gopher: a tall rounded blue body, round ears, huge eyes, a nose
/// and two front teeth, and arms that swing with `t`.
pub fn gopher(t: f32) -> Vec<Part> {
    let blue = Color::Rgb(0, 190, 230);
    let swing = (t * 1.5).sin() * 0.25;
    let mut parts = vec![
        part(
            Shape::RoundBox {
                c: v(0.0, -0.15, 0.0),
                h: v(0.7, 1.0, 0.45),
                r: 0.45,
            },
            blue,
        ),
        part(
            Shape::Sphere {
                c: v(-0.85, 1.05, -0.1),
                r: 0.25,
            },
            blue,
        ),
        part(
            Shape::Sphere {
                c: v(0.85, 1.05, -0.1),
                r: 0.25,
            },
            blue,
        ),
        part(
            Shape::Sphere {
                c: v(0.0, 0.1, 0.78),
                r: 0.16,
            },
            Color::Rgb(235, 200, 160),
        ),
        mark(
            Shape::RoundBox {
                c: v(0.0, -0.22, 0.8),
                h: v(0.17, 0.12, 0.03),
                r: 0.02,
            },
            WHITE,
            '#',
        ),
        part(
            Shape::Capsule {
                a: v(-0.95, -0.5, 0.0),
                b: v(-1.35, -1.1 + swing, 0.3),
                r: 0.13,
            },
            blue,
        ),
        part(
            Shape::Capsule {
                a: v(0.95, -0.5, 0.0),
                b: v(1.35, -1.1 - swing, 0.3),
                r: 0.13,
            },
            blue,
        ),
        part(
            Shape::Ellipsoid {
                c: v(-0.45, -1.6, 0.25),
                r: v(0.35, 0.15, 0.4),
            },
            blue,
        ),
        part(
            Shape::Ellipsoid {
                c: v(0.45, -1.6, 0.25),
                r: v(0.35, 0.15, 0.4),
            },
            blue,
        ),
    ];
    parts.extend(eyes(0.4, 0.5, 0.75, 0.32));
    parts
}

/// Duke, the Java mascot: a dark triangle with a white lower half and a red
/// nose, waving one arm with `t`.
pub fn duke(t: f32) -> Vec<Part> {
    let dark = Color::Rgb(120, 120, 140);
    let wave = (t * 2.0).sin() * 0.4;
    vec![
        part(
            Shape::Prism {
                c: v(0.0, -1.2, 0.0),
                w: 1.3,
                h: 2.7,
                d: 0.45,
            },
            dark,
        ),
        part(
            Shape::RoundBox {
                c: v(0.0, -0.75, 0.2),
                h: v(0.75, 0.5, 0.3),
                r: 0.25,
            },
            WHITE,
        ),
        mark(
            Shape::Sphere {
                c: v(0.0, 0.05, 0.5),
                r: 0.2,
            },
            RED,
            '@',
        ),
        part(
            Shape::Capsule {
                a: v(-0.7, -0.4, 0.0),
                b: v(-1.35, -1.0, 0.2),
                r: 0.1,
            },
            dark,
        ),
        part(
            Shape::Capsule {
                a: v(0.7, -0.4, 0.0),
                b: v(1.4, 0.4 + wave, 0.2),
                r: 0.1,
            },
            dark,
        ),
        part(
            Shape::Ellipsoid {
                c: v(-0.4, -1.4, 0.3),
                r: v(0.3, 0.14, 0.35),
            },
            WHITE,
        ),
        part(
            Shape::Ellipsoid {
                c: v(0.4, -1.4, 0.3),
                r: v(0.3, 0.14, 0.35),
            },
            WHITE,
        ),
    ]
}

/// A python: coils of blue and yellow, a raised head that sways, and a
/// tongue that flicks with `t`.
pub fn snake(t: f32) -> Vec<Part> {
    let blue = Color::Rgb(70, 140, 200);
    let yellow = Color::Rgb(255, 212, 59);
    let flick = if (t * 3.0).sin() > 0.0 { 0.4 } else { 0.0 };
    let sway = (t * 1.3).sin() * 0.15;
    let head = v(0.45 + sway, 0.85, 0.4);
    vec![
        part(
            Shape::Torus {
                c: v(0.0, -1.35, 0.0),
                big: 0.95,
                small: 0.3,
            },
            blue,
        ),
        part(
            Shape::Torus {
                c: v(0.1, -0.85, 0.0),
                big: 0.7,
                small: 0.27,
            },
            yellow,
        ),
        part(
            Shape::Torus {
                c: v(0.15, -0.4, 0.0),
                big: 0.45,
                small: 0.24,
            },
            blue,
        ),
        part(
            Shape::Capsule {
                a: v(0.3, -0.3, 0.1),
                b: v(head.x, head.y - 0.25, head.z - 0.1),
                r: 0.22,
            },
            yellow,
        ),
        part(
            Shape::Ellipsoid {
                c: head,
                r: v(0.45, 0.3, 0.38),
            },
            blue,
        ),
        mark(
            Shape::Capsule {
                a: v(head.x + 0.45, head.y - 0.05, head.z + 0.05),
                b: v(head.x + 0.45 + flick, head.y - 0.15, head.z + 0.1),
                r: 0.07,
            },
            RED,
            '~',
        ),
        mark(
            Shape::Sphere {
                c: v(head.x - 0.15, head.y + 0.12, head.z + 0.3),
                r: 0.09,
            },
            WHITE,
            'O',
        ),
        mark(
            Shape::Sphere {
                c: v(head.x + 0.15, head.y + 0.12, head.z + 0.3),
                r: 0.09,
            },
            WHITE,
            'O',
        ),
    ]
}

/// A monitor with `code` being typed out on its coloured screen, its stand,
/// and a light that blinks with `t`. The text types out, sits a while, then
/// clears and starts again.
pub fn monitor(t: f32, screen: Color, code: &'static [&'static str]) -> Vec<Part> {
    let grey = Color::Rgb(170, 170, 190);
    let led = if (t * 2.0).sin() > 0.0 {
        Color::Rgb(90, 255, 120)
    } else {
        Color::Rgb(40, 90, 50)
    };
    let total = Screen::len(code);
    let cycle = total as f32 / TYPING_RATE + TYPING_PAUSE;
    let typed = ((t % cycle) * TYPING_RATE) as usize;
    let face = Shape::RoundBox {
        c: v(0.0, 0.35, 0.27),
        h: v(1.3, 0.8, 0.03),
        r: 0.02,
    };
    vec![
        part(
            Shape::RoundBox {
                c: v(0.0, 0.3, 0.0),
                h: v(1.5, 1.0, 0.15),
                r: 0.1,
            },
            grey,
        ),
        Part {
            shape: face,
            color: screen,
            glyph: Some(' '),
            screen: Some(Screen {
                c: v(0.0, 0.35, 0.27),
                h: v(1.3, 0.8, 0.03),
                lines: code,
                typed: typed.min(total),
                cursor_on: (t * 4.0).sin() > 0.0,
            }),
        },
        mark(
            Shape::Sphere {
                c: v(1.25, -0.55, 0.27),
                r: 0.07,
            },
            led,
            '*',
        ),
        part(
            Shape::Capsule {
                a: v(0.0, -0.7, 0.0),
                b: v(0.0, -1.4, 0.0),
                r: 0.15,
            },
            grey,
        ),
        part(
            Shape::RoundBox {
                c: v(0.0, -1.5, 0.0),
                h: v(0.8, 0.08, 0.5),
                r: 0.05,
            },
            grey,
        ),
    ]
}

/// A robot: a boxy head on a boxy body, cyan eyes, an antenna whose light
/// blinks with `t`, and arms that swing.
pub fn robot(t: f32) -> Vec<Part> {
    let steel = Color::Rgb(160, 170, 200);
    let cyan = Color::Rgb(80, 230, 255);
    let swing = (t * 1.5).sin() * 0.3;
    let light = if (t * 2.5).sin() > 0.0 {
        RED
    } else {
        Color::Rgb(90, 40, 40)
    };
    vec![
        part(
            Shape::RoundBox {
                c: v(0.0, 0.75, 0.0),
                h: v(0.8, 0.55, 0.55),
                r: 0.1,
            },
            steel,
        ),
        part(
            Shape::RoundBox {
                c: v(0.0, -0.6, 0.0),
                h: v(0.95, 0.7, 0.5),
                r: 0.1,
            },
            steel,
        ),
        part(
            Shape::Capsule {
                a: v(0.0, 1.3, 0.0),
                b: v(0.0, 1.7, 0.0),
                r: 0.05,
            },
            steel,
        ),
        mark(
            Shape::Sphere {
                c: v(0.0, 1.8, 0.0),
                r: 0.12,
            },
            light,
            '*',
        ),
        mark(
            Shape::Sphere {
                c: v(-0.32, 0.8, 0.5),
                r: 0.16,
            },
            cyan,
            'o',
        ),
        mark(
            Shape::Sphere {
                c: v(0.32, 0.8, 0.5),
                r: 0.16,
            },
            cyan,
            'o',
        ),
        mark(
            Shape::RoundBox {
                c: v(0.0, 0.4, 0.55),
                h: v(0.4, 0.06, 0.02),
                r: 0.01,
            },
            cyan,
            '=',
        ),
        part(
            Shape::Capsule {
                a: v(-1.05, -0.1, 0.0),
                b: v(-1.3, -1.0 + swing, 0.2),
                r: 0.13,
            },
            steel,
        ),
        part(
            Shape::Capsule {
                a: v(1.05, -0.1, 0.0),
                b: v(1.3, -1.0 - swing, 0.2),
                r: 0.13,
            },
            steel,
        ),
        part(
            Shape::RoundBox {
                c: v(-0.45, -1.55, 0.1),
                h: v(0.3, 0.15, 0.4),
                r: 0.05,
            },
            steel,
        ),
        part(
            Shape::RoundBox {
                c: v(0.45, -1.55, 0.1),
                h: v(0.3, 0.15, 0.4),
                r: 0.05,
            },
            steel,
        ),
    ]
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
    fn every_figure_renders_something_and_moves_with_time() {
        let figures: [fn(f32) -> Vec<Part>; 7] = [
            kodee,
            ferris,
            gopher,
            duke,
            snake,
            |t| monitor(t, Color::Rgb(247, 223, 30), &["const x = 1;", "run(x);"]),
            robot,
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
