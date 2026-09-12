//! An arcade arena for the sky above the commit scene: light cycles out of
//! GLTron and a snake out of Snake, sharing one grid. The cycles ride at a
//! steady pace, turn only at right angles and leave solid trails; the snake
//! hunts food and grows. Everything is an obstacle to everything else, so
//! collisions are checked before a move and no two things ever share a cell.
//! A crash blooms into an explosion, the wreck is cleared and someone new
//! rides in when the field is thin. The simulation is deterministic from its
//! seed and runs on a fixed tick, so any frame is the same picture whoever
//! draws it; explosions and food glow on the millisecond clock between ticks.

use std::cell::RefCell;
use std::collections::VecDeque;

use ratatui::style::{Color, Modifier, Style};

mod draw;
mod motion;
pub use draw::frame;

/// Milliseconds between simulation steps. A cycle moving sideways covers
/// one cell a tick; one moving up or down covers one every other tick, as a
/// cell is twice as tall as it is wide and the speed on screen should not
/// depend on the heading.
pub const TICK_MS: u64 = 70;
/// Ticks an explosion takes to expand and go out.
const EXPLOSION_TICKS: u64 = 9;
/// Ticks a dead rider's trail stays up before it fades, and how long the
/// fade takes. GLTron drops a wreck's trail at once; here it lingers as an
/// obstacle for a while, so the arena does not fill up over a long wait.
const TRAIL_LINGER_TICKS: u64 = 90;
const TRAIL_FADE_TICKS: u64 = 25;
/// Ticks before a replacement rides in, and before a dead snake returns.
const BIKE_RESPAWN_TICKS: u64 = 24;
const SNAKE_RESPAWN_TICKS: u64 = 20;
/// The oldest trail is retired when less than this share of the arena is
/// still open, so a long wait does not silt up.
const CROWDED: f32 = 0.4;
/// Cells a rider must have reachable before it is safe to spawn there.
const SPAWN_ROOM: usize = 40;
/// How far the flood fill behind a rider's decision reaches. Enough to tell
/// a dead end from open ground; not so far that every option looks alike.
const LOOKAHEAD_ROOM: usize = 160;
const LOOKAHEAD_STRAIGHT: usize = 14;
/// Cells the snake grows per meal, and the length it stops growing at: a
/// snake long enough to wall off a shallow arena spoils the round for
/// everyone, itself included.
const MEAL: u32 = 2;
const SNAKE_START_LEN: usize = 4;
const SNAKE_MAX_LEN: usize = 32;

const BIKE_COLORS: [Color; 6] = [
    Color::Rgb(0, 255, 255),
    Color::Rgb(255, 70, 255),
    Color::Rgb(255, 240, 60),
    Color::Rgb(255, 140, 20),
    Color::Rgb(140, 255, 60),
    Color::Rgb(90, 150, 255),
];
const SNAKE_HEAD: Color = Color::Rgb(200, 255, 200);
const SNAKE_BODY: Color = Color::Rgb(60, 235, 110);
const FOOD_GLYPHS: [char; 3] = ['*', '$', '+'];
const FOOD_COLORS: [Color; 3] = [
    Color::Rgb(255, 235, 90),
    Color::Rgb(255, 120, 200),
    Color::Rgb(120, 255, 240),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    fn left(self) -> Dir {
        match self {
            Dir::Up => Dir::Left,
            Dir::Left => Dir::Down,
            Dir::Down => Dir::Right,
            Dir::Right => Dir::Up,
        }
    }

    fn right(self) -> Dir {
        self.left().left().left()
    }

    fn reverse(self) -> Dir {
        self.left().left()
    }

    fn horizontal(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }

    fn bike_glyph(self) -> char {
        match self {
            Dir::Up => '^',
            Dir::Down => 'v',
            Dir::Left => '<',
            Dir::Right => '>',
        }
    }

    /// The trail glyph left where a rider heading `self` turns to `next`.
    fn trail_glyph(self, next: Dir) -> char {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Empty,
    /// Part of a light trail; `owner` says whose, for its colour and for
    /// clearing it when the rider is gone.
    Trail {
        owner: u32,
        glyph: char,
    },
    Bike(u32),
    Snake(usize),
    Food(u8),
}

#[derive(Debug, Clone)]
struct Bike {
    /// Identifies the rider's trail; a new one is issued when the arena is
    /// cleared so the old trail can fade under it.
    owner: u32,
    color: usize,
    x: usize,
    y: usize,
    dir: Dir,
}

#[derive(Debug, Clone)]
struct Snake {
    /// Head first.
    body: VecDeque<(usize, usize)>,
    dir: Dir,
    /// Cells still to grow: the tail stays put this many moves.
    grow: u32,
}

#[derive(Debug, Clone)]
struct Explosion {
    x: usize,
    y: usize,
    start: u64,
    /// Rows the blast reaches at its widest.
    radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Bike,
    Snake,
}

/// One arena and everything in it.
pub struct Arena {
    pub seed: usize,
    pub width: usize,
    pub height: usize,
    pub tick: u64,
    cells: Vec<Cell>,
    bikes: Vec<Bike>,
    snakes: Vec<Option<Snake>>,
    explosions: Vec<Explosion>,
    /// Trails whose riders are gone, with the tick they died.
    dead_trails: Vec<(u32, u64)>,
    /// Riders and snakes due to come back, and when.
    respawns: Vec<(Kind, u64)>,
    next_owner: u32,
    /// The tick a trail was last retired for crowding: one goes at a time,
    /// so the arena thins out rather than being wiped.
    last_retire: u64,
    rng: u64,
    food_target: usize,
    bike_target: usize,
}

impl Arena {
    pub fn new(seed: usize, width: usize, height: usize, tick: u64) -> Self {
        let area = width * height;
        let mut arena = Self {
            seed,
            width,
            height,
            tick,
            cells: vec![Cell::Empty; area],
            bikes: Vec::new(),
            snakes: vec![None; (area / 2_500).clamp(1, 2)],
            explosions: Vec::new(),
            dead_trails: Vec::new(),
            respawns: Vec::new(),
            next_owner: 0,
            last_retire: 0,
            rng: (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            food_target: (area / 250).clamp(3, 12),
            bike_target: (area / 600).clamp(2, 5),
        };
        for _ in 0..arena.bike_target {
            arena.spawn_bike();
        }
        for i in 0..arena.snakes.len() {
            arena.spawn_snake(i);
        }
        arena.replenish_food();
        arena
    }

    fn rand(&mut self) -> u64 {
        // xorshift64*: plenty for scattering spawns and food.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.rand() % n.max(1) as u64) as usize
    }

    fn at(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize, cell: Cell) {
        self.cells[y * self.width + x] = cell;
    }

    /// The cell one step from (`x`, `y`) in `dir`, if it is in the arena.
    fn step_from(&self, x: usize, y: usize, dir: Dir) -> Option<(usize, usize)> {
        let (dx, dy) = dir.delta();
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        (nx >= 0 && ny >= 0 && (nx as usize) < self.width && (ny as usize) < self.height)
            .then_some((nx as usize, ny as usize))
    }

    /// Whether a rider or a snake may move into a cell: nothing solid there.
    /// Food is not solid; a snake eats it and a cycle runs it over.
    fn passable(&self, x: usize, y: usize) -> bool {
        matches!(self.at(x, y), Cell::Empty | Cell::Food(_))
    }

    /// How many cells can be reached from (`x`, `y`) without crossing
    /// anything solid, counting up to `limit`.
    fn room(&self, x: usize, y: usize, limit: usize) -> usize {
        if !self.passable(x, y) {
            return 0;
        }
        let mut seen = vec![false; self.cells.len()];
        let mut queue = VecDeque::from([(x, y)]);
        seen[y * self.width + x] = true;
        let mut count = 0;
        while let Some((cx, cy)) = queue.pop_front() {
            count += 1;
            if count >= limit {
                break;
            }
            for dir in Dir::ALL {
                if let Some((nx, ny)) = self.step_from(cx, cy, dir) {
                    let i = ny * self.width + nx;
                    if !seen[i] && self.passable(nx, ny) {
                        seen[i] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
        }
        count
    }

    /// Open cells straight ahead from (`x`, `y`) in `dir`, up to a limit.
    fn straight(&self, x: usize, y: usize, dir: Dir) -> usize {
        let mut n = 0;
        let (mut cx, mut cy) = (x, y);
        while n < LOOKAHEAD_STRAIGHT {
            match self.step_from(cx, cy, dir) {
                Some((nx, ny)) if self.passable(nx, ny) => {
                    (cx, cy) = (nx, ny);
                    n += 1;
                }
                _ => break,
            }
        }
        n
    }

    /// Whether any explosion is still burning over a cell.
    fn burning(&self, x: usize, y: usize) -> bool {
        self.explosions.iter().any(|e| {
            let dx = (x as f32 - e.x as f32) / 2.0;
            let dy = y as f32 - e.y as f32;
            (dx * dx + dy * dy).sqrt() <= e.radius + 0.5
        })
    }

    /// A random open cell with room around it, for something to spawn in.
    fn open_spot(&mut self) -> Option<(usize, usize)> {
        for _ in 0..40 {
            let x = self.below(self.width);
            let y = self.below(self.height);
            if self.at(x, y) == Cell::Empty
                && !self.burning(x, y)
                && self.room(x, y, SPAWN_ROOM) >= SPAWN_ROOM.min(self.cells.len() / 4)
            {
                return Some((x, y));
            }
        }
        None
    }

    /// One tick of the world.
    pub fn step(&mut self) {
        self.tick += 1;
        let tick = self.tick;
        self.explosions
            .retain(|e| tick.saturating_sub(e.start) <= EXPLOSION_TICKS);
        self.clear_wrecks();
        self.respawn_due();
        self.move_bikes();
        self.move_snakes();
        self.replenish_food();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seed: usize, w: usize, h: usize, ticks: u64) -> Arena {
        let mut arena = Arena::new(seed, w, h, 0);
        for _ in 0..ticks {
            arena.step();
        }
        arena
    }

    /// Every solid thing on the grid is accounted for by exactly one
    /// entity, and everything the entities claim is on the grid.
    fn consistent(arena: &Arena) {
        for bike in &arena.bikes {
            assert_eq!(
                arena.at(bike.x, bike.y),
                Cell::Bike(bike.owner),
                "bike {} is where the grid says",
                bike.owner
            );
        }
        for (id, snake) in arena.snakes.iter().enumerate() {
            let Some(snake) = snake else { continue };
            for &(x, y) in &snake.body {
                assert_eq!(arena.at(x, y), Cell::Snake(id), "snake {id} owns its body");
            }
            let mut sorted: Vec<_> = snake.body.iter().collect();
            sorted.sort();
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                snake.body.len(),
                "a snake never overlaps itself"
            );
        }
        let claimed = arena.bikes.len()
            + arena
                .snakes
                .iter()
                .flatten()
                .map(|s| s.body.len())
                .sum::<usize>();
        let on_grid = arena
            .cells
            .iter()
            .filter(|c| matches!(c, Cell::Bike(_) | Cell::Snake(_)))
            .count();
        assert_eq!(claimed, on_grid, "no cell holds two things");
    }

    #[test]
    fn nothing_ever_shares_a_cell_and_the_world_stays_populated() {
        for seed in 0..4 {
            let mut arena = Arena::new(seed, 120, 24, 0);
            let mut crashes = 0;
            for _ in 0..1_500 {
                let before = arena.bikes.len();
                arena.step();
                consistent(&arena);
                crashes += before.saturating_sub(arena.bikes.len());
                assert!(
                    !arena.bikes.is_empty() || !arena.respawns.is_empty(),
                    "someone is always riding or about to"
                );
            }
            assert!(crashes > 0, "seed {seed}: riders do crash over 100 seconds");
            assert!(!arena.bikes.is_empty(), "seed {seed}: riders come back");
            assert!(
                arena.cells.iter().any(|c| matches!(c, Cell::Trail { .. })),
                "trails are left behind"
            );
        }
    }

    #[test]
    fn a_crash_leaves_an_explosion_and_the_trail_behind() {
        let mut arena = Arena::new(1, 60, 12, 0);
        let mut crashed_at = None;
        for _ in 0..600 {
            let before: Vec<(usize, usize)> = arena.bikes.iter().map(|b| (b.x, b.y)).collect();
            arena.step();
            if arena.bikes.len() < before.len() {
                let tick = arena.tick;
                crashed_at = arena
                    .explosions
                    .iter()
                    .find(|e| e.start == tick && before.contains(&(e.x, e.y)))
                    .map(|e| (e.x, e.y));
                if crashed_at.is_some() {
                    break;
                }
            }
        }
        let (x, y) = crashed_at.expect("a rider crashes within 600 ticks");
        assert!(
            matches!(arena.at(x, y), Cell::Trail { .. }),
            "the trail stays where the rider fell"
        );
        let mut drawn = Vec::new();
        arena.draw(arena.tick * TICK_MS + 30, &mut |px, py, c, _| {
            drawn.push((px, py, c))
        });
        assert!(
            drawn.iter().any(|&(px, py, c)| px.abs_diff(x) <= 4
                && py.abs_diff(y) <= 2
                && "*+x.#%@".contains(c)),
            "explosion glyphs are drawn round the wreck"
        );
    }

    #[test]
    fn the_snake_eats_and_grows_and_food_is_replaced() {
        let mut arena = Arena::new(2, 100, 20, 0);
        let start_len = arena.snakes[0].as_ref().map(|s| s.body.len()).unwrap();
        let mut longest = start_len;
        for _ in 0..1_000 {
            arena.step();
            if let Some(s) = arena.snakes[0].as_ref() {
                longest = longest.max(s.body.len());
            }
            let food = arena
                .cells
                .iter()
                .filter(|c| matches!(c, Cell::Food(_)))
                .count();
            assert!(food >= 1, "there is always food about");
        }
        assert!(longest > start_len, "the snake found a meal in 70 seconds");
    }

    #[test]
    fn food_never_spawns_on_anything() {
        let arena = run(3, 90, 15, 300);
        for (i, cell) in arena.cells.iter().enumerate() {
            if matches!(cell, Cell::Food(_)) {
                let (x, y) = (i % arena.width, i / arena.width);
                assert!(!arena.bikes.iter().any(|b| (b.x, b.y) == (x, y)));
                assert!(
                    !arena
                        .snakes
                        .iter()
                        .flatten()
                        .any(|s| s.body.contains(&(x, y)))
                );
            }
        }
    }

    #[test]
    fn the_same_seed_and_clock_draw_the_same_frame() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        frame(7, 80, 14, 3_000, &mut |x, y, c, s| a.push((x, y, c, s)));
        frame(7, 80, 14, 3_000, &mut |x, y, c, s| b.push((x, y, c, s)));
        assert_eq!(a, b);
        let mut later = Vec::new();
        frame(7, 80, 14, 3_000 + TICK_MS * 3, &mut |x, y, c, s| {
            later.push((x, y, c, s))
        });
        assert_ne!(a, later, "three ticks on, things have moved");
        assert!(!a.is_empty());
    }
}
