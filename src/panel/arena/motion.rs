//! Spawning, steering and moving the bikes and snakes, and clearing up after crashes.

use super::*;

impl Arena {
    pub(super) fn spawn_bike(&mut self) {
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

    pub(super) fn spawn_snake(&mut self, id: usize) {
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

    pub(super) fn replenish_food(&mut self) {
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

    pub(super) fn explode(&mut self, x: usize, y: usize, radius: f32, start: u64) {
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
    pub(super) fn steer_bike(&mut self, i: usize) -> Dir {
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

    pub(super) fn move_bikes(&mut self) {
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
    pub(super) fn ride(&mut self, i: usize, dir: Dir) -> bool {
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
    pub(super) fn path_to_food(&self, id: usize) -> Option<Dir> {
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
    pub(super) fn steer_snake(&self, id: usize) -> Option<Dir> {
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

    pub(super) fn move_snakes(&mut self) {
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
    pub(super) fn slither(&mut self, id: usize, dir: Dir) {
        let Some(snake) = self.snakes[id].as_ref() else {
            return;
        };
        let Some(&(hx, hy)) = snake.body.front() else {
            return;
        };
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
        let Some(snake) = self.snakes[id].as_mut() else {
            return;
        };
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
    pub(super) fn kill_snake(&mut self, id: usize) {
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
    pub(super) fn clear_wrecks(&mut self) {
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

    pub(super) fn respawn_due(&mut self) {
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
}
