//! Wolfenstein 3D: a maze of stone and wood corridors walked in the first
//! person, guards coming round the corners and a pistol answering them.
//! Everything is drawn by the raycaster, so the walls have real perspective:
//! a corridor narrows to its end, a doorway opens as it is walked through
//! and a guard grows as it closes. The colours are the game's own — grey
//! stone, blue stone, wood panelling, the flat grey ceiling and floor it
//! had instead of a sky — and a face is lit by which way it points, which
//! is the only shading the original did.

use std::cell::RefCell;

use super::grid::{Rng, Sim};
use super::raycast::{self, Art, Frame, Map, View, Walker};
use super::*;

/// The walk: units a second, and radians a second turning.
const SPEED: f32 = 1.8;
const TURN: f32 = 3.2;
/// How wide the eye sees, in radians.
const FOV: f32 = 1.25;
/// Guards in the maze at once, and ticks before a dead one is replaced.
const GUARDS: usize = 7;
const RESPAWN: u64 = 80;
/// Ticks a guard's death takes, and how long a muzzle flash lasts.
const DYING: u64 = 14;
const FLASH: u64 = 3;
/// Ticks between shots the player can take, and between a guard's.
const COOLDOWN: u64 = 14;
const GUARD_COOLDOWN: u64 = 22;
/// How near the middle of the view a guard has to be to be shot at.
const AIM: f32 = 0.2;

const STONE: Color = Color::Rgb(150, 150, 156);
const BLUE_STONE: Color = Color::Rgb(80, 96, 180);
const WOOD: Color = Color::Rgb(168, 120, 64);
const BRICK: Color = Color::Rgb(172, 78, 62);
const CEILING: Color = Color::Rgb(70, 70, 76);
const FLOOR: Color = Color::Rgb(108, 106, 100);

/// A guard, an officer or an SS trooper: the three that patrol the halls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rank {
    Guard,
    Officer,
    SS,
}

impl Rank {
    fn of(n: u64) -> Self {
        match n % 3 {
            0 => Rank::Guard,
            1 => Rank::Officer,
            _ => Rank::SS,
        }
    }

    /// The colour of the uniform, and how much punishment it takes.
    fn uniform(self) -> Color {
        match self {
            Rank::Guard => Color::Rgb(128, 100, 60),
            Rank::Officer => Color::Rgb(220, 220, 228),
            Rank::SS => Color::Rgb(60, 86, 156),
        }
    }

    /// Shots it takes to put down: the better dressed, the more of them.
    fn health(self) -> u8 {
        match self {
            Rank::Guard => 2,
            Rank::Officer => 3,
            Rank::SS => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Guard {
    x: f32,
    y: f32,
    rank: Rank,
    health: u8,
    /// The tick it was shot on, once it has been.
    died: Option<u64>,
    /// The tick it last fired on.
    fired: u64,
}

/// A standing guard, an arm out with the gun in it.
const STANDING: [&str; 9] = [
    "  hhh  ", " hfffh ", " feyef ", "  fmf  ", " uuuuu ", "guuuuug", " uuuuu ", "  u u  ",
    "  b b  ",
];
/// The same, firing: the gun lit by its own flash.
const FIRING: [&str; 9] = [
    "  hhh  ", " hfffh ", " feyef ", "  fmf  ", " uuuuu ", "*Guuuuu", " uuuuu ", "  u u  ",
    "  b b  ",
];
/// Going down, and down.
const HIT: [&str; 9] = [
    "       ", "  hhh  ", " hfffh ", " frrrf ", " ruuur ", " uuuuu ", "  u u  ", "  b b  ",
    "       ",
];
const DOWN: [&str; 9] = [
    "       ", "       ", "       ", "       ", "       ", "   h   ", " uuuuu ", "buuuuub",
    "rrrrrrr",
];

/// The pistol, held low and to the right, and the flash off the end of it.
const PISTOL: [&str; 6] = [
    "    mm    ",
    "   wwww   ",
    "   wwww   ",
    "  hwwwwh  ",
    " hhwwwwhh ",
    " hhhhhhhh ",
];
const PISTOL_FIRING: [&str; 6] = [
    "  ff**ff  ",
    "  f****f  ",
    "   wwww   ",
    "  hwwwwh  ",
    " hhwwwwhh ",
    " hhhhhhhh ",
];

/// The colours a guard is drawn in, its uniform by rank.
fn paint(rank: Rank, c: char) -> Option<Color> {
    Some(match c {
        'h' => match rank {
            Rank::Officer => Color::Rgb(180, 180, 190),
            _ => Color::Rgb(74, 62, 44),
        },
        'f' => Color::Rgb(224, 176, 136),
        'e' => Color::Rgb(30, 30, 40),
        'm' => Color::Rgb(150, 80, 70),
        'u' => rank.uniform(),
        'b' => Color::Rgb(40, 40, 46),
        'g' => Color::Rgb(70, 70, 78),
        'G' => Color::Rgb(210, 190, 120),
        'r' => Color::Rgb(150, 40, 36),
        '*' => Color::Rgb(255, 240, 170),
        _ => return None,
    })
}

fn paint_guard(c: char) -> Option<Color> {
    paint(Rank::Guard, c)
}

fn paint_officer(c: char) -> Option<Color> {
    paint(Rank::Officer, c)
}

fn paint_ss(c: char) -> Option<Color> {
    paint(Rank::SS, c)
}

fn pistol_color(c: char) -> Option<Color> {
    Some(match c {
        'w' => Color::Rgb(96, 98, 108),
        'm' => Color::Rgb(52, 54, 62),
        'h' => Color::Rgb(212, 168, 130),
        'f' => Color::Rgb(255, 170, 60),
        '*' => Color::Rgb(255, 250, 210),
        _ => return None,
    })
}

/// The maze, who is in it and what they are doing.
struct World {
    key: (usize, usize),
    tick: u64,
    rng: Rng,
    map: Map,
    walker: Walker,
    guards: Vec<Guard>,
    /// The tick the player last fired on, and last took a hit on.
    fired: u64,
    hurt: u64,
}

impl World {
    /// Whether one point can see another: nothing solid between them.
    fn sees(&self, from: (f32, f32), to: (f32, f32)) -> bool {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let d = dx.hypot(dy).max(0.001);
        raycast::cast(&self.map, from.0, from.1, dx / d, dy / d).dist >= d - 0.05
    }

    /// Somewhere to put a guard: far enough off not to appear out of thin
    /// air in front of the player, near enough to be met, and by choice
    /// somewhere the player can see from where they stand — a guard is
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

    /// A guard comes after the player down whichever hall they went, stops
    /// short once it has them in sight, and fires from where it stands.
    fn move_guard(&mut self, i: usize) {
        let guard = self.guards[i];
        let (px, py) = (self.walker.x, self.walker.y);
        let sees = self.sees((guard.x, guard.y), (px, py));
        let (dx, dy) = (px - guard.x, py - guard.y);
        let d = dx.hypot(dy).max(0.001);
        if d > 2.2 || !sees {
            let step = if sees { 0.075 } else { 0.09 };
            let (x, y) = raycast::chase(&self.map, (guard.x, guard.y), (px, py), step);
            self.guards[i].x = x;
            self.guards[i].y = y;
        }
        if sees && self.tick.saturating_sub(guard.fired) > GUARD_COOLDOWN && self.rng.below(14) == 0
        {
            self.guards[i].fired = self.tick;
            if self.rng.below(3) == 0 {
                self.hurt = self.tick;
            }
        }
    }

    /// The player shoots whatever is in front of them: the nearest guard
    /// within the sights, once the pistol has come back down.
    fn take_a_shot(&mut self) {
        if self.tick.saturating_sub(self.fired) < COOLDOWN {
            return;
        }
        let (px, py, pa) = (self.walker.x, self.walker.y, self.walker.view);
        let mut target: Option<(usize, f32)> = None;
        for (i, guard) in self.guards.iter().enumerate() {
            if guard.died.is_some() {
                continue;
            }
            let (dx, dy) = (guard.x - px, guard.y - py);
            let d = dx.hypot(dy);
            let mut off = (dy.atan2(dx) - pa).rem_euclid(std::f32::consts::TAU);
            if off > std::f32::consts::PI {
                off -= std::f32::consts::TAU;
            }
            if off.abs() < AIM
                && d < 9.0
                && self.sees((px, py), (guard.x, guard.y))
                && target.is_none_or(|(_, best)| d < best)
            {
                target = Some((i, d));
            }
        }
        let Some((i, _)) = target else { return };
        self.fired = self.tick;
        self.guards[i].health = self.guards[i].health.saturating_sub(1);
        if self.guards[i].health == 0 {
            self.guards[i].died = Some(self.tick);
        }
    }

    /// Turn on whatever is coming: a target in sight is worth stopping the
    /// walk for, and the walk only carries on once it has been dealt with.
    fn face(&mut self, dt: f32) {
        let (px, py) = (self.walker.x, self.walker.y);
        let nearest = self
            .guards
            .iter()
            .filter(|m| m.died.is_none())
            .map(|m| ((m.x - px).hypot(m.y - py), m.x, m.y))
            .filter(|&(d, x, y)| d < 12.0 && self.sees((px, py), (x, y)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, x, y)) = nearest else { return };
        self.walker.look((y - py).atan2(x - px), TURN * dt);
    }

    /// The picture of a guard as it is at this tick.
    fn art(&self, guard: &Guard) -> Art {
        let rows = match guard.died {
            Some(died) => {
                if self.tick - died < DYING / 2 {
                    &HIT
                } else {
                    &DOWN
                }
            }
            None if self.tick.saturating_sub(guard.fired) < FLASH => &FIRING,
            None => &STANDING,
        };
        Art {
            rows,
            color: match guard.rank {
                Rank::Guard => paint_guard,
                Rank::Officer => paint_officer,
                Rank::SS => paint_ss,
            },
            // A nine-row band is not much to be seen in, so a guard stands
            // a little taller than the room would allow: one the geometry's
            // own size would be three pixels of mush.
            height: 0.95,
            lift: 0.0,
        }
    }
}

/// The face of a wall: its stone, its mortar and the crest hung on the odd
/// one, at `u` across the face and `v` down it.
fn wall_pixel(kind: u8, cell: (i32, i32), u: f32, v: f32, vertical: bool) -> Color {
    let base = match kind {
        1 => STONE,
        2 => BLUE_STONE,
        3 => WOOD,
        _ => BRICK,
    };
    let color = if kind == 3 {
        // Wood: vertical boards with a dark groove between them.
        let board = (u * 6.0).fract();
        dim(
            base,
            if board < 0.08 {
                0.55
            } else {
                0.9 + 0.1 * (u * 37.0).sin()
            },
        )
    } else if kind == 2
        && hash(cell.0 as usize, cell.1 as usize).is_multiple_of(4)
        && (0.34..0.58).contains(&v)
        && (0.42..0.58).contains(&u)
    {
        // The crest hung along the blue stone halls.
        Color::Rgb(178, 34, 40)
    } else {
        // Stone: courses of brick, every other one offset by half a brick.
        let course = (v * 8.0).floor();
        let along = (u * 4.0 + if course as i32 % 2 == 0 { 0.0 } else { 0.5 }).fract();
        let seam = (v * 8.0).fract() < 0.12 || along < 0.06;
        let grain = (hash((u * 64.0) as usize, (v * 64.0) as usize) % 24) as f32 / 200.0;
        dim(base, if seam { 0.62 } else { 0.92 + grain })
    };
    // The only shading the original did: a face that runs the other way is
    // darker, which is what tells one corridor wall from the next.
    if vertical { color } else { dim(color, 0.72) }
}

impl Sim for World {
    const TICK_MS: u64 = 33;

    fn new(seed: usize, width: usize, _height: usize, tick: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0x5701);
        let map = Map::maze(&mut rng, 23, 23, 4);
        let walker = Walker::new(&map, &mut rng);
        let mut world = Self {
            key: (seed, width),
            tick,
            rng,
            map,
            walker,
            guards: Vec::new(),
            fired: 0,
            hurt: 0,
        };
        for i in 0..GUARDS {
            let (x, y) = world.spawn();
            let rank = Rank::of(hash(i, seed));
            world.guards.push(Guard {
                x,
                y,
                rank,
                health: rank.health(),
                died: None,
                fired: 0,
            });
        }
        world
    }

    fn key(&self) -> (usize, usize) {
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
        for i in 0..self.guards.len() {
            match self.guards[i].died {
                Some(died) if self.tick - died > DYING + RESPAWN => {
                    let (x, y) = self.spawn();
                    let rank = Rank::of(self.rng.next());
                    self.guards[i] = Guard {
                        x,
                        y,
                        rank,
                        health: rank.health(),
                        died: None,
                        fired: 0,
                    };
                }
                Some(_) => {}
                None => self.move_guard(i),
            }
        }
        self.face(dt);
        self.take_a_shot();
    }

    fn draw(&self, _ms: u64, rows: usize, put: &mut dyn FnMut(usize, usize, char, Style)) {
        let w = self.key.1;
        let mut frame = Frame::new(w, rows);
        // The step puts a swing in the view and in the hands.
        let swing = (self.walker.walked * 3.4).sin();
        let view = View {
            x: self.walker.x,
            y: self.walker.y,
            a: self.walker.view,
            bob: swing * 0.6,
            w,
            h: frame.height(),
            fov: FOV,
        };
        let mut depth = vec![f32::MAX; w];
        for (col, ahead) in depth.iter_mut().enumerate() {
            let (rdx, rdy) = view.ray(col);
            let hit = raycast::cast(&self.map, view.x, view.y, rdx, rdy);
            // The distance along the view, not along the ray: otherwise a
            // flat wall would bulge towards the middle of the screen.
            let straight = hit.dist * (rdx * view.a.cos() + rdy * view.a.sin());
            *ahead = straight;
            let (top, bottom) = view.wall(straight);
            let light = (1.0 - straight / 26.0).clamp(0.45, 1.0);
            for y in 0..frame.height() {
                let yf = y as f32;
                if yf < top {
                    frame.set(col, y, dim(CEILING, 0.7 + 0.3 * (yf / top.max(1.0))));
                } else if yf >= bottom {
                    // The floor lightens towards the feet as it nears.
                    let d = view.floor_dist(yf + 0.5);
                    frame.set(col, y, dim(FLOOR, (1.0 - d / 22.0).clamp(0.28, 1.0)));
                } else {
                    let v = (yf - top) / (bottom - top).max(0.001);
                    frame.set(
                        col,
                        y,
                        dim(
                            wall_pixel(hit.kind, hit.cell, hit.tex, v, hit.vertical),
                            light,
                        ),
                    );
                }
            }
        }
        // Guards farthest first, so a near one covers the one behind it.
        let mut guards: Vec<&Guard> = self.guards.iter().collect();
        guards.sort_by(|a, b| {
            let d = |g: &Guard| (g.x - view.x).hypot(g.y - view.y);
            d(b).total_cmp(&d(a))
        });
        for guard in guards {
            let d = (guard.x - view.x).hypot(guard.y - view.y);
            let light = (1.0 - d / 26.0).clamp(0.35, 1.0);
            raycast::billboard(
                &mut frame,
                &view,
                &depth,
                guard.x,
                guard.y,
                &self.art(guard),
                light,
            );
        }
        let firing = self.tick.saturating_sub(self.fired) < FLASH;
        if firing {
            // The shot lights the corridor for as long as it lasts.
            frame.wash(Color::Rgb(255, 230, 160), 0.14);
        }
        if self.tick.saturating_sub(self.hurt) < 2 {
            frame.wash(Color::Rgb(180, 30, 30), 0.28);
        }
        let px = (frame.height() as f32 / 15.0).max(1.0);
        raycast::held(
            &mut frame,
            if firing { &PISTOL_FIRING } else { &PISTOL },
            pistol_color,
            px,
            swing * px * 0.8,
            if firing { -px } else { 0.0 },
        );
        frame.flush(put);
    }
}

thread_local! {
    static WORLDS: RefCell<Vec<World>> = const { RefCell::new(Vec::new()) };
}

/// Draw the maze for `seed`, `width` by `height`, as it stands at `ms`.
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

    fn world(ticks: u64) -> World {
        let mut world = World::new(3, 80, 16, 0);
        for _ in 0..ticks {
            world.step();
        }
        world
    }

    #[test]
    fn the_walk_goes_down_the_corridors_and_the_guards_come_and_go() {
        let world = world(1_200);
        assert!(!world.map.solid(world.walker.x, world.walker.y));
        assert!(world.walker.walked > 30.0, "the maze is being walked");
        assert!(
            world.guards.iter().any(|g| g.died.is_some()) || world.fired > 0,
            "and the pistol is being used"
        );
        for guard in &world.guards {
            assert!(
                !world.map.solid(guard.x, guard.y),
                "guards keep to the halls"
            );
        }
    }

    #[test]
    fn a_nearer_wall_covers_more_of_the_view_than_a_far_one() {
        let view = View {
            x: 0.0,
            y: 0.0,
            a: 0.0,
            bob: 0.0,
            w: 80,
            h: 32,
            fov: FOV,
        };
        let (near_top, near_bottom) = view.wall(1.0);
        let (far_top, far_bottom) = view.wall(6.0);
        assert!(
            near_bottom - near_top > 4.0 * (far_bottom - far_top),
            "depth is depth"
        );
        assert!(
            near_top < far_top && near_bottom > far_bottom,
            "and it grows about the horizon"
        );
    }
}
