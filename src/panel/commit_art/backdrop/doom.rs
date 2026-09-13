//! Doom: the same first-person view as the Wolfenstein backdrop, in a
//! darker place. Where that maze is flat-lit and made of stone, this one is
//! a tech base that fades to black a few rooms out, its floor and ceiling
//! textured rather than painted — each pixel of them is worked back out to
//! the slab it lands on, so the tiles run away under the feet with the same
//! perspective as the walls. Imps and cacodemons come at the player through
//! it, throwing fire that lights the room it crosses, and a shotgun answers
//! them with a flash that lights the lot.

use std::cell::RefCell;

use super::grid::{Rng, Sim};
use super::raycast::{self, Art, Frame, Map, View, Walker};
use super::*;

const SPEED: f32 = 1.9;
const TURN: f32 = 3.4;
const FOV: f32 = 1.3;
/// Monsters in the base at once, and ticks before a dead one is replaced.
const MONSTERS: usize = 6;
const RESPAWN: u64 = 80;
const DYING: u64 = 18;
/// Ticks a muzzle flash lasts, and between shells.
const FLASH: u64 = 4;
const COOLDOWN: u64 = 20;
const AIM: f32 = 0.26;
/// How far the light reaches before the base goes black.
const REACH: f32 = 13.0;
/// Units a second a fireball travels, and seconds before it burns out.
const BALL_SPEED: f32 = 4.0;
const BALL_LIFE: u64 = 90;

const BROWN_BRICK: Color = Color::Rgb(150, 104, 64);
const GREEN_MARBLE: Color = Color::Rgb(74, 106, 78);
const GREY_TECH: Color = Color::Rgb(118, 118, 126);
const COMPUTER: Color = Color::Rgb(70, 78, 96);
const FLOOR_A: Color = Color::Rgb(96, 78, 58);
const FLOOR_B: Color = Color::Rgb(72, 60, 46);
const CEIL_A: Color = Color::Rgb(62, 66, 70);
const CEIL_B: Color = Color::Rgb(46, 50, 54);
const FIRE: Color = Color::Rgb(255, 150, 40);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Imp,
    Pinky,
    Cacodemon,
}

impl Kind {
    fn of(n: u64) -> Self {
        match n % 3 {
            0 => Kind::Imp,
            1 => Kind::Pinky,
            _ => Kind::Cacodemon,
        }
    }

    /// Shells it takes to put down.
    fn health(self) -> u8 {
        match self {
            Kind::Imp => 2,
            Kind::Pinky => 3,
            Kind::Cacodemon => 4,
        }
    }

    /// How high off the floor it is: a cacodemon floats.
    fn lift(self) -> f32 {
        match self {
            Kind::Cacodemon => 0.3,
            _ => 0.0,
        }
    }

    /// How tall it is drawn. A nine-row band is not much to be seen in, so
    /// everything in it stands a little taller than the room would allow:
    /// a demon the geometry's own size would be three pixels of mush.
    fn height(self) -> f32 {
        match self {
            Kind::Imp => 0.95,
            Kind::Pinky => 0.8,
            Kind::Cacodemon => 0.7,
        }
    }

    /// Whether it throws fire or has to reach the player to do anything.
    fn throws(self) -> bool {
        !matches!(self, Kind::Pinky)
    }
}

#[derive(Debug, Clone, Copy)]
struct Monster {
    x: f32,
    y: f32,
    kind: Kind,
    health: u8,
    died: Option<u64>,
    threw: u64,
}

#[derive(Debug, Clone, Copy)]
struct Ball {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    born: u64,
}

const IMP: [&str; 8] = [
    " h   h ", "  bbb  ", " beyeb ", " bbtbb ", "cbbbbbc", "cbbbbbc", " bb bb ", " f   f ",
];
const IMP_THROWING: [&str; 8] = [
    " h   h ", "  bbb  ", " beyeb ", " bbtbb ", "*bbbbb*", "obbbbbo", " bb bb ", " f   f ",
];
const PINKY: [&str; 8] = [
    "       ", " ppppp ", "peptpep", "ptttttp", "ppppppp", "cpppppc", " pp pp ", " f   f ",
];
const CACO: [&str; 7] = [
    "  ddd  ", " ddddd ", "ddhthdd", "dddedd d", "ddtttdd", " ddddd ", "  ddd  ",
];
const CORPSE: [&str; 8] = [
    "       ", "       ", "       ", "       ", "       ", "  r r  ", " rrrrr ", "rrrrrrr",
];
const BALL_ART: [&str; 3] = [" *o* ", "*ooo*", " *o* "];

const SHOTGUN: [&str; 7] = [
    "     bb     ",
    "     bb     ",
    "    bwwb    ",
    "   bbwwbb   ",
    "  hbbwwbbh  ",
    "  hhhwwhhh  ",
    "   hhhhhh   ",
];
const SHOTGUN_FIRING: [&str; 7] = [
    "   f*  *f   ",
    "  f**bb**f  ",
    "   f*ww*f   ",
    "   bbwwbb   ",
    "  hbbwwbbh  ",
    "  hhhwwhhh  ",
    "   hhhhhh   ",
];

fn paint(c: char) -> Option<Color> {
    Some(match c {
        'b' => Color::Rgb(140, 88, 52),
        'h' => Color::Rgb(96, 60, 36),
        'c' => Color::Rgb(112, 70, 44),
        'f' => Color::Rgb(72, 46, 30),
        'p' => Color::Rgb(196, 108, 116),
        'd' => Color::Rgb(168, 48, 44),
        'e' => Color::Rgb(255, 220, 90),
        'y' => Color::Rgb(255, 90, 60),
        't' => Color::Rgb(240, 236, 220),
        'r' => Color::Rgb(150, 28, 26),
        'o' => Color::Rgb(255, 120, 30),
        '*' => Color::Rgb(255, 232, 150),
        _ => return None,
    })
}

fn shotgun_paint(c: char) -> Option<Color> {
    Some(match c {
        'b' => Color::Rgb(58, 60, 68),
        'w' => Color::Rgb(92, 96, 106),
        'h' => Color::Rgb(112, 72, 40),
        'f' => Color::Rgb(255, 160, 40),
        '*' => Color::Rgb(255, 250, 210),
        _ => return None,
    })
}

/// The base, what is loose in it and what is in the air.
struct World {
    key: (usize, usize, usize),
    tick: u64,
    rng: Rng,
    map: Map,
    walker: Walker,
    monsters: Vec<Monster>,
    balls: Vec<Ball>,
    fired: u64,
    hurt: u64,
}

impl World {
    fn sees(&self, from: (f32, f32), to: (f32, f32)) -> bool {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let d = dx.hypot(dy).max(0.001);
        raycast::cast(&self.map, from.0, from.1, dx / d, dy / d).dist >= d - 0.05
    }

    /// Somewhere to put a monster: far enough off not to appear out of thin
    /// air in front of the player, near enough to be met, and by choice
    /// somewhere the player can see from where they stand — a monster is
    /// wanted down the hall ahead, not lost three rooms away.
    fn spawn(&mut self) -> (f32, f32) {
        let open = self.map.open();
        let (px, py) = (self.walker.x, self.walker.y);
        let mut fallback = (px, py);
        for tries in 0..48 {
            let (x, y) = open[self.rng.below(open.len().max(1))];
            let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = (x - px).hypot(y - py);
            if !(3.5..12.0).contains(&d) {
                continue;
            }
            fallback = (x, y);
            // After enough tries, anywhere at the right distance will do.
            if self.sees((px, py), (x, y)) || tries > 32 {
                return (x, y);
            }
        }
        fallback
    }

    /// A monster comes for the player wherever they are, and throws fire at
    /// them when it has them in sight and is the sort that can.
    fn move_monster(&mut self, i: usize) {
        let monster = self.monsters[i];
        let (px, py) = (self.walker.x, self.walker.y);
        let sees = self.sees((monster.x, monster.y), (px, py));
        let (dx, dy) = (px - monster.x, py - monster.y);
        let d = dx.hypot(dy).max(0.001);
        let close = if monster.kind.throws() && sees {
            2.5
        } else {
            0.9
        };
        if d > close {
            let step = if monster.kind == Kind::Pinky {
                0.1
            } else {
                0.075
            };
            let (x, y) = raycast::chase(&self.map, (monster.x, monster.y), (px, py), step);
            self.monsters[i].x = x;
            self.monsters[i].y = y;
        } else if !monster.kind.throws() {
            self.hurt = self.tick;
        }
        if sees
            && monster.kind.throws()
            && self.tick.saturating_sub(monster.threw) > 50
            && self.rng.below(18) == 0
        {
            self.monsters[i].threw = self.tick;
            self.balls.push(Ball {
                x: monster.x,
                y: monster.y,
                dx: dx / d,
                dy: dy / d,
                born: self.tick,
            });
        }
    }

    /// Fire crosses the room until it reaches the player or a wall.
    fn move_balls(&mut self) {
        let dt = Self::TICK_MS as f32 / 1000.0;
        let (px, py) = (self.walker.x, self.walker.y);
        let tick = self.tick;
        let mut hit = false;
        let map = &self.map;
        self.balls.retain_mut(|ball| {
            ball.x += ball.dx * BALL_SPEED * dt;
            ball.y += ball.dy * BALL_SPEED * dt;
            if (ball.x - px).hypot(ball.y - py) < 0.4 {
                hit = true;
                return false;
            }
            !map.solid(ball.x, ball.y) && tick - ball.born < BALL_LIFE
        });
        if hit {
            self.hurt = self.tick;
        }
    }

    /// A shell into whatever is in front of the player.
    fn take_a_shot(&mut self) {
        if self.tick.saturating_sub(self.fired) < COOLDOWN {
            return;
        }
        let (px, py, pa) = (self.walker.x, self.walker.y, self.walker.view);
        let mut target: Option<(usize, f32)> = None;
        for (i, monster) in self.monsters.iter().enumerate() {
            if monster.died.is_some() {
                continue;
            }
            let (dx, dy) = (monster.x - px, monster.y - py);
            let d = dx.hypot(dy);
            let mut off = (dy.atan2(dx) - pa).rem_euclid(std::f32::consts::TAU);
            if off > std::f32::consts::PI {
                off -= std::f32::consts::TAU;
            }
            if off.abs() < AIM
                && d < REACH
                && self.sees((px, py), (monster.x, monster.y))
                && target.is_none_or(|(_, best)| d < best)
            {
                target = Some((i, d));
            }
        }
        let Some((i, _)) = target else { return };
        self.fired = self.tick;
        // A shotgun at close quarters takes more out of a thing than one
        // fired across the hall.
        self.monsters[i].health = self.monsters[i].health.saturating_sub(1);
        if self.monsters[i].health == 0 {
            self.monsters[i].died = Some(self.tick);
        }
    }

    /// Turn on whatever is coming: a target in sight is worth stopping the
    /// walk for, and the walk only carries on once it has been dealt with.
    fn face(&mut self, dt: f32) {
        let (px, py) = (self.walker.x, self.walker.y);
        let nearest = self
            .monsters
            .iter()
            .filter(|m| m.died.is_none())
            .map(|m| ((m.x - px).hypot(m.y - py), m.x, m.y))
            .filter(|&(d, x, y)| d < REACH && self.sees((px, py), (x, y)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, x, y)) = nearest else { return };
        self.walker.look((y - py).atan2(x - px), TURN * dt);
    }

    fn art(&self, monster: &Monster) -> Art {
        let rows: &'static [&'static str] = match (monster.died, monster.kind) {
            (Some(died), _) if self.tick - died > DYING / 3 => &CORPSE,
            (_, Kind::Imp) if self.tick.saturating_sub(monster.threw) < 8 => &IMP_THROWING,
            (_, Kind::Imp) => &IMP,
            (_, Kind::Pinky) => &PINKY,
            (_, Kind::Cacodemon) => &CACO,
        };
        Art {
            rows,
            color: paint,
            height: monster.kind.height(),
            lift: if monster.died.is_some() {
                0.0
            } else {
                monster.kind.lift()
            },
        }
    }
}

/// The face of a wall: tech panels, marble and brick, with the lights that
/// are the only thing lit in the place.
fn wall_pixel(kind: u8, cell: (i32, i32), u: f32, v: f32, vertical: bool) -> Color {
    let color = match kind {
        1 => {
            // Brick, in courses.
            let course = (v * 7.0).floor();
            let along = (u * 4.0 + if course as i32 % 2 == 0 { 0.0 } else { 0.5 }).fract();
            let seam = (v * 7.0).fract() < 0.13 || along < 0.07;
            dim(BROWN_BRICK, if seam { 0.55 } else { 0.95 })
        }
        2 => {
            // Marble, veined.
            let vein = ((u * 9.0).sin() + (v * 6.0 + u * 4.0).sin()) * 0.5;
            mix(
                GREEN_MARBLE,
                Color::Rgb(30, 44, 34),
                (vein * 0.5 + 0.5) * 0.6,
            )
        }
        3 => {
            // Panelled metal with a rivet at each corner.
            let (pu, pv) = ((u * 3.0).fract(), (v * 3.0).fract());
            let edge = pu < 0.08 || pv < 0.08;
            let rivet = (pu - 0.5).hypot(pv - 0.5) < 0.08;
            if rivet {
                dim(GREY_TECH, 1.25)
            } else {
                dim(GREY_TECH, if edge { 0.6 } else { 0.88 })
            }
        }
        _ => {
            // A computer bank: rows of lamps that stay lit in the dark.
            let (pu, pv) = ((u * 8.0).fract(), (v * 5.0).fract());
            if pv > 0.55 && pv < 0.85 && pu > 0.2 && pu < 0.8 {
                let on = hash(
                    (u * 8.0) as usize + cell.0.unsigned_abs() as usize * 8,
                    (v * 5.0) as usize + cell.1.unsigned_abs() as usize * 5,
                ) % 3;
                match on {
                    0 => Color::Rgb(255, 90, 70),
                    1 => Color::Rgb(90, 220, 140),
                    _ => Color::Rgb(60, 64, 80),
                }
            } else {
                dim(COMPUTER, 0.9)
            }
        }
    };
    if vertical { color } else { dim(color, 0.72) }
}

/// How much light reaches something `d` away: the base falls off into the
/// dark, which is what makes it Doom rather than a well-lit maze.
fn light(d: f32) -> f32 {
    (1.0 - d / REACH).clamp(0.0, 1.0).powf(1.3) * 0.92 + 0.08
}

impl Sim for World {
    const TICK_MS: u64 = 33;

    fn new(seed: usize, width: usize, height: usize, tick: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0x1993);
        let map = Map::maze(&mut rng, 25, 25, 4);
        let walker = Walker::new(&map, &mut rng);
        let mut world = Self {
            key: (seed, width, height),
            tick,
            rng,
            map,
            walker,
            monsters: Vec::new(),
            balls: Vec::new(),
            fired: 0,
            hurt: 0,
        };
        for i in 0..MONSTERS {
            let (x, y) = world.spawn();
            let kind = Kind::of(hash(i, seed + 5));
            world.monsters.push(Monster {
                x,
                y,
                kind,
                health: kind.health(),
                died: None,
                threw: 0,
            });
        }
        world
    }

    fn key(&self) -> (usize, usize, usize) {
        self.key
    }

    fn tick(&self) -> u64 {
        self.tick
    }

    fn step(&mut self) {
        self.tick += 1;
        let dt = Self::TICK_MS as f32 / 1000.0;
        let mut rng = std::mem::replace(&mut self.rng, Rng::new(0));
        self.walker.step(&self.map, &mut rng, dt, SPEED, TURN);
        self.rng = rng;
        for i in 0..self.monsters.len() {
            match self.monsters[i].died {
                Some(died) if self.tick - died > DYING + RESPAWN => {
                    let (x, y) = self.spawn();
                    let kind = Kind::of(self.rng.next());
                    self.monsters[i] = Monster {
                        x,
                        y,
                        kind,
                        health: kind.health(),
                        died: None,
                        threw: 0,
                    };
                }
                Some(_) => {}
                None => self.move_monster(i),
            }
        }
        self.move_balls();
        self.face(dt);
        self.take_a_shot();
    }

    fn draw(&self, _ms: u64, put: &mut dyn FnMut(usize, usize, char, Style)) {
        let (w, rows) = (self.key.1, self.key.2);
        let mut frame = Frame::new(w, rows);
        let swing = (self.walker.walked * 3.0).sin();
        let view = View {
            x: self.walker.x,
            y: self.walker.y,
            a: self.walker.view,
            bob: swing * 0.7,
            w,
            h: frame.height(),
            fov: FOV,
        };
        let mut depth = vec![f32::MAX; w];
        for (col, ahead) in depth.iter_mut().enumerate() {
            let (rdx, rdy) = view.ray(col);
            let hit = raycast::cast(&self.map, view.x, view.y, rdx, rdy);
            let straight = hit.dist * (rdx * view.a.cos() + rdy * view.a.sin());
            *ahead = straight;
            let (top, bottom) = view.wall(straight);
            for y in 0..frame.height() {
                let yf = y as f32 + 0.5;
                if yf >= top && yf < bottom {
                    let v = (yf - top) / (bottom - top).max(0.001);
                    let color = wall_pixel(hit.kind, hit.cell, hit.tex, v, hit.vertical);
                    frame.set(col, y, dim(color, light(straight)));
                    continue;
                }
                // Floor and ceiling: the slab each pixel lands on is worked
                // back out of the view, so the tiles keep their perspective.
                let ceiling = yf < top;
                let row = if ceiling {
                    2.0 * view.horizon() - yf
                } else {
                    yf
                };
                let d = view.floor_dist(row);
                if d > REACH {
                    frame.set(col, y, Color::Rgb(0, 0, 0));
                    continue;
                }
                let (fx, fy) = view.floor_at(col, row);
                let tile = (fx.floor() as i32 + fy.floor() as i32) % 2 == 0;
                let grain = (hash((fx * 4.0) as usize, (fy * 4.0) as usize) % 20) as f32 / 160.0;
                let color = match (ceiling, tile) {
                    (true, true) => CEIL_A,
                    (true, false) => CEIL_B,
                    (false, true) => FLOOR_A,
                    (false, false) => FLOOR_B,
                };
                frame.set(col, y, dim(color, light(d) * (0.92 + grain)));
            }
        }
        // Everything loose in the room, farthest first.
        let mut things: Vec<(f32, f32, Art, bool)> = self
            .monsters
            .iter()
            .map(|m| (m.x, m.y, self.art(m), false))
            .chain(self.balls.iter().map(|b| {
                (
                    b.x,
                    b.y,
                    Art {
                        rows: &BALL_ART,
                        color: paint,
                        height: 0.22,
                        lift: 0.36,
                    },
                    true,
                )
            }))
            .collect();
        things.sort_by(|a, b| {
            let d = |x: f32, y: f32| (x - view.x).hypot(y - view.y);
            d(b.0, b.1).total_cmp(&d(a.0, a.1))
        });
        for (x, y, art, glowing) in &things {
            let d = (x - view.x).hypot(y - view.y);
            // Fire carries its own light; everything else takes the room's.
            let lit = if *glowing { 1.0 } else { light(d) };
            raycast::billboard(&mut frame, &view, &depth, *x, *y, art, lit);
        }
        // What a fireball throws onto the walls around it, brightest on the
        // ones it is nearest.
        for ball in &self.balls {
            let d = (ball.x - view.x).hypot(ball.y - view.y);
            if d < 5.0 {
                frame.wash(FIRE, 0.05 * (1.0 - d / 5.0));
            }
        }
        let firing = self.tick.saturating_sub(self.fired) < FLASH;
        if firing {
            frame.wash(Color::Rgb(255, 220, 150), 0.22);
        }
        if self.tick.saturating_sub(self.hurt) < 2 {
            frame.wash(Color::Rgb(190, 20, 20), 0.3);
        }
        let px = (frame.height() as f32 / 16.0).max(1.0);
        raycast::held(
            &mut frame,
            if firing { &SHOTGUN_FIRING } else { &SHOTGUN },
            shotgun_paint,
            px,
            swing * px * 0.7,
            if firing { -px * 1.4 } else { 0.0 },
        );
        frame.flush(put);
    }
}

thread_local! {
    static WORLDS: RefCell<Vec<World>> = const { RefCell::new(Vec::new()) };
}

/// Draw the base for `seed`, `width` by `height`, as it stands at `ms`.
pub(super) fn frame(
    seed: usize,
    width: usize,
    height: usize,
    ms: u64,
    put: &mut dyn FnMut(usize, usize, char, Style),
) {
    super::grid::frame(&WORLDS, seed, width, height, ms, put)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_is_walked_and_the_things_in_it_come_at_the_player() {
        let mut world = World::new(7, 90, 18, 0);
        let mut thrown = false;
        let mut killed = false;
        for _ in 0..2_000 {
            world.step();
            thrown |= !world.balls.is_empty();
            killed |= world.monsters.iter().any(|m| m.died.is_some());
            assert!(!world.map.solid(world.walker.x, world.walker.y));
            for monster in &world.monsters {
                assert!(
                    !world.map.solid(monster.x, monster.y),
                    "monsters keep to the rooms"
                );
            }
        }
        assert!(world.walker.walked > 30.0, "the base is being walked");
        assert!(thrown, "fire is thrown at the player");
        assert!(killed, "and the shotgun answers");
    }

    #[test]
    fn the_far_end_of_a_hall_is_darker_than_the_near_end() {
        assert!(light(1.0) > light(5.0), "the light falls off with distance");
        assert!(light(REACH + 1.0) < 0.1, "and runs out");
    }
}
