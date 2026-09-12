//! Drawing the arena, and the frame clock that drives it.

use super::*;

impl Arena {
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

    pub(super) fn trail_color(&self, owner: u32) -> Color {
        self.bikes
            .iter()
            .find(|b| b.owner == owner)
            .map(|b| BIKE_COLORS[b.color])
            .unwrap_or(BIKE_COLORS[owner as usize % BIKE_COLORS.len()])
    }

    /// A blast growing from its centre: a ring of sparks widening over the
    /// explosion's life, white-hot at first and cooling to a dull red, with
    /// a dense core that burns out as the ring leaves it behind.
    pub(super) fn draw_explosion(
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

pub(super) fn dim(color: Color, k: f32) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * k) as u8,
            (g as f32 * k) as u8,
            (b as f32 * k) as u8,
        ),
        other => other,
    }
}

pub(super) fn mix(a: Color, b: Color, k: f32) -> Color {
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
pub(super) const KEPT: usize = 16;
/// Farther behind the clock than this and the arena is started over rather
/// than caught up: a long pause is not worth a stall.
pub(super) const MAX_CATCH_UP: u64 = 400;

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
