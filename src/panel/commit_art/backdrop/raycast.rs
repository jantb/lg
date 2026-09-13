//! The engine the two first-person backdrops are built on: a maze to walk,
//! something to walk it with, and a frame buffer with twice the vertical
//! resolution of the terminal.
//!
//! Every cell is drawn as an upper half block, its foreground the top pixel
//! and its background the bottom one, so a pixel is about as wide as it is
//! tall and a wall has an edge rather than a staircase. Walls are found by
//! the same digital differential analyser the originals used: step the ray
//! from grid line to grid line and stop at the first solid cell, which gives
//! the exact distance to the face and where along it the ray landed, so the
//! columns line up into flat surfaces instead of a fan.

use super::grid::Rng;
use super::*;

/// Pixels per cell row.
const SUB: usize = 2;
/// The cell the two pixels of a column share.
const BLOCK: char = '\u{2580}';
/// How far a ray is followed before the sky is called empty.
const REACH: f32 = 40.0;
/// The eye's height in a room one unit tall.
const EYE: f32 = 0.5;

/// A frame buffer of pixels, two to a terminal row.
pub(super) struct Frame {
    w: usize,
    h: usize,
    px: Vec<Color>,
}

impl Frame {
    /// A buffer covering `rows` terminal rows, `w` cells across, all black.
    pub(super) fn new(w: usize, rows: usize) -> Self {
        Self {
            w,
            h: rows * SUB,
            px: vec![Color::Rgb(0, 0, 0); w * rows * SUB],
        }
    }

    pub(super) fn width(&self) -> usize {
        self.w
    }

    pub(super) fn height(&self) -> usize {
        self.h
    }

    pub(super) fn set(&mut self, x: usize, y: usize, color: Color) {
        if x < self.w && y < self.h {
            self.px[y * self.w + x] = color;
        }
    }

    /// Pull the whole picture `k` of the way towards `color`: a muzzle
    /// flash lighting the room, or a blast washing it out.
    pub(super) fn wash(&mut self, color: Color, k: f32) {
        for px in &mut self.px {
            *px = mix(*px, color, k);
        }
    }

    pub(super) fn flush(&self, put: &mut dyn FnMut(usize, usize, char, Style)) {
        for row in 0..self.h / SUB {
            for x in 0..self.w {
                let top = self.px[row * SUB * self.w + x];
                let bottom = self.px[(row * SUB + 1) * self.w + x];
                put(x, row, BLOCK, Style::default().fg(top).bg(bottom));
            }
        }
    }
}

/// A grid of cells: zero is open floor, anything else a wall of that kind.
pub(super) struct Map {
    pub(super) w: usize,
    pub(super) h: usize,
    cells: Vec<u8>,
}

impl Map {
    /// A maze `w` by `h`, both odd, carved by walking a random path and
    /// backing out of the dead ends, then opened up: a sixth of the walls
    /// left standing come down, which turns some of the corridors into
    /// rooms and leaves loops to walk round and sight lines to see down. Walls come in `kinds` sorts,
    /// scattered so that a stretch of corridor is of one material.
    pub(super) fn maze(rng: &mut Rng, w: usize, h: usize, kinds: u8) -> Self {
        let (w, h) = (w | 1, h | 1);
        let mut map = Self {
            w,
            h,
            cells: vec![1; w * h],
        };
        let mut stack = vec![(1usize, 1usize)];
        map.cells[w + 1] = 0;
        while let Some(&(x, y)) = stack.last() {
            let mut open: Vec<(usize, usize)> = Vec::new();
            for (dx, dy) in [(2i32, 0i32), (-2, 0), (0, 2), (0, -2)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx > 0 && ny > 0 && (nx as usize) < w - 1 && (ny as usize) < h - 1 {
                    let (nx, ny) = (nx as usize, ny as usize);
                    if map.cells[ny * w + nx] != 0 {
                        open.push((nx, ny));
                    }
                }
            }
            if open.is_empty() {
                stack.pop();
                continue;
            }
            let (nx, ny) = open[rng.below(open.len())];
            map.cells[(y + ny) / 2 * w + (x + nx) / 2] = 0;
            map.cells[ny * w + nx] = 0;
            stack.push((nx, ny));
        }
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                if map.cells[y * w + x] != 0 && rng.below(6) == 0 {
                    map.cells[y * w + x] = 0;
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                if map.cells[y * w + x] != 0 {
                    // Blocks of one material rather than a chequerboard of
                    // them: the kind is picked per four-cell block.
                    map.cells[y * w + x] = 1 + (hash(x / 4, y / 4) % kinds.max(1) as u64) as u8;
                }
            }
        }
        map
    }

    /// What stands at a cell; outside the grid is solid.
    pub(super) fn kind(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return 1;
        }
        self.cells[y as usize * self.w + x as usize]
    }

    pub(super) fn solid(&self, x: f32, y: f32) -> bool {
        self.kind(x.floor() as i32, y.floor() as i32) != 0
    }

    /// Every open cell, in order, for picking somewhere to be.
    pub(super) fn open(&self) -> Vec<(usize, usize)> {
        (0..self.h)
            .flat_map(|y| (0..self.w).map(move |x| (x, y)))
            .filter(|&(x, y)| self.cells[y * self.w + x] == 0)
            .collect()
    }

    /// The shortest way from one cell to another, the start not included.
    pub(super) fn route(&self, from: (usize, usize), to: (usize, usize)) -> Vec<(usize, usize)> {
        let mut prev = vec![usize::MAX; self.w * self.h];
        let mut queue = std::collections::VecDeque::from([from]);
        prev[from.1 * self.w + from.0] = from.1 * self.w + from.0;
        while let Some((x, y)) = queue.pop_front() {
            if (x, y) == to {
                break;
            }
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if self.kind(nx, ny) != 0 {
                    continue;
                }
                let i = ny as usize * self.w + nx as usize;
                if prev[i] == usize::MAX {
                    prev[i] = y * self.w + x;
                    queue.push_back((nx as usize, ny as usize));
                }
            }
        }
        let mut at = to.1 * self.w + to.0;
        if prev[at] == usize::MAX {
            return Vec::new();
        }
        let mut path = Vec::new();
        while at != from.1 * self.w + from.0 {
            path.push((at % self.w, at / self.w));
            at = prev[at];
        }
        path.reverse();
        path
    }
}

/// Where a ray ran into a wall.
pub(super) struct Hit {
    /// Distance along the view direction, so the wall is flat and not bowed.
    pub(super) dist: f32,
    pub(super) kind: u8,
    /// Whether the face hit runs north-south, which is lit differently from
    /// one running east-west.
    pub(super) vertical: bool,
    /// Where along the face the ray landed, 0 to 1.
    pub(super) tex: f32,
    /// The cell the wall stands in, for what is hung on this one and not
    /// on the next.
    pub(super) cell: (i32, i32),
}

/// Follow a ray from (`px`, `py`) until it meets a wall.
pub(super) fn cast(map: &Map, px: f32, py: f32, rdx: f32, rdy: f32) -> Hit {
    let (mut cx, mut cy) = (px.floor() as i32, py.floor() as i32);
    let step_x = if rdx < 0.0 { -1 } else { 1 };
    let step_y = if rdy < 0.0 { -1 } else { 1 };
    let delta_x = if rdx == 0.0 {
        f32::MAX
    } else {
        1.0 / rdx.abs()
    };
    let delta_y = if rdy == 0.0 {
        f32::MAX
    } else {
        1.0 / rdy.abs()
    };
    let mut side_x = if rdx < 0.0 {
        (px - cx as f32) * delta_x
    } else {
        (cx as f32 + 1.0 - px) * delta_x
    };
    let mut side_y = if rdy < 0.0 {
        (py - cy as f32) * delta_y
    } else {
        (cy as f32 + 1.0 - py) * delta_y
    };
    let mut vertical = true;
    let mut dist = 0.0;
    while dist < REACH {
        if side_x < side_y {
            dist = side_x;
            side_x += delta_x;
            cx += step_x;
            vertical = true;
        } else {
            dist = side_y;
            side_y += delta_y;
            cy += step_y;
            vertical = false;
        }
        let kind = map.kind(cx, cy);
        if kind != 0 {
            let tex = if vertical {
                py + dist * rdy
            } else {
                px + dist * rdx
            };
            return Hit {
                dist: dist.max(0.01),
                kind,
                vertical,
                tex: tex - tex.floor(),
                cell: (cx, cy),
            };
        }
    }
    Hit {
        dist: REACH,
        kind: 0,
        vertical,
        tex: 0.0,
        cell: (cx, cy),
    }
}

/// How much of the world the eye takes in up and down. The band the
/// backdrop gets is a wide letterbox, so the picture is squeezed sideways
/// rather than shown through a slit: the view is as wide as the box and as
/// tall as this, which is about what a person sees, and a corridor is
/// recognisable instead of being one wall filling the frame.
const VFOV: f32 = 1.45;

/// The eye: where it is, which way it looks, and how much it sees.
pub(super) struct View {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) a: f32,
    /// Rows the eye is lifted or dropped by the walk, in pixels.
    pub(super) bob: f32,
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) fov: f32,
}

impl View {
    /// Pixels a length of one covers across the view at one unit away.
    pub(super) fn scale(&self) -> f32 {
        self.w as f32 / (2.0 * (self.fov / 2.0).tan())
    }

    /// The same up and down, which is not the same number: the box is far
    /// wider than it is tall, so the two views are set apart and the picture
    /// is squeezed rather than cropped.
    pub(super) fn scale_v(&self) -> f32 {
        self.h as f32 / (2.0 * (VFOV / 2.0).tan())
    }

    /// The row the eye looks along.
    pub(super) fn horizon(&self) -> f32 {
        self.h as f32 / 2.0 + self.bob
    }

    /// The ray through pixel column `col`.
    pub(super) fn ray(&self, col: usize) -> (f32, f32) {
        let (dx, dy) = (self.a.cos(), self.a.sin());
        let plane = (self.fov / 2.0).tan();
        let camera = 2.0 * col as f32 / self.w as f32 - 1.0;
        (dx - dy * plane * camera, dy + dx * plane * camera)
    }

    /// The rows a wall one unit tall stands between at `dist`: its top, and
    /// the floor under it.
    pub(super) fn wall(&self, dist: f32) -> (f32, f32) {
        let half = self.scale_v() / dist;
        let floor = self.horizon() + EYE * half;
        (floor - half, floor)
    }

    /// How far away the floor is at pixel row `y`, below the horizon.
    pub(super) fn floor_dist(&self, y: f32) -> f32 {
        EYE * self.scale_v() / (y - self.horizon()).max(0.001)
    }

    /// Where in the world the floor under pixel (`col`, `y`) is.
    pub(super) fn floor_at(&self, col: usize, y: f32) -> (f32, f32) {
        let (rdx, rdy) = self.ray(col);
        let d = self.floor_dist(y) / (rdx * self.a.cos() + rdy * self.a.sin()).max(0.001);
        (self.x + rdx * d, self.y + rdy * d)
    }
}

/// A thing in the world drawn as a flat picture always facing the eye: one
/// string a row, a colour for each character it uses, and how tall it stands.
pub(super) struct Art {
    pub(super) rows: &'static [&'static str],
    pub(super) color: fn(char) -> Option<Color>,
    /// Height in world units, a room being one.
    pub(super) height: f32,
    /// How far off the floor it hangs, in world units.
    pub(super) lift: f32,
}

/// Draw `art` standing at (`sx`, `sy`), hidden by anything nearer in
/// `depth`, and lit by `light` (1 is its own colour, 0 is black).
pub(super) fn billboard(
    frame: &mut Frame,
    view: &View,
    depth: &[f32],
    sx: f32,
    sy: f32,
    art: &Art,
    light: f32,
) {
    let (dx, dy) = (sx - view.x, sy - view.y);
    let (ca, sa) = (view.a.cos(), view.a.sin());
    let forward = dx * ca + dy * sa;
    if forward < 0.2 {
        return;
    }
    let right = -dx * sa + dy * ca;
    let rows = art.rows.len() as f32;
    let cols = art
        .rows
        .iter()
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(1) as f32;
    // A figure keeps the shape it was drawn in. The view is squeezed
    // sideways to fit the letterbox, and a figure put through that squeeze
    // would be a smear a few pixels tall and half the width of the box, so
    // it is sized by the up-and-down scale both ways. Where it stands still
    // comes from the squeezed view, so it stays on the floor it is standing
    // on.
    let height = art.height * view.scale_v() / forward;
    let width = art.height * cols / rows * view.scale_v() / forward;
    let centre = view.w as f32 / 2.0 + right / forward * view.scale();
    let floor = view.horizon() + (EYE - art.lift) * view.scale_v() / forward;
    let (left, top) = (centre - width / 2.0, floor - height);
    for py in 0..height.ceil() as usize {
        let y = top + py as f32;
        if y < 0.0 || y >= view.h as f32 {
            continue;
        }
        let row = art.rows[((py as f32 / height * rows) as usize).min(art.rows.len() - 1)];
        for px in 0..width.ceil() as usize {
            let x = left + px as f32;
            if x < 0.0 || x >= view.w as f32 {
                continue;
            }
            let x = x as usize;
            if depth[x] < forward {
                continue;
            }
            let c = row
                .chars()
                .nth(((px as f32 / width * cols) as usize).min(row.chars().count().max(1) - 1))
                .unwrap_or(' ');
            if let Some(color) = (art.color)(c) {
                frame.set(x, y as usize, dim(color, light));
            }
        }
    }
}

/// Draw a picture over the bottom middle of the view, sized to the view
/// rather than to the world: the weapon in the hands, which is never any
/// farther away. `px_per_cell` is how many pixels a character of it covers,
/// `sway` shifts it as the walk goes and `raise` lifts it, both in pixels.
pub(super) fn held(
    frame: &mut Frame,
    art: &[&str],
    color: fn(char) -> Option<Color>,
    px_per_cell: f32,
    sway: f32,
    raise: f32,
) {
    let rows = art.len() as f32;
    let cols = art.iter().map(|r| r.chars().count()).max().unwrap_or(1) as f32;
    let height = rows * px_per_cell;
    let width = cols * px_per_cell;
    let left = (frame.width() as f32 - width) / 2.0 + sway;
    let top = frame.height() as f32 - height + raise;
    for py in 0..height.ceil() as usize {
        let y = top + py as f32;
        if y < 0.0 {
            continue;
        }
        let row = art[((py as f32 / height * rows) as usize).min(art.len() - 1)];
        let len = row.chars().count().max(1);
        for px in 0..width.ceil() as usize {
            let x = left + px as f32;
            if x < 0.0 {
                continue;
            }
            let c = row
                .chars()
                .nth(((px as f32 / width * cols) as usize).min(len - 1))
                .unwrap_or(' ');
            if let Some(color) = color(c) {
                frame.set(x as usize, y as usize, color);
            }
        }
    }
}

/// Whether someone stands clear of the walls where they are: a body has
/// shoulders, so the cell a point is in being open is not enough.
fn fits(map: &Map, x: f32, y: f32) -> bool {
    [
        (0.0, 0.0),
        (0.24, 0.0),
        (-0.24, 0.0),
        (0.0, 0.24),
        (0.0, -0.24),
    ]
    .into_iter()
    .all(|(ox, oy)| !map.solid(x + ox, y + oy))
}

/// Whether the straight line between two points is walkable, shoulders and
/// all: a way through has to be wide enough to fit down, not just a line
/// that misses the corners.
fn clear(map: &Map, from: (f32, f32), to: (f32, f32)) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let d = dx.hypot(dy);
    let steps = (d / 0.12).ceil() as usize;
    (0..=steps).all(|i| {
        let k = i as f32 / steps.max(1) as f32;
        let (x, y) = (from.0 + dx * k, from.1 + dy * k);
        fits(map, x, y)
    })
}

/// A step of `step` from `from` towards `to`, round the walls in between:
/// straight at it when the way is clear, and otherwise along the shortest
/// way through the maze, so that whatever is doing the chasing comes round
/// the corner rather than standing against the far side of a wall. Each
/// axis is tried against where the last one left it, so a diagonal never
/// slips through the corner of a wall.
pub(super) fn chase(map: &Map, from: (f32, f32), to: (f32, f32), step: f32) -> (f32, f32) {
    let mut aim = to;
    if !clear(map, from, to) {
        let here = (from.0 as usize, from.1 as usize);
        let there = (to.0 as usize, to.1 as usize);
        match map.route(here, there).first() {
            Some(&(cx, cy)) => aim = (cx as f32 + 0.5, cy as f32 + 0.5),
            None => return from,
        }
    }
    let (dx, dy) = (aim.0 - from.0, aim.1 - from.1);
    let d = dx.hypot(dy).max(0.001);
    let (nx, ny) = (from.0 + dx / d * step, from.1 + dy / d * step);
    let x = if fits(map, nx, from.1) { nx } else { from.0 };
    let y = if fits(map, x, ny) { ny } else { from.1 };
    (x, y)
}

/// Someone walking the maze: they pick somewhere to be, take the shortest
/// way there, and pick again when they arrive.
pub(super) struct Walker {
    pub(super) x: f32,
    pub(super) y: f32,
    /// The way they are walking.
    pub(super) a: f32,
    /// The way they are looking, which is the way they are walking unless
    /// something has taken their attention: a target is turned on and
    /// followed without breaking stride, the walk carrying on sideways
    /// underneath it.
    pub(super) view: f32,
    /// How far they have walked, for the swing it puts in the step.
    pub(super) walked: f32,
    path: Vec<(usize, usize)>,
    at: usize,
}

impl Walker {
    pub(super) fn new(map: &Map, rng: &mut Rng) -> Self {
        let open = map.open();
        let (x, y) = open[rng.below(open.len().max(1))];
        Self {
            x: x as f32 + 0.5,
            y: y as f32 + 0.5,
            a: 0.0,
            view: 0.0,
            walked: 0.0,
            path: Vec::new(),
            at: 0,
        }
    }

    /// Turn the eyes `most` radians towards `want`.
    pub(super) fn look(&mut self, want: f32, most: f32) {
        let mut delta = (want - self.view).rem_euclid(std::f32::consts::TAU);
        if delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        }
        self.view =
            (self.view + most.min(delta.abs()) * delta.signum()).rem_euclid(std::f32::consts::TAU);
    }

    /// Whether a cell has been reached.
    fn near(&self, (cx, cy): (usize, usize)) -> bool {
        (cx as f32 + 0.5 - self.x).hypot(cy as f32 + 0.5 - self.y) < 0.3
    }

    /// Walk for `dt` seconds at `speed` units a second, turning at `turn`
    /// radians a second. Walking and turning are separate: a corner is
    /// turned on the spot rather than cut through the wall.
    pub(super) fn step(&mut self, map: &Map, rng: &mut Rng, dt: f32, speed: f32, turn: f32) {
        if self.at >= self.path.len() {
            let open = map.open();
            if open.is_empty() {
                return;
            }
            let here = (self.x as usize, self.y as usize);
            self.path = map.route(here, open[rng.below(open.len())]);
            self.at = 0;
            if self.path.is_empty() {
                return;
            }
        }
        // Anything already reached is behind us.
        while self.at < self.path.len() && self.near(self.path[self.at]) {
            self.at += 1;
        }
        if self.at >= self.path.len() {
            return;
        }
        // Look as far down the path as can be walked in a straight line,
        // not at the next cell: the walk cuts corners and runs down the
        // middle of a corridor rather than zig-zagging from square to
        // square, and the view goes where the corridor goes.
        let mut aim = self.at;
        for (i, &(cx, cy)) in self.path[self.at..].iter().enumerate().take(8) {
            if clear(map, (self.x, self.y), (cx as f32 + 0.5, cy as f32 + 0.5)) {
                aim = self.at + i;
            }
        }
        let (tx, ty) = (self.path[aim].0 as f32 + 0.5, self.path[aim].1 as f32 + 0.5);
        let want = (ty - self.y).atan2(tx - self.x);
        let mut delta = (want - self.a).rem_euclid(std::f32::consts::TAU);
        if delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        }
        let swing = (turn * dt).min(delta.abs()) * delta.signum();
        self.a = (self.a + swing).rem_euclid(std::f32::consts::TAU);
        self.look(self.a, turn * 0.8 * dt);
        // Full speed down a straight, easing off into a corner.
        let ahead = (1.0 - delta.abs() / 0.8).clamp(0.0, 1.0) * speed * dt;
        // The heading lags the turn, so the step is taken one axis at a
        // time and only where there is room: a corner taken too tight is
        // brushed along rather than walked through.
        let (nx, ny) = (self.x + self.a.cos() * ahead, self.y + self.a.sin() * ahead);
        if fits(map, nx, self.y) {
            self.x = nx;
        }
        if fits(map, self.x, ny) {
            self.y = ny;
        }
        self.walked += ahead;
        if self.near(self.path[aim]) {
            self.at = aim + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_maze_is_walled_in_and_every_room_of_it_can_be_reached() {
        let mut rng = Rng::new(4);
        let map = Map::maze(&mut rng, 21, 21, 3);
        for i in 0..21 {
            assert_ne!(map.kind(i, 0), 0, "the maze is closed in");
            assert_ne!(map.kind(0, i), 0);
            assert_ne!(map.kind(i, 20), 0);
            assert_ne!(map.kind(20, i), 0);
        }
        let open = map.open();
        assert!(open.len() > 100, "and there is somewhere to walk");
        let from = open[0];
        for &to in &open {
            assert!(
                to == from || !map.route(from, to).is_empty(),
                "every open cell can be walked to"
            );
        }
    }

    #[test]
    fn a_walk_stays_out_of_the_walls() {
        let mut rng = Rng::new(9);
        let map = Map::maze(&mut rng, 21, 21, 3);
        let mut walker = Walker::new(&map, &mut rng);
        for _ in 0..4_000 {
            walker.step(&map, &mut rng, 0.033, 2.4, 3.0);
            assert!(!map.solid(walker.x, walker.y), "the walker is in the open");
        }
        assert!(
            walker.walked > 20.0,
            "and gets somewhere: {}",
            walker.walked
        );
    }

    #[test]
    fn a_ray_down_a_corridor_finds_the_wall_at_the_end_of_it() {
        let mut rng = Rng::new(1);
        let map = Map::maze(&mut rng, 21, 21, 4);
        for &(x, y) in map.open().iter().take(40) {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            for step in 0..16 {
                // Off the exact diagonals, where which of two cells sharing
                // a corner the ray grazes first is a coin toss.
                let a = (step as f32 + 0.37) * std::f32::consts::TAU / 16.0;
                let hit = cast(&map, px, py, a.cos(), a.sin());
                assert_ne!(hit.kind, 0, "a maze is walled in, so every ray lands");
                assert!((0.0..=1.0).contains(&hit.tex));
                // Walking the ray by hand reaches the wall at the same
                // distance the analyser jumped to.
                let mut marched = 0.0f32;
                while marched < hit.dist + 1.0
                    && !map.solid(px + a.cos() * marched, py + a.sin() * marched)
                {
                    marched += 0.005;
                }
                assert!(
                    (hit.dist - marched).abs() < 0.02,
                    "the wall is {marched} away, not {}",
                    hit.dist
                );
            }
        }
    }
}
