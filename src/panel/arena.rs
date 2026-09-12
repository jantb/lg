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

    fn spawn_bike(&mut self) {
        let Some((x, y)) = self.open_spot() else {
            return;
        };
        // Ride off the way with the most open road.
        let dir = Dir::ALL
            .into_iter()
            .max_by_key(|&d| self.straight(x, y, d) * 4 + self.below(4))
            .unwrap_or(Dir::Right);
        let owner = self.next_owner;
        self.next_owner += 1;
        let color = self.below(BIKE_COLORS.len());
        self.bikes.push(Bike {
            owner,
            color,
            x,
            y,
            dir,
        });
        self.set(x, y, Cell::Bike(owner));
    }

    fn spawn_snake(&mut self, id: usize) {
        for _ in 0..20 {
            let Some((x, y)) = self.open_spot() else {
                return;
            };
            // The body trails behind the head, so the spot needs a straight
            // stretch behind it; the snake sets off away from its tail.
            let dir = Dir::ALL[self.below(4)];
            let mut body = VecDeque::from([(x, y)]);
            let (mut cx, mut cy) = (x, y);
            while body.len() < SNAKE_START_LEN {
                match self.step_from(cx, cy, dir.reverse()) {
                    Some((nx, ny)) if self.at(nx, ny) == Cell::Empty => {
                        (cx, cy) = (nx, ny);
                        body.push_back((nx, ny));
                    }
                    _ => break,
                }
            }
            if body.len() < SNAKE_START_LEN {
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
                let x = self.below(self.width);
                let y = self.below(self.height);
                if self.at(x, y) == Cell::Empty && !self.burning(x, y) {
                    let kind = self.below(FOOD_GLYPHS.len()) as u8;
                    self.set(x, y, Cell::Food(kind));
                    break;
                }
            }
        }
    }

    fn explode(&mut self, x: usize, y: usize, radius: f32, start: u64) {
        self.explosions.push(Explosion {
            x,
            y,
            start,
            radius,
        });
    }

    /// Where a rider wants to go next: ahead, or a turn, whichever leaves
    /// the most road. Straight on is preferred when it is about as good, so
    /// the trails run long and geometric, with the odd turn for variety;
    /// and when a rival is close ahead, the way that cuts across its path
    /// scores extra, to wall it in.
    fn steer_bike(&mut self, i: usize) -> Dir {
        let bike = self.bikes[i].clone();
        let whim = self.below(8) == 0;
        let rival = self
            .bikes
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
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
            let room = self.room(nx, ny, LOOKAHEAD_ROOM) as i64;
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
        let mut i = 0;
        while i < self.bikes.len() {
            let dir = self.steer_bike(i);
            if !dir.horizontal() && self.tick % 2 == 1 {
                // Vertical moves only every other tick; see TICK_MS. A
                // rider may still swing onto a horizontal now.
                i += 1;
                continue;
            }
            if self.ride(i, dir) {
                i += 1;
            }
        }
    }

    /// Move rider `i` one cell in `dir`, or crash it if the cell is taken.
    /// Returns whether it is still riding.
    fn ride(&mut self, i: usize, dir: Dir) -> bool {
        let Bike {
            owner,
            x,
            y,
            dir: was,
            ..
        } = self.bikes[i];
        let next = self
            .step_from(x, y, dir)
            .filter(|&(nx, ny)| self.passable(nx, ny));
        match next {
            Some((nx, ny)) => {
                self.set(
                    x,
                    y,
                    Cell::Trail {
                        owner,
                        glyph: was.trail_glyph(dir),
                    },
                );
                self.set(nx, ny, Cell::Bike(owner));
                let bike = &mut self.bikes[i];
                bike.x = nx;
                bike.y = ny;
                bike.dir = dir;
                true
            }
            None => {
                self.set(
                    x,
                    y,
                    Cell::Trail {
                        owner,
                        glyph: was.trail_glyph(was),
                    },
                );
                self.explode(x, y, 2.5, self.tick);
                self.dead_trails.push((owner, self.tick));
                self.bikes.remove(i);
                self.respawns
                    .push((Kind::Bike, self.tick + BIKE_RESPAWN_TICKS));
                false
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

    /// Where the snake goes next: towards food if it can get there, else
    /// wherever there is the most room to keep moving.
    fn steer_snake(&self, id: usize) -> Option<Dir> {
        let snake = self.snakes[id].as_ref()?;
        let &(hx, hy) = snake.body.front()?;
        if let Some(dir) = self.path_to_food(id) {
            return Some(dir);
        }
        let leaving = (snake.grow == 0 && snake.body.len() > 1)
            .then(|| snake.body.back().copied())
            .flatten();
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

    fn move_snakes(&mut self) {
        for id in 0..self.snakes.len() {
            let Some(dir) = self.steer_snake(id) else {
                continue;
            };
            if !dir.horizontal() && self.tick % 2 == 1 {
                // As with the riders, vertical moves are half as frequent.
                continue;
            }
            self.slither(id, dir);
        }
    }

    /// Move snake `id` one cell in `dir`: eat what is there, or die on it.
    fn slither(&mut self, id: usize, dir: Dir) {
        let Some(snake) = self.snakes[id].as_ref() else {
            return;
        };
        let &(hx, hy) = snake.body.front().expect("a snake has a head");
        // The cell the tail is about to leave is free to move into: a snake
        // follows its own tail rather than dying on it, as it always has.
        let leaving = (snake.grow == 0 && snake.body.len() > 1)
            .then(|| snake.body.back().copied())
            .flatten();
        let next = self
            .step_from(hx, hy, dir)
            .filter(|&(nx, ny)| self.passable(nx, ny) || Some((nx, ny)) == leaving);
        let Some((nx, ny)) = next else {
            self.kill_snake(id);
            return;
        };
        let ate = matches!(self.at(nx, ny), Cell::Food(_));
        let room = (self.width * self.height / 24).clamp(8, SNAKE_MAX_LEN);
        let snake = self.snakes[id].as_mut().expect("checked above");
        snake.dir = dir;
        snake.body.push_front((nx, ny));
        if ate && snake.body.len() < room {
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
    fn kill_snake(&mut self, id: usize) {
        let Some(snake) = self.snakes[id].take() else {
            return;
        };
        for (i, &(x, y)) in snake.body.iter().enumerate() {
            self.set(x, y, Cell::Empty);
            let radius = if i == 0 { 1.5 } else { 0.6 };
            self.explode(x, y, radius, self.tick + (i as u64 / 3).min(4));
        }
        self.respawns
            .push((Kind::Snake, self.tick + SNAKE_RESPAWN_TICKS));
    }

    /// Trails of the fallen fade and go; and when the field is nearly full,
    /// the oldest trail still being ridden is retired to fade with them,
    /// its rider carrying on behind a fresh one. One goes at a time, so the
    /// arena thins out where it is oldest instead of being wiped.
    fn clear_wrecks(&mut self) {
        let tick = self.tick;
        let gone: Vec<u32> = self
            .dead_trails
            .iter()
            .filter(|&&(_, died)| tick - died > TRAIL_LINGER_TICKS + TRAIL_FADE_TICKS)
            .map(|&(owner, _)| owner)
            .collect();
        if !gone.is_empty() {
            for cell in &mut self.cells {
                if matches!(cell, Cell::Trail { owner, .. } if gone.contains(owner)) {
                    *cell = Cell::Empty;
                }
            }
            self.dead_trails.retain(|(owner, _)| !gone.contains(owner));
        }
        let open = self.cells.iter().filter(|c| **c == Cell::Empty).count();
        if (open as f32) >= self.cells.len() as f32 * CROWDED
            || tick.saturating_sub(self.last_retire) < TRAIL_FADE_TICKS
        {
            return;
        }
        let Some(i) = self
            .bikes
            .iter()
            .enumerate()
            .filter(|(_, b)| !self.dead_trails.iter().any(|(o, _)| *o == b.owner))
            .min_by_key(|(_, b)| b.owner)
            .map(|(i, _)| i)
        else {
            return;
        };
        // Retired trails fade from now rather than lingering first: the
        // room they free is wanted at once.
        self.dead_trails
            .push((self.bikes[i].owner, tick.saturating_sub(TRAIL_LINGER_TICKS)));
        let owner = self.next_owner;
        self.next_owner += 1;
        self.bikes[i].owner = owner;
        let (x, y) = (self.bikes[i].x, self.bikes[i].y);
        self.set(x, y, Cell::Bike(owner));
        self.last_retire = tick;
    }

    fn respawn_due(&mut self) {
        let tick = self.tick;
        let due: Vec<Kind> = self
            .respawns
            .iter()
            .filter(|&&(_, at)| at <= tick)
            .map(|&(kind, _)| kind)
            .collect();
        self.respawns.retain(|&(_, at)| at > tick);
        for kind in due {
            match kind {
                Kind::Bike => self.spawn_bike(),
                Kind::Snake => {
                    if let Some(id) = self.snakes.iter().position(Option::is_none) {
                        self.spawn_snake(id);
                    }
                }
            }
        }
        // Keep the field populated whatever happened: a spawn can fail for
        // want of room, and the population must not dwindle for it.
        let bikes_pending = self
            .respawns
            .iter()
            .filter(|(k, _)| *k == Kind::Bike)
            .count();
        if self.bikes.len() + bikes_pending < self.bike_target {
            self.respawns.push((Kind::Bike, tick + BIKE_RESPAWN_TICKS));
        }
        let snakes_pending = self
            .respawns
            .iter()
            .filter(|(k, _)| *k == Kind::Snake)
            .count();
        let snakes_missing = self.snakes.iter().filter(|s| s.is_none()).count();
        if snakes_missing > snakes_pending {
            self.respawns
                .push((Kind::Snake, tick + SNAKE_RESPAWN_TICKS));
        }
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

    /// Paint the arena at clock `ms`, which is somewhere within the current
    /// tick, through `put`.
    pub fn draw(&self, ms: u64, put: &mut dyn FnMut(usize, usize, char, Style)) {
        let t = ms as f32 / 1000.0;
        for y in 0..self.height {
            for x in 0..self.width {
                match self.at(x, y) {
                    Cell::Empty => {}
                    Cell::Trail { owner, glyph } => {
                        let color = self.trail_color(owner);
                        let faded = self
                            .dead_trails
                            .iter()
                            .find(|(o, _)| *o == owner)
                            .map(|&(_, died)| {
                                let age = self.tick.saturating_sub(died);
                                let into = age.saturating_sub(TRAIL_LINGER_TICKS) as f32;
                                1.0 - (into / TRAIL_FADE_TICKS as f32).min(0.85)
                            })
                            .unwrap_or(1.0);
                        put(x, y, glyph, Style::default().fg(dim(color, 0.7 * faded)));
                    }
                    Cell::Bike(owner) => {
                        let Some(bike) = self.bikes.iter().find(|b| b.owner == owner) else {
                            continue;
                        };
                        put(
                            x,
                            y,
                            bike.dir.bike_glyph(),
                            Style::default()
                                .fg(BIKE_COLORS[bike.color])
                                .add_modifier(Modifier::BOLD),
                        );
                    }
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
                            (
                                '@',
                                Style::default().fg(SNAKE_HEAD).add_modifier(Modifier::BOLD),
                            )
                        } else {
                            let fade = 1.0 - 0.5 * (index as f32 / snake.body.len().max(1) as f32);
                            let glyph = if index % 2 == 0 { 'o' } else { '=' };
                            (glyph, Style::default().fg(dim(SNAKE_BODY, fade)))
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
            self.draw_explosion(e, ms, put);
        }
    }

    fn trail_color(&self, owner: u32) -> Color {
        self.bikes
            .iter()
            .find(|b| b.owner == owner)
            .map(|b| BIKE_COLORS[b.color])
            .unwrap_or(BIKE_COLORS[owner as usize % BIKE_COLORS.len()])
    }

    /// A blast growing from its centre: a ring of sparks widening over the
    /// explosion's life, white-hot at first and cooling to a dull red, with
    /// a dense core that burns out as the ring leaves it behind.
    fn draw_explosion(
        &self,
        e: &Explosion,
        ms: u64,
        put: &mut dyn FnMut(usize, usize, char, Style),
    ) {
        let start_ms = e.start * TICK_MS;
        if ms < start_ms {
            return;
        }
        let life = (ms - start_ms) as f32 / (EXPLOSION_TICKS * TICK_MS) as f32;
        if life > 1.0 {
            return;
        }
        let radius = e.radius * life.sqrt();
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
        let reach = (e.radius.ceil() as i32).max(1);
        for dy in -reach..=reach {
            for dx in -reach * 2..=reach * 2 {
                let d = ((dx as f32 / 2.0).powi(2) + (dy as f32).powi(2)).sqrt();
                let on_ring = (d - radius).abs() < 0.75;
                let core = d < radius * 0.4 && life < 0.5;
                if !(on_ring || core) {
                    continue;
                }
                let x = e.x as i32 + dx;
                let y = e.y as i32 + dy;
                if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
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

fn mix(a: Color, b: Color, k: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) => {
            let k = k.clamp(0.0, 1.0);
            let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
            Color::Rgb(lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
        }
        _ => a,
    }
}

thread_local! {
    /// Arenas in progress, one per seed and box size in use. A frame is a
    /// pure function of clock and seed only if the arena that got there is
    /// the same one every time, so they are kept between frames and stepped
    /// forward to the clock rather than rebuilt.
    static ARENAS: RefCell<Vec<Arena>> = const { RefCell::new(Vec::new()) };
}

/// Arenas kept at once. The commit box has one size at a time, but the
/// tests and a resizing terminal want a few.
const KEPT: usize = 16;
/// Farther behind the clock than this and the arena is started over rather
/// than caught up: a long pause is not worth a stall.
const MAX_CATCH_UP: u64 = 400;

/// Draw the arena for `seed`, `width` by `height`, as it stands at `ms`.
/// The clock is expected to run forward: an arena caught up to a later `ms`
/// cannot be rewound, and a call for an earlier one starts a fresh round.
pub fn frame(
    seed: usize,
    width: usize,
    height: usize,
    ms: u64,
    put: &mut dyn FnMut(usize, usize, char, Style),
) {
    if width < 8 || height < 3 {
        return;
    }
    let target = ms / TICK_MS;
    ARENAS.with_borrow_mut(|arenas| {
        let same = |a: &Arena| a.seed == seed && a.width == width && a.height == height;
        let stale = |a: &Arena| a.tick > target || target - a.tick > MAX_CATCH_UP;
        arenas.retain(|a| !(same(a) && stale(a)));
        let index = match arenas.iter().position(same) {
            Some(i) => i,
            None => {
                if arenas.len() >= KEPT {
                    arenas.remove(0);
                }
                arenas.push(Arena::new(seed, width, height, target));
                arenas.len() - 1
            }
        };
        let arena = &mut arenas[index];
        while arena.tick < target {
            arena.step();
        }
        arena.draw(ms, put);
    });
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
