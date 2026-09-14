//! What the arcade backdrops have in common: a grid of cells twice as tall
//! as they are wide, four headings, a seeded random source, explosions, and
//! the frame clock that keeps a simulation running between draws so that
//! any frame is a pure function of the seed and the clock.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::thread::LocalKey;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub(super) const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    pub(super) fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    pub(super) fn left(self) -> Dir {
        match self {
            Dir::Up => Dir::Left,
            Dir::Left => Dir::Down,
            Dir::Down => Dir::Right,
            Dir::Right => Dir::Up,
        }
    }

    pub(super) fn right(self) -> Dir {
        self.left().left().left()
    }

    pub(super) fn reverse(self) -> Dir {
        self.left().left()
    }

    pub(super) fn horizontal(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }

    pub(super) fn arrow(self) -> char {
        match self {
            Dir::Up => '^',
            Dir::Down => 'v',
            Dir::Left => '<',
            Dir::Right => '>',
        }
    }

    /// The line glyph joining a step heading `self` to one heading `next`.
    pub(super) fn corner(self, next: Dir) -> char {
        match (self, next) {
            (Dir::Left | Dir::Right, Dir::Left | Dir::Right) => '\u{2500}',
            (Dir::Up | Dir::Down, Dir::Up | Dir::Down) => '\u{2502}',
            (Dir::Right, Dir::Up) | (Dir::Down, Dir::Left) => '\u{2518}',
            (Dir::Right, Dir::Down) | (Dir::Up, Dir::Left) => '\u{2510}',
            (Dir::Left, Dir::Up) | (Dir::Down, Dir::Right) => '\u{2514}',
            (Dir::Left, Dir::Down) | (Dir::Up, Dir::Right) => '\u{250c}',
        }
    }
}

/// xorshift64*: plenty for scattering spawns and food.
#[derive(Debug, Clone)]
pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: usize) -> Self {
        Self((seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n`.
    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The cell one step from (`x`, `y`) in `dir`, if it is on a grid `width`
/// by `height`.
pub(super) fn step_from(
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    dir: Dir,
) -> Option<(usize, usize)> {
    let (dx, dy) = dir.delta();
    let nx = x as i32 + dx;
    let ny = y as i32 + dy;
    (nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height)
        .then_some((nx as usize, ny as usize))
}

/// How many cells can be reached from (`x`, `y`) through cells `passable`
/// says are open, counting up to `limit`. Enough to tell a dead end from
/// open ground without walking the whole grid.
pub(super) fn room(
    width: usize,
    height: usize,
    passable: impl Fn(usize, usize) -> bool,
    x: usize,
    y: usize,
    limit: usize,
) -> usize {
    if !passable(x, y) {
        return 0;
    }
    let mut seen = vec![false; width * height];
    let mut queue = VecDeque::from([(x, y)]);
    seen[y * width + x] = true;
    let mut count = 0;
    while let Some((cx, cy)) = queue.pop_front() {
        count += 1;
        if count >= limit {
            break;
        }
        for dir in Dir::ALL {
            if let Some((nx, ny)) = step_from(width, height, cx, cy, dir) {
                let i = ny * width + nx;
                if !seen[i] && passable(nx, ny) {
                    seen[i] = true;
                    queue.push_back((nx, ny));
                }
            }
        }
    }
    count
}

/// Ticks an explosion takes to expand and go out.
pub(super) const EXPLOSION_TICKS: u64 = 9;

#[derive(Debug, Clone)]
pub(super) struct Explosion {
    pub(super) x: usize,
    pub(super) y: usize,
    /// The tick it went off.
    pub(super) start: u64,
    /// Rows the blast reaches at its widest.
    pub(super) radius: f32,
}

impl Explosion {
    /// Whether the blast is still burning over a cell.
    pub(super) fn covers(&self, x: usize, y: usize) -> bool {
        let dx = (x as f32 - self.x as f32) / 2.0;
        let dy = y as f32 - self.y as f32;
        (dx * dx + dy * dy).sqrt() <= self.radius + 0.5
    }

    pub(super) fn over(&self, tick: u64) -> bool {
        tick.saturating_sub(self.start) > EXPLOSION_TICKS
    }

    /// A blast growing from its centre: a ring of sparks widening over the
    /// explosion's life, white-hot at first and cooling to a dull red, with
    /// a dense core that burns out as the ring leaves it behind. `ms` is
    /// the clock; `tick_ms` how long the simulation's tick is on it.
    pub(super) fn draw(
        &self,
        ms: u64,
        tick_ms: u64,
        width: usize,
        height: usize,
        put: &mut dyn FnMut(usize, usize, char, Style),
    ) {
        let start_ms = self.start * tick_ms;
        if ms < start_ms {
            return;
        }
        let life = (ms - start_ms) as f32 / (EXPLOSION_TICKS * tick_ms) as f32;
        if life > 1.0 {
            return;
        }
        let radius = self.radius * life.sqrt();
        let heat = 1.0 - life;
        let color = if heat > 0.66 {
            mix(
                Color::Rgb(255, 240, 120),
                Color::Rgb(255, 255, 255),
                (heat - 0.66) * 3.0,
            )
        } else if heat > 0.33 {
            mix(
                Color::Rgb(255, 110, 30),
                Color::Rgb(255, 240, 120),
                (heat - 0.33) * 3.0,
            )
        } else {
            mix(
                Color::Rgb(120, 20, 10),
                Color::Rgb(255, 110, 30),
                heat * 3.0,
            )
        };
        let ring: &[char] = if life < 0.3 {
            &['*', '#', '%']
        } else if life < 0.6 {
            &['*', '+', 'x']
        } else {
            &['+', '.', 'x']
        };
        let reach = (self.radius.ceil() as i32).max(1);
        for dy in -reach..=reach {
            for dx in -reach * 2..=reach * 2 {
                let d = ((dx as f32 / 2.0).powi(2) + (dy as f32).powi(2)).sqrt();
                let on_ring = (d - radius).abs() < 0.75;
                let core = d < radius * 0.4 && life < 0.5;
                if !(on_ring || core) {
                    continue;
                }
                let x = self.x as i32 + dx;
                let y = self.y as i32 + dy;
                if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
                    continue;
                }
                let glyph = if core {
                    '@'
                } else {
                    ring[((x * 31 + y * 17) as usize) % ring.len()]
                };
                let style = Style::default()
                    .fg(if core { dim(color, 0.8) } else { color })
                    .add_modifier(Modifier::BOLD);
                put(x as usize, y as usize, glyph, style);
            }
        }
    }
}

/// Whether any of `explosions` is still burning over a cell.
pub(super) fn burning(explosions: &[Explosion], x: usize, y: usize) -> bool {
    explosions.iter().any(|e| e.covers(x, y))
}

/// A simulation that runs on a fixed tick and can be drawn at any moment
/// within one. It is started at the height the box had then and keeps it:
/// the message streaming in above takes rows off the box as it grows, and
/// a game that started over each time a line landed would never get
/// anywhere. Each draw is told the rows it has now instead.
pub(super) trait Sim: Sized + 'static {
    /// Milliseconds between steps.
    const TICK_MS: u64;
    fn new(seed: usize, width: usize, height: usize, tick: u64) -> Self;
    /// The seed and width this was started with. The height is not part of
    /// it: the same world is drawn into whatever rows are left.
    fn key(&self) -> (usize, usize);
    fn tick(&self) -> u64;
    fn step(&mut self);
    /// Draw the world into `height` rows.
    fn draw(&self, ms: u64, height: usize, put: &mut dyn FnMut(usize, usize, char, Style));
}

/// A `put` into `height` rows for a world `world_height` rows tall: the
/// world keeps its feet on the ground line, so when the box is shorter the
/// top rows are out of view, and when it is taller the sky above is empty.
pub(super) fn grounded<'a>(
    world_height: usize,
    height: usize,
    put: &'a mut dyn FnMut(usize, usize, char, Style),
) -> impl FnMut(usize, usize, char, Style) + 'a {
    let hidden = world_height.saturating_sub(height);
    let lowered = height.saturating_sub(world_height);
    move |x, y, c, style| {
        if y >= hidden {
            put(x, y - hidden + lowered, c, style);
        }
    }
}

/// Simulations kept at once. The commit box has one size at a time, but the
/// tests and a resizing terminal want a few.
const KEPT: usize = 16;
/// Farther behind the clock than this and the simulation is started over
/// rather than caught up: a long pause is not worth a stall.
const MAX_CATCH_UP: u64 = 400;

/// Draw the simulation for `seed`, `width` by `height`, as it stands at
/// `ms`, keeping it in `store` between frames. A frame is a pure function of
/// clock and seed only if the simulation that got there is the same one
/// every time, so it is stepped forward to the clock rather than rebuilt.
/// The clock is expected to run forward: a call for an earlier `ms` than
/// the one caught up to starts a fresh round. A change of height does not:
/// the world that is running is drawn into the rows there are.
pub(super) fn frame<S: Sim>(
    store: &'static LocalKey<RefCell<Vec<S>>>,
    seed: usize,
    width: usize,
    height: usize,
    ms: u64,
    put: &mut dyn FnMut(usize, usize, char, Style),
) {
    if width < 8 || height < 3 {
        return;
    }
    let target = ms / S::TICK_MS;
    store.with_borrow_mut(|sims| {
        let same = |s: &S| s.key() == (seed, width);
        let stale = |s: &S| s.tick() > target || target - s.tick() > MAX_CATCH_UP;
        sims.retain(|s| !(same(s) && stale(s)));
        let index = match sims.iter().position(same) {
            Some(i) => i,
            None => {
                if sims.len() >= KEPT {
                    sims.remove(0);
                }
                sims.push(S::new(seed, width, height, target));
                sims.len() - 1
            }
        };
        let sim = &mut sims[index];
        while sim.tick() < target {
            sim.step();
        }
        sim.draw(ms, height, put);
    });
}
