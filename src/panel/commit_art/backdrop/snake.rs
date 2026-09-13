//! Snake for the sky above the commit scene. A snake hunts the food
//! scattered about the grid, growing with every meal, and dies if it runs
//! into the edge or itself, coming apart segment by segment before a new
//! one sets out. A big enough sky gets two snakes, and then each is an
//! obstacle to the other. The simulation is deterministic from its seed and
//! runs on a fixed tick, so any frame is the same picture whoever draws it;
//! food glows and explosions burn on the millisecond clock between ticks.

use std::cell::RefCell;
use std::collections::VecDeque;

use super::grid::{self, Dir, Explosion, Rng, Sim};
use super::*;

/// Milliseconds between simulation steps. A snake moving sideways covers
/// one cell a tick; one moving up or down covers one every other tick, as a
/// cell is twice as tall as it is wide and the speed on screen should not
/// depend on the heading.
pub(super) const TICK_MS: u64 = 80;
/// Ticks before a dead snake is replaced.
const RESPAWN_TICKS: u64 = 20;
/// Cells a snake must have reachable before it is safe to spawn there.
const SPAWN_ROOM: usize = 40;
/// How far the flood fill behind a snake's decision reaches.
const LOOKAHEAD_ROOM: usize = 160;
/// Cells a snake grows per meal, its starting length, and the most it may
/// grow to: a snake long enough to wall off the sky spoils its own game.
const MEAL: u32 = 2;
const START_LEN: usize = 4;
const MAX_LEN: usize = 60;

const HEAD: Color = Color::Rgb(200, 255, 200);
const BODY: Color = Color::Rgb(60, 235, 110);
const FOOD_GLYPHS: [char; 3] = ['*', '$', '+'];
const FOOD_COLORS: [Color; 3] = [
    Color::Rgb(255, 235, 90),
    Color::Rgb(255, 120, 200),
    Color::Rgb(120, 255, 240),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Empty,
    Snake(usize),
    Food(u8),
}

#[derive(Debug, Clone)]
struct Snake {
    /// Head first.
    body: VecDeque<(usize, usize)>,
    dir: Dir,
    /// Cells still to grow: the tail stays put this many moves.
    grow: u32,
}

/// One pit and everything in it.
pub(super) struct Pit {
    seed: usize,
    width: usize,
    height: usize,
    tick: u64,
    cells: Vec<Cell>,
    snakes: Vec<Option<Snake>>,
    explosions: Vec<Explosion>,
    /// Ticks at which a dead snake comes back.
    respawns: Vec<u64>,
    rng: Rng,
    food_target: usize,
}

impl Sim for Pit {
    const TICK_MS: u64 = TICK_MS;

    fn new(seed: usize, width: usize, height: usize, tick: u64) -> Self {
        let area = width * height;
        let mut pit = Self {
            seed,
            width,
            height,
            tick,
            cells: vec![Cell::Empty; area],
            snakes: vec![None; (area / 1_500).clamp(1, 2)],
            explosions: Vec::new(),
            respawns: Vec::new(),
            rng: Rng::new(seed),
            food_target: (area / 250).clamp(3, 12),
        };
        for i in 0..pit.snakes.len() {
            pit.spawn(i);
        }
        pit.replenish_food();
        pit
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
        self.respawn_due();
        for id in 0..self.snakes.len() {
            let Some(dir) = self.steer(id) else {
                continue;
            };
            if !dir.horizontal() && self.tick % 2 == 1 {
                // Vertical moves are half as frequent; see TICK_MS.
                continue;
            }
            self.slither(id, dir);
        }
        self.replenish_food();
    }

    fn draw(&self, ms: u64, put: &mut dyn FnMut(usize, usize, char, Style)) {
        let t = ms as f32 / 1000.0;
        for y in 0..self.height {
            for x in 0..self.width {
                match self.at(x, y) {
                    Cell::Empty => {}
                    Cell::Snake(id) => {
                        let Some(snake) = self.snakes[id].as_ref() else {
                            continue;
                        };
                        let index = snake
                            .body
                            .iter()
                            .position(|&p| p == (x, y))
                            .unwrap_or(snake.body.len());
                        let (glyph, style) = if index == 0 {
                            ('@', Style::default().fg(HEAD).add_modifier(Modifier::BOLD))
                        } else {
                            let fade = 1.0 - 0.5 * (index as f32 / snake.body.len().max(1) as f32);
                            let glyph = if index % 2 == 0 { 'o' } else { '=' };
                            (glyph, Style::default().fg(dim(BODY, fade)))
                        };
                        put(x, y, glyph, style);
                    }
                    Cell::Food(kind) => {
                        let kind = kind as usize % FOOD_GLYPHS.len();
                        let phase = (x * 7 + y * 13) as f32;
                        let pulse = 0.65 + 0.35 * (t * 4.0 + phase).sin();
                        let mut style = Style::default().fg(dim(FOOD_COLORS[kind], pulse));
                        if pulse > 0.9 {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        put(x, y, FOOD_GLYPHS[kind], style);
                    }
                }
            }
        }
        for e in &self.explosions {
            e.draw(ms, TICK_MS, self.width, self.height, put);
        }
    }
}

impl Pit {
    fn at(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize, cell: Cell) {
        self.cells[y * self.width + x] = cell;
    }

    fn step_from(&self, x: usize, y: usize, dir: Dir) -> Option<(usize, usize)> {
        grid::step_from(self.width, self.height, x, y, dir)
    }

    /// Whether a snake may move into a cell: food is eaten, a body is hit.
    fn passable(&self, x: usize, y: usize) -> bool {
        matches!(self.at(x, y), Cell::Empty | Cell::Food(_))
    }

    fn room(&self, x: usize, y: usize, limit: usize) -> usize {
        grid::room(
            self.width,
            self.height,
            |x, y| self.passable(x, y),
            x,
            y,
            limit,
        )
    }

    fn burning(&self, x: usize, y: usize) -> bool {
        grid::burning(&self.explosions, x, y)
    }

    /// A random open cell with room around it, for a snake to start in.
    fn open_spot(&mut self) -> Option<(usize, usize)> {
        for _ in 0..40 {
            let x = self.rng.below(self.width);
            let y = self.rng.below(self.height);
            if self.at(x, y) == Cell::Empty
                && !self.burning(x, y)
                && self.room(x, y, SPAWN_ROOM) >= SPAWN_ROOM.min(self.cells.len() / 4)
            {
                return Some((x, y));
            }
        }
        None
    }

    fn spawn(&mut self, id: usize) {
        for _ in 0..20 {
            let Some((x, y)) = self.open_spot() else {
                return;
            };
            // The body trails behind the head, so the spot needs a straight
            // stretch behind it; the snake sets off away from its tail.
            let dir = Dir::ALL[self.rng.below(4)];
            let mut body = VecDeque::from([(x, y)]);
            let (mut cx, mut cy) = (x, y);
            while body.len() < START_LEN {
                match self.step_from(cx, cy, dir.reverse()) {
                    Some((nx, ny)) if self.at(nx, ny) == Cell::Empty => {
                        (cx, cy) = (nx, ny);
                        body.push_back((nx, ny));
                    }
                    _ => break,
                }
            }
            if body.len() < START_LEN {
                continue;
            }
            for &(bx, by) in &body {
                self.set(bx, by, Cell::Snake(id));
            }
            self.snakes[id] = Some(Snake { body, dir, grow: 0 });
            return;
        }
    }

    fn replenish_food(&mut self) {
        let have = self
            .cells
            .iter()
            .filter(|c| matches!(c, Cell::Food(_)))
            .count();
        for _ in have..self.food_target {
            for _ in 0..20 {
                let x = self.rng.below(self.width);
                let y = self.rng.below(self.height);
                if self.at(x, y) == Cell::Empty && !self.burning(x, y) {
                    let kind = self.rng.below(FOOD_GLYPHS.len()) as u8;
                    self.set(x, y, Cell::Food(kind));
                    break;
                }
            }
        }
    }

    /// The first step of the shortest open path from the snake's head to
    /// any food, if there is one.
    fn path_to_food(&self, id: usize) -> Option<Dir> {
        let snake = self.snakes[id].as_ref()?;
        let &(hx, hy) = snake.body.front()?;
        let mut first: Vec<Option<Dir>> = vec![None; self.cells.len()];
        let mut seen = vec![false; self.cells.len()];
        let mut queue = VecDeque::new();
        seen[hy * self.width + hx] = true;
        for dir in Dir::ALL {
            if dir == snake.dir.reverse() {
                continue;
            }
            if let Some((nx, ny)) = self.step_from(hx, hy, dir)
                && self.passable(nx, ny)
                && !self.burning(nx, ny)
            {
                let i = ny * self.width + nx;
                seen[i] = true;
                first[i] = Some(dir);
                queue.push_back((nx, ny));
            }
        }
        while let Some((cx, cy)) = queue.pop_front() {
            let i = cy * self.width + cx;
            if matches!(self.at(cx, cy), Cell::Food(_)) {
                return first[i];
            }
            for dir in Dir::ALL {
                if let Some((nx, ny)) = self.step_from(cx, cy, dir) {
                    let j = ny * self.width + nx;
                    if !seen[j] && self.passable(nx, ny) {
                        seen[j] = true;
                        first[j] = first[i];
                        queue.push_back((nx, ny));
                    }
                }
            }
        }
        None
    }

    /// The cell the tail is about to leave, which the head may move into:
    /// a snake follows its own tail rather than dying on it, as it always
    /// has.
    fn leaving(snake: &Snake) -> Option<(usize, usize)> {
        (snake.grow == 0 && snake.body.len() > 1)
            .then(|| snake.body.back().copied())
            .flatten()
    }

    /// Where the snake goes next: towards food if it can get there, else
    /// wherever there is the most room to keep moving.
    fn steer(&self, id: usize) -> Option<Dir> {
        let snake = self.snakes[id].as_ref()?;
        let &(hx, hy) = snake.body.front()?;
        if let Some(dir) = self.path_to_food(id) {
            return Some(dir);
        }
        let leaving = Self::leaving(snake);
        [snake.dir, snake.dir.left(), snake.dir.right()]
            .into_iter()
            .filter_map(|dir| {
                let (nx, ny) = self.step_from(hx, hy, dir)?;
                (self.passable(nx, ny) || Some((nx, ny)) == leaving).then(|| {
                    let room = self.room(nx, ny, LOOKAHEAD_ROOM);
                    let burning = usize::from(self.burning(nx, ny)) * 4;
                    (room.saturating_sub(burning), dir)
                })
            })
            .max_by_key(|&(room, _)| room)
            .map(|(_, dir)| dir)
            .or(Some(snake.dir))
    }

    /// Move snake `id` one cell in `dir`: eat what is there, or die on it.
    fn slither(&mut self, id: usize, dir: Dir) {
        let Some(snake) = self.snakes[id].as_ref() else {
            return;
        };
        let Some(&(hx, hy)) = snake.body.front() else {
            return;
        };
        let leaving = Self::leaving(snake);
        let next = self
            .step_from(hx, hy, dir)
            .filter(|&(nx, ny)| self.passable(nx, ny) || Some((nx, ny)) == leaving);
        let Some((nx, ny)) = next else {
            self.kill(id);
            return;
        };
        let ate = matches!(self.at(nx, ny), Cell::Food(_));
        let longest = (self.width * self.height / 20).clamp(8, MAX_LEN);
        let Some(snake) = self.snakes[id].as_mut() else {
            return;
        };
        snake.dir = dir;
        snake.body.push_front((nx, ny));
        if ate && snake.body.len() < longest {
            snake.grow += MEAL;
        }
        let tail = if snake.grow > 0 {
            snake.grow -= 1;
            None
        } else {
            snake.body.pop_back()
        };
        // The tail is cleared before the head is drawn, so a head that has
        // just taken the tail's cell is not wiped out by it.
        if let Some((tx, ty)) = tail {
            self.set(tx, ty, Cell::Empty);
        }
        self.set(nx, ny, Cell::Snake(id));
    }

    /// The snake comes apart from the head back, each segment its own small
    /// burst, and its cells are freed for whatever comes next.
    fn kill(&mut self, id: usize) {
        let Some(snake) = self.snakes[id].take() else {
            return;
        };
        for (i, &(x, y)) in snake.body.iter().enumerate() {
            self.set(x, y, Cell::Empty);
            self.explosions.push(Explosion {
                x,
                y,
                start: self.tick + (i as u64 / 3).min(4),
                radius: if i == 0 { 1.5 } else { 0.6 },
            });
        }
        self.respawns.push(self.tick + RESPAWN_TICKS);
    }

    fn respawn_due(&mut self) {
        let tick = self.tick;
        let due = self.respawns.iter().filter(|&&at| at <= tick).count();
        self.respawns.retain(|&at| at > tick);
        for _ in 0..due {
            if let Some(id) = self.snakes.iter().position(Option::is_none) {
                self.spawn(id);
            }
        }
        // Keep the pit populated whatever happened: a spawn can fail for
        // want of room, and the population must not dwindle for it.
        let missing = self.snakes.iter().filter(|s| s.is_none()).count();
        if missing > self.respawns.len() {
            self.respawns.push(tick + RESPAWN_TICKS);
        }
    }
}

thread_local! {
    static PITS: RefCell<Vec<Pit>> = const { RefCell::new(Vec::new()) };
}

/// Draw the game for `seed`, `width` by `height`, as it stands at `ms`.
pub(super) fn frame(
    seed: usize,
    width: usize,
    height: usize,
    ms: u64,
    put: &mut dyn FnMut(usize, usize, char, Style),
) {
    grid::frame(&PITS, seed, width, height, ms, put)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every snake owns exactly the cells its body claims, and never
    /// overlaps itself or another.
    fn consistent(pit: &Pit) {
        for (id, snake) in pit.snakes.iter().enumerate() {
            let Some(snake) = snake else { continue };
            for &(x, y) in &snake.body {
                assert_eq!(pit.at(x, y), Cell::Snake(id), "snake {id} owns its body");
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
        let claimed: usize = pit.snakes.iter().flatten().map(|s| s.body.len()).sum();
        let on_grid = pit
            .cells
            .iter()
            .filter(|c| matches!(c, Cell::Snake(_)))
            .count();
        assert_eq!(claimed, on_grid, "no cell holds two things");
    }

    #[test]
    fn the_snake_eats_and_grows_and_food_is_replaced() {
        let mut pit = Pit::new(2, 100, 20, 0);
        let start_len = pit.snakes[0].as_ref().map(|s| s.body.len()).unwrap();
        let mut longest = start_len;
        for _ in 0..1_000 {
            pit.step();
            consistent(&pit);
            if let Some(s) = pit.snakes[0].as_ref() {
                longest = longest.max(s.body.len());
            }
            let food = pit
                .cells
                .iter()
                .filter(|c| matches!(c, Cell::Food(_)))
                .count();
            assert!(food >= 1, "there is always food about");
        }
        assert!(longest > start_len, "the snake found a meal in 80 seconds");
    }

    #[test]
    fn a_dead_snake_is_cleared_and_replaced() {
        for seed in 0..6 {
            let mut pit = Pit::new(seed, 60, 10, 0);
            let mut died = None;
            for _ in 0..3_000 {
                pit.step();
                consistent(&pit);
                if pit.snakes.iter().all(Option::is_none) {
                    died = Some(pit.tick);
                    break;
                }
            }
            let Some(died) = died else {
                // A snake that never dies is not wrong, only lucky.
                continue;
            };
            assert!(
                !pit.explosions.is_empty(),
                "seed {seed}: it went out with a bang"
            );
            for _ in 0..RESPAWN_TICKS * 3 {
                pit.step();
                consistent(&pit);
            }
            assert!(
                pit.snakes.iter().any(Option::is_some),
                "seed {seed}: a new snake sets out after the one that died at {died}"
            );
            return;
        }
        panic!("some snake dies within four minutes on a small grid");
    }

    #[test]
    fn food_never_spawns_on_a_snake() {
        let mut pit = Pit::new(3, 90, 15, 0);
        for _ in 0..300 {
            pit.step();
        }
        for (i, cell) in pit.cells.iter().enumerate() {
            if matches!(cell, Cell::Food(_)) {
                let (x, y) = (i % pit.width, i / pit.width);
                assert!(
                    !pit.snakes
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
