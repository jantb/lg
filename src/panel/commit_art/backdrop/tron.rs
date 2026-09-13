//! A round of GLTron for the sky above the commit scene. Light cycles ride
//! at a steady pace, turn only at right angles and leave solid walls of
//! light behind them; a rider that hits a wall, the edge or another rider
//! crashes. As in the game, a crashed rider's wall goes with it, fading out
//! over a moment, so the survivors get the room back; and when one rider is
//! left it has won the round, everything is cleared and the next round
//! starts with a full field. The simulation is deterministic from its seed
//! and runs on a fixed tick, so any frame is the same picture whoever draws
//! it; explosions burn on the millisecond clock between ticks.

use std::cell::RefCell;

use super::grid::{self, Dir, Explosion, Rng, Sim};
use super::*;

/// Milliseconds between simulation steps. A cycle moving sideways covers
/// one cell a tick; one moving up or down covers one every other tick, as a
/// cell is twice as tall as it is wide and the speed on screen should not
/// depend on the heading.
pub(super) const TICK_MS: u64 = 70;
/// Ticks a crashed rider's wall takes to fade out. In GLTron the wall goes
/// when its rider does; the fade is the game's own effect, and while it
/// fades the cells are already open.
const WALL_FADE_TICKS: u64 = 8;
/// Ticks between the round being decided and the next one starting.
const ROUND_OVER_TICKS: u64 = 28;
/// How far the flood fill behind a rider's decision reaches. Enough to tell
/// a dead end from open ground; not so far that every option looks alike.
const LOOKAHEAD_ROOM: usize = 160;
const LOOKAHEAD_STRAIGHT: usize = 14;

/// One colour per rider; no two in a round share one.
const COLORS: [Color; 6] = [
    Color::Rgb(0, 255, 255),
    Color::Rgb(255, 70, 255),
    Color::Rgb(255, 240, 60),
    Color::Rgb(255, 140, 20),
    Color::Rgb(140, 255, 60),
    Color::Rgb(90, 150, 255),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Empty,
    /// Part of a rider's wall; `rider` says whose, for its colour and for
    /// clearing it when the rider crashes.
    Wall {
        rider: usize,
        glyph: char,
    },
    Bike(usize),
}

#[derive(Debug, Clone)]
struct Bike {
    color: usize,
    x: usize,
    y: usize,
    dir: Dir,
    alive: bool,
}

/// A cell of a wall that is fading out after its rider crashed.
#[derive(Debug, Clone)]
struct Ghost {
    x: usize,
    y: usize,
    glyph: char,
    color: usize,
    /// The tick it started fading.
    since: u64,
}

/// One arena and the round being ridden in it.
pub(super) struct Arena {
    seed: usize,
    width: usize,
    height: usize,
    tick: u64,
    cells: Vec<Cell>,
    /// The riders of this round, dead or alive; a rider's index is its
    /// identity on the grid.
    bikes: Vec<Bike>,
    ghosts: Vec<Ghost>,
    explosions: Vec<Explosion>,
    /// The tick the next round starts, once this one is decided.
    next_round: Option<u64>,
    rng: Rng,
    riders: usize,
}

impl Sim for Arena {
    const TICK_MS: u64 = TICK_MS;

    fn new(seed: usize, width: usize, height: usize, tick: u64) -> Self {
        let area = width * height;
        let mut arena = Self {
            seed,
            width,
            height,
            tick,
            cells: vec![Cell::Empty; area],
            bikes: Vec::new(),
            ghosts: Vec::new(),
            explosions: Vec::new(),
            next_round: None,
            rng: Rng::new(seed),
            riders: (area / 500).clamp(2, COLORS.len()),
        };
        arena.start_round();
        arena
    }

    fn key(&self) -> (usize, usize, usize) {
        (self.seed, self.width, self.height)
    }

    fn tick(&self) -> u64 {
        self.tick
    }

    /// One tick of the world.
    fn step(&mut self) {
        self.tick += 1;
        let tick = self.tick;
        self.explosions.retain(|e| !e.over(tick));
        self.ghosts
            .retain(|g| tick.saturating_sub(g.since) <= WALL_FADE_TICKS);
        if self.next_round.is_some_and(|at| tick >= at) {
            self.start_round();
        }
        self.move_bikes();
        let alive = self.bikes.iter().filter(|b| b.alive).count();
        let decided = alive == 0 || (alive == 1 && self.bikes.len() > 1);
        if decided && self.next_round.is_none() {
            self.next_round = Some(tick + ROUND_OVER_TICKS);
        }
    }

    fn draw(&self, ms: u64, put: &mut dyn FnMut(usize, usize, char, Style)) {
        for g in &self.ghosts {
            let into = self.tick.saturating_sub(g.since) as f32 / WALL_FADE_TICKS as f32;
            let left = (1.0 - into).clamp(0.0, 1.0) * 0.6;
            put(
                g.x,
                g.y,
                g.glyph,
                Style::default().fg(dim(COLORS[g.color], left)),
            );
        }
        for y in 0..self.height {
            for x in 0..self.width {
                match self.at(x, y) {
                    Cell::Empty => {}
                    Cell::Wall { rider, glyph } => {
                        let color = COLORS[self.bikes[rider].color];
                        put(x, y, glyph, Style::default().fg(dim(color, 0.7)));
                    }
                    Cell::Bike(rider) => {
                        let bike = &self.bikes[rider];
                        put(
                            x,
                            y,
                            bike.dir.arrow(),
                            Style::default()
                                .fg(COLORS[bike.color])
                                .add_modifier(Modifier::BOLD),
                        );
                    }
                }
            }
        }
        for e in &self.explosions {
            e.draw(ms, TICK_MS, self.width, self.height, put);
        }
    }
}

impl Arena {
    fn at(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize, cell: Cell) {
        self.cells[y * self.width + x] = cell;
    }

    fn step_from(&self, x: usize, y: usize, dir: Dir) -> Option<(usize, usize)> {
        grid::step_from(self.width, self.height, x, y, dir)
    }

    fn passable(&self, x: usize, y: usize) -> bool {
        self.at(x, y) == Cell::Empty
    }

    fn room(&self, x: usize, y: usize) -> usize {
        grid::room(
            self.width,
            self.height,
            |x, y| self.passable(x, y),
            x,
            y,
            LOOKAHEAD_ROOM,
        )
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

    /// Clear the grid, the last round's walls fading where they stood, and
    /// send in a full field of riders. They start spread across the arena
    /// facing the middle, each in its own colour, as the game lines its
    /// players up.
    fn start_round(&mut self) {
        let tick = self.tick;
        for i in 0..self.cells.len() {
            if let Cell::Wall { rider, glyph } = self.cells[i] {
                self.ghosts.push(Ghost {
                    x: i % self.width,
                    y: i / self.width,
                    glyph,
                    color: self.bikes[rider].color,
                    since: tick,
                });
            }
        }
        self.cells.fill(Cell::Empty);
        self.bikes.clear();
        self.next_round = None;
        let mut colors: Vec<usize> = (0..COLORS.len()).collect();
        for i in (1..colors.len()).rev() {
            let j = self.rng.below(i + 1);
            colors.swap(i, j);
        }
        let n = self.riders;
        for (i, &color) in colors.iter().enumerate().take(n) {
            let x = self.width * (i + 1) / (n + 1);
            let lane = self.height / 4 + self.rng.below((self.height / 2).max(1));
            let y = if i % 2 == 0 {
                lane.min(self.height / 2)
            } else {
                lane.max(self.height / 2)
            }
            .min(self.height - 1);
            let dir = if x * 2 < self.width {
                Dir::Right
            } else {
                Dir::Left
            };
            self.bikes.push(Bike {
                color,
                x,
                y,
                dir,
                alive: true,
            });
            self.set(x, y, Cell::Bike(i));
        }
    }

    /// Where a rider wants to go next: ahead, or a turn, whichever leaves
    /// the most road. Straight on is preferred when it is about as good, so
    /// the walls run long and geometric, with the odd turn for variety;
    /// and when a rival is close ahead, the way that cuts across its path
    /// scores extra, to wall it in.
    fn steer(&mut self, i: usize) -> Dir {
        let bike = self.bikes[i].clone();
        let whim = self.rng.below(8) == 0;
        let rival = self
            .bikes
            .iter()
            .enumerate()
            .filter(|&(j, o)| j != i && o.alive)
            .map(|(_, o)| {
                let (dx, dy) = o.dir.delta();
                let ahead = (o.x as i32 + dx * 3, o.y as i32 + dy * 3);
                let dist = (ahead.0 - bike.x as i32).abs() + (ahead.1 - bike.y as i32).abs();
                (dist, ahead)
            })
            .filter(|&(dist, _)| dist < 12)
            .min_by_key(|&(dist, _)| dist);
        let mut best = (i64::MIN, bike.dir);
        for dir in [bike.dir, bike.dir.left(), bike.dir.right()] {
            let Some((nx, ny)) = self.step_from(bike.x, bike.y, dir) else {
                continue;
            };
            if !self.passable(nx, ny) {
                continue;
            }
            let room = self.room(nx, ny) as i64;
            let straight = self.straight(nx, ny, dir) as i64;
            let mut score = room * 2 + straight;
            if dir == bike.dir {
                score += if whim { -6 } else { 5 };
            }
            if let Some((dist, (ax, ay))) = rival {
                let after = (ax - nx as i32).abs() + (ay - ny as i32).abs();
                if after < dist {
                    score += 5;
                }
            }
            if score > best.0 {
                best = (score, dir);
            }
        }
        best.1
    }

    fn move_bikes(&mut self) {
        for i in 0..self.bikes.len() {
            if !self.bikes[i].alive {
                continue;
            }
            let dir = self.steer(i);
            if !dir.horizontal() && self.tick % 2 == 1 {
                // Vertical moves only every other tick; see TICK_MS. A
                // rider may still swing onto a horizontal now.
                continue;
            }
            self.ride(i, dir);
        }
    }

    /// Move rider `i` one cell in `dir`, or crash it if the cell is taken.
    fn ride(&mut self, i: usize, dir: Dir) {
        let Bike { x, y, dir: was, .. } = self.bikes[i];
        let next = self
            .step_from(x, y, dir)
            .filter(|&(nx, ny)| self.passable(nx, ny));
        match next {
            Some((nx, ny)) => {
                self.set(
                    x,
                    y,
                    Cell::Wall {
                        rider: i,
                        glyph: was.corner(dir),
                    },
                );
                self.set(nx, ny, Cell::Bike(i));
                let bike = &mut self.bikes[i];
                bike.x = nx;
                bike.y = ny;
                bike.dir = dir;
            }
            None => self.crash(i),
        }
    }

    /// Rider `i` hits something: it blows up where it stands and its wall
    /// goes with it, every cell fading from where it was.
    fn crash(&mut self, i: usize) {
        let tick = self.tick;
        let Bike { x, y, color, .. } = self.bikes[i];
        self.bikes[i].alive = false;
        self.explosions.push(Explosion {
            x,
            y,
            start: tick,
            radius: 2.5,
        });
        for c in 0..self.cells.len() {
            let glyph = match self.cells[c] {
                Cell::Wall { rider, glyph } if rider == i => glyph,
                Cell::Bike(rider) if rider == i => '+',
                _ => continue,
            };
            self.cells[c] = Cell::Empty;
            self.ghosts.push(Ghost {
                x: c % self.width,
                y: c / self.width,
                glyph,
                color,
                since: tick,
            });
        }
    }
}

thread_local! {
    static ARENAS: RefCell<Vec<Arena>> = const { RefCell::new(Vec::new()) };
}

/// Draw the round for `seed`, `width` by `height`, as it stands at `ms`.
pub(super) fn frame(
    seed: usize,
    width: usize,
    height: usize,
    ms: u64,
    put: &mut dyn FnMut(usize, usize, char, Style),
) {
    grid::frame(&ARENAS, seed, width, height, ms, put)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alive(arena: &Arena) -> usize {
        arena.bikes.iter().filter(|b| b.alive).count()
    }

    fn walls_of(arena: &Arena, rider: usize) -> usize {
        arena
            .cells
            .iter()
            .filter(|c| matches!(c, Cell::Wall { rider: r, .. } if *r == rider))
            .count()
    }

    /// Every living rider is on the grid where it says it is, every rider
    /// on the grid is alive, and no wall belongs to a crashed rider.
    fn consistent(arena: &Arena) {
        for (i, bike) in arena.bikes.iter().enumerate() {
            if bike.alive {
                assert_eq!(
                    arena.at(bike.x, bike.y),
                    Cell::Bike(i),
                    "rider {i} is where the grid says"
                );
            } else {
                assert_eq!(walls_of(arena, i), 0, "crashed rider {i} has no wall left");
            }
        }
        let on_grid = arena
            .cells
            .iter()
            .filter(|c| matches!(c, Cell::Bike(_)))
            .count();
        assert_eq!(alive(arena), on_grid, "no cell holds two riders");
    }

    #[test]
    fn a_crash_takes_the_wall_with_it_and_leaves_an_explosion() {
        let mut arena = Arena::new(1, 80, 16, 0);
        let mut crashed = None;
        for _ in 0..1_000 {
            let before: Vec<bool> = arena.bikes.iter().map(|b| b.alive).collect();
            arena.step();
            consistent(&arena);
            if let Some(i) = (0..arena.bikes.len()).find(|&i| before[i] && !arena.bikes[i].alive) {
                crashed = Some(i);
                break;
            }
        }
        let i = crashed.expect("a rider crashes within 70 seconds");
        let (x, y) = (arena.bikes[i].x, arena.bikes[i].y);
        assert_eq!(walls_of(&arena, i), 0, "the fallen rider's wall is gone");
        assert!(
            arena
                .bikes
                .iter()
                .enumerate()
                .any(|(j, b)| b.alive && walls_of(&arena, j) > 0),
            "the survivors' walls still stand"
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
    fn the_last_rider_wins_and_a_fresh_round_follows() {
        for seed in 0..4 {
            let mut arena = Arena::new(seed, 100, 20, 0);
            let field = arena.bikes.len();
            assert!(field >= 2, "there is someone to race");
            let mut decided = None;
            for _ in 0..4_000 {
                arena.step();
                consistent(&arena);
                if alive(&arena) <= 1 {
                    decided = Some(arena.tick);
                    break;
                }
            }
            let decided = decided.expect("seed {seed}: a round is decided within five minutes");
            let mut restarted = false;
            for _ in 0..ROUND_OVER_TICKS + 2 {
                arena.step();
                consistent(&arena);
                if alive(&arena) == field && arena.tick > decided {
                    restarted = true;
                    break;
                }
            }
            assert!(
                restarted,
                "seed {seed}: a full field rides in for the next round"
            );
            let walls = arena
                .cells
                .iter()
                .filter(|c| matches!(c, Cell::Wall { .. }))
                .count();
            assert!(
                walls <= field * 2,
                "seed {seed}: the new round starts on a clear grid"
            );
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
