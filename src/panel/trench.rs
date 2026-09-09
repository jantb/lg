//! The trench run for the sky above the commit scene, seen from behind the
//! X-wing: the Death Star's surface fills the view, its limb curving across
//! the top with stars beyond, and the trench runs away down the middle to
//! the exhaust port on the horizon. The fighter weaves down it with TIE
//! fighters on its tail, turrets along the rim firing up at it and its own
//! cannon answering. When the model's first words fly in, the shot has been
//! taken: the torpedoes go home, the surface flashes and the fire runs out
//! from the port until the whole station is gone, a ring racing away where
//! it was and the fighter climbing clear. Every frame is a function of the
//! clock alone, so any frame is the same picture whoever draws it.

use ratatui::style::{Color, Modifier, Style};

/// Seconds after the first word arrives before the torpedoes reach the port.
const TORPEDO_S: f32 = 0.6;
/// Seconds the fire takes to run out to the edges of the view, and to burn
/// out after.
const BLAST_S: f32 = 1.4;
const EMBERS_S: f32 = 1.2;
/// Seconds the debris keeps drifting after the fire is out.
const DEBRIS_S: f32 = 4.0;
/// Panel rungs down the trench at once, and how many a second come at the
/// reader: this is a dive at full throttle, not a cruise.
const RUNGS: usize = 10;
const TRENCH_SPEED: f32 = 3.4;
/// One cell in this many is a star.
const STAR_DENSITY: u64 = 23;
/// The fire, faint to white-hot.
const FIRE: &[char] = &['.', ':', '*', '#', '%', '@'];

type Put<'a> = &'a mut dyn FnMut(usize, usize, char, Style);

fn hash(a: usize, b: usize) -> u64 {
    let mut h = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (b as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 29)
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

fn dim(color: Color, k: f32) -> Color {
    mix(Color::Rgb(0, 0, 0), color, k)
}

fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

const HULL: Color = Color::Rgb(128, 134, 156);
const WALL: Color = Color::Rgb(150, 156, 180);
const FLOOR: Color = Color::Rgb(84, 92, 128);

/// Where everything sits in a sky `width` by `height`.
#[derive(Debug, Clone, Copy)]
struct Scene {
    width: usize,
    height: usize,
    /// The exhaust port, on the horizon: the point the trench runs to.
    vx: f32,
    vy: f32,
    /// How far the limb curves down from the horizon at the edges.
    curve: f32,
    /// The trench at the bottom of the view: its floor's left and right
    /// edges and the row the surface is at there; the floor itself is off
    /// the bottom.
    ml: f32,
    mr: f32,
    top: f32,
    bottom: f32,
}

impl Scene {
    fn fit(width: usize, height: usize) -> Self {
        let (w, h) = (width as f32, height as f32);
        let vx = w * 0.5;
        let vy = (h * 0.3).max(1.0);
        let half = w * 0.28;
        Self {
            width,
            height,
            vx,
            vy,
            curve: (h * 0.14).max(1.0),
            ml: vx - half,
            mr: vx + half,
            top: vy + (h - 0.5 - vy) * 0.55,
            bottom: h - 0.5,
        }
    }

    /// The row the limb is at over column `x`: highest in the middle,
    /// falling away at the sides as the surface curves out of view.
    fn horizon(&self, x: f32) -> f32 {
        let k = (x - self.vx) / (self.width as f32 / 2.0);
        self.vy - 0.5 + self.curve * k * k
    }

    /// Depth on the surface plane at row `y`: 0 at the horizon, 1 where
    /// the trench's rim reaches the row the view runs out at, and beyond
    /// for the rows under it.
    fn plane_depth(&self, y: f32) -> f32 {
        (y - self.vy) / (self.top - self.vy)
    }

    /// The trench rim at plane depth `s`: the walls are vertical, so the rim
    /// stands straight over the floor edge at the same depth.
    fn rim(&self, s: f32) -> (f32, f32) {
        (
            self.vx + (self.ml - self.vx) * s,
            self.vx + (self.mr - self.vx) * s,
        )
    }

    /// The trench at floor depth `s`: 0 at the port, 1 at the bottom row.
    /// Left and right floor edges, floor row and rim row.
    fn floor(&self, s: f32) -> (f32, f32, f32, f32) {
        (
            self.vx + (self.ml - self.vx) * s,
            self.vx + (self.mr - self.vx) * s,
            self.vy + (self.bottom - self.vy) * s,
            self.vy + (self.top - self.vy) * s,
        )
    }

    fn plot(&self, put: Put<'_>, x: f32, y: f32, c: char, style: Style) {
        if x < 0.0 || y < 0.0 {
            return;
        }
        let (x, y) = (x.round() as usize, y.round() as usize);
        if x < self.width && y < self.height {
            put(x, y, c, style);
        }
    }
}

/// Draw the run into a sky `width` by `height` as it stands at `ms`.
/// `boom_ms` is when the model's first word arrived: the moment the shot
/// went in, from which the station has seconds left.
pub fn frame(
    seed: usize,
    width: usize,
    height: usize,
    ms: u64,
    boom_ms: Option<u64>,
    put: Put<'_>,
) {
    if width < 16 || height < 4 {
        return;
    }
    let t = ms as f32 / 1000.0;
    let since = boom_ms.map(|b| ms.saturating_sub(b) as f32 / 1000.0);
    let blast = since.map(|s| s - TORPEDO_S).filter(|b| *b >= 0.0);
    let scene = Scene::fit(width, height);
    draw_stars(&scene, seed, t, blast, put);
    draw_station(&scene, t, since, blast, put);
    if let Some(blast) = blast {
        draw_blast(&scene, blast, put);
    }
    draw_fighters(&scene, seed, t, since, put);
}

/// Stars above the limb; and, once the fire has passed, where the station
/// was too.
fn draw_stars(scene: &Scene, seed: usize, t: f32, blast: Option<f32>, put: Put<'_>) {
    let bounds = blast.map(|b| fire_bounds(scene, b));
    for y in 0..scene.height {
        for x in 0..scene.width {
            let (xf, yf) = (x as f32, y as f32);
            if yf >= scene.horizon(xf)
                && !bounds.is_some_and(|(inner, _, _)| distance(scene, xf, yf) < inner)
            {
                continue;
            }
            let h = hash(x, y.wrapping_add(seed.wrapping_mul(131)));
            if h % STAR_DENSITY != 0 {
                continue;
            }
            let period = 1.5 + ((h >> 8) % 100) as f32 / 40.0;
            let phase = ((h >> 16) % 628) as f32 / 100.0;
            let bright = 0.5 + 0.5 * (t * std::f32::consts::TAU / period + phase).sin();
            let glyph = match bright {
                b if b > 0.8 => '+',
                b if b > 0.45 => '*',
                _ => '.',
            };
            put(
                x,
                y,
                glyph,
                fg(mix(
                    Color::Rgb(70, 70, 110),
                    Color::Rgb(230, 230, 255),
                    bright,
                )),
            );
        }
    }
}

/// The station's surface and the trench cut into it, as far as the limb;
/// during the blast, only what the fire has not reached yet.
fn draw_station(scene: &Scene, t: f32, since: Option<f32>, blast: Option<f32>, put: Put<'_>) {
    // As the torpedoes fly the hull brightens with what is coming.
    let glow = since.map_or(0.0, |s| (s / TORPEDO_S).clamp(0.0, 1.0) * 0.4);
    let lit = |color: Color, k: f32| fg(mix(dim(color, k), Color::Rgb(255, 255, 255), glow));
    let front = blast.map(|b| fire_bounds(scene, b).1);
    let reached = |x: f32, y: f32| front.is_some_and(|front| distance(scene, x, y) < front);
    for y in 0..scene.height {
        let yf = y as f32;
        let (rl, rr) = scene.rim(scene.plane_depth(yf).max(0.0));
        let (fl, fr, _, _) = scene.floor(((yf - scene.vy) / (scene.bottom - scene.vy)).max(0.0));
        for x in 0..scene.width {
            let xf = x as f32;
            let horizon = scene.horizon(xf);
            if yf < horizon || reached(xf, yf) {
                continue;
            }
            // Nearer is brighter: the surface falls into shade towards the limb.
            let near = ((yf - scene.vy) / (scene.bottom - scene.vy)).clamp(0.0, 1.0);
            let k = 0.3 + 0.7 * near;
            if yf - horizon < 1.0 {
                put(x, y, '-', lit(HULL, 0.9));
                continue;
            }
            let inside = yf >= scene.vy && xf > rl && xf < rr;
            if !inside {
                // Open hull: panel seams and the odd light, sparse and faint.
                let h = hash(x, y);
                let glyph = match h % 7 {
                    0 => '.',
                    1 if near > 0.5 => ':',
                    _ => continue,
                };
                put(x, y, glyph, lit(HULL, k * 0.7));
                continue;
            }
            if xf >= fl && xf <= fr {
                // The floor, deep in shadow with its markings.
                if hash(x, y.wrapping_add(7)) % 4 == 0 {
                    put(x, y, '.', lit(FLOOR, k * 0.6));
                }
                continue;
            }
            // The walls, ribbed.
            match hash(x, y.wrapping_add(13)) % 4 {
                0 => put(x, y, ':', lit(WALL, k * 0.5)),
                1 => put(x, y, '|', lit(WALL, k * 0.55)),
                _ => {}
            }
        }
    }
    let port = (scene.vx, scene.vy);
    let clip = |scene: &Scene, put: Put<'_>, from: (f32, f32), to: (f32, f32), style: Style| {
        // Edge lines stop where the fire has got to.
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let steps = dx.abs().max(dy.abs()).ceil().max(1.0);
        let glyph = if dx.abs() < 0.45 * dy.abs() {
            '|'
        } else if dx.abs() > 3.5 * dy.abs() {
            '-'
        } else if dx * dy > 0.0 {
            '\\'
        } else {
            '/'
        };
        for i in 0..=steps as usize {
            let k = i as f32 / steps;
            let (x, y) = (from.0 + dx * k, from.1 + dy * k);
            if !reached(x, y) {
                scene.plot(put, x, y, glyph, style);
            }
        }
    };
    // The edges of the trench: rim and floor, sharp lines running to the port.
    let (ml, mr) = (scene.ml, scene.mr);
    clip(scene, put, port, (ml, scene.top), lit(WALL, 0.95));
    clip(scene, put, port, (mr, scene.top), lit(WALL, 0.95));
    clip(scene, put, port, (ml, scene.bottom), lit(FLOOR, 1.0));
    clip(scene, put, port, (mr, scene.bottom), lit(FLOOR, 1.0));
    // Seams down the walls, spaced for perspective and coming at the reader,
    // with a turret on the rim at every other one.
    let phase = (t * TRENCH_SPEED).fract();
    for i in 0..RUNGS {
        let p = (i as f32 + phase) / RUNGS as f32;
        let s = p.powf(2.2);
        if s < 0.05 {
            continue;
        }
        let (xl, xr, fy, ty) = scene.floor(s);
        let k = 0.4 + 0.6 * s;
        let seam = lit(WALL, k);
        clip(scene, put, (xl, ty), (xl, fy), seam);
        clip(scene, put, (xr, ty), (xr, fy), seam);
        if i % 2 == 0 && ty > scene.vy + 1.0 {
            let turret = lit(HULL, k);
            if !reached(xl - 2.0, ty - 1.0) {
                scene.plot(put, xl - 2.0, ty - 1.0, 'Y', turret);
            }
            if !reached(xr + 2.0, ty - 1.0) {
                scene.plot(put, xr + 2.0, ty - 1.0, 'Y', turret);
            }
        }
    }
    // The exhaust port, small and hard to hit, until it is.
    if since.is_none_or(|s| s < TORPEDO_S) {
        let blink = (t * 4.0).sin() > 0.0;
        scene.plot(
            put,
            scene.vx,
            scene.vy,
            'o',
            fg(if blink {
                Color::Rgb(255, 120, 90)
            } else {
                Color::Rgb(150, 70, 60)
            })
            .add_modifier(Modifier::BOLD),
        );
    }
}

/// How far a cell is from the port, in columns with rows counting double:
/// a cell is twice as tall as it is wide, and the fire spreads as a circle.
fn distance(scene: &Scene, x: f32, y: f32) -> f32 {
    ((x - scene.vx).powi(2) + (2.0 * (y - scene.vy)).powi(2)).sqrt()
}

/// The fire `blast` seconds in: a shell running out from the port, its
/// inner edge following the front once the middle has burned out. Returns
/// the inner edge, the front, and the distance to the farthest corner.
fn fire_bounds(scene: &Scene, blast: f32) -> (f32, f32, f32) {
    let (w, h) = (scene.width as f32, scene.height as f32);
    let reach = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)]
        .into_iter()
        .map(|(x, y)| distance(scene, x, y))
        .fold(0.0, f32::max);
    let front = blast / BLAST_S * reach;
    let inner = ((blast - BLAST_S * 0.45) / BLAST_S).max(0.0) * reach * 1.2;
    (inner, front, reach)
}

/// The station going up, `blast` seconds after the torpedoes went in: a
/// shell of fire running out from the port over everything, hottest at its
/// front and burning out behind, a shock ring racing away along the horizon,
/// and debris thrown clear.
fn draw_blast(scene: &Scene, blast: f32, put: Put<'_>) {
    let w = scene.width as f32;
    let (inner, front, reach) = fire_bounds(scene, blast);
    if inner < reach {
        for y in 0..scene.height {
            for x in 0..scene.width {
                let (xf, yf) = (x as f32, y as f32);
                let d = distance(scene, xf, yf);
                if d > front {
                    // Not reached yet: the hull ahead of the fire flashes white.
                    if yf >= scene.horizon(xf) {
                        let flash = (1.0 - (d - front) / 12.0).clamp(0.0, 1.0);
                        if flash > 0.0 && hash(x, y) % 3 != 0 {
                            let glyph = if flash > 0.6 { '#' } else { '+' };
                            put(
                                x,
                                y,
                                glyph,
                                fg(mix(HULL, Color::Rgb(255, 255, 255), flash))
                                    .add_modifier(Modifier::BOLD),
                            );
                        }
                    }
                    continue;
                }
                if d < inner {
                    continue;
                }
                // In the shell: hottest at the front, thinning behind it,
                // with a flicker of its own.
                let across = (d - inner) / (front - inner).max(1.0);
                let flicker =
                    (hash(x, y.wrapping_add((blast * 12.0) as usize)) % 100) as f32 / 100.0;
                let heat = ((0.3 + 0.7 * across) * (0.7 + 0.6 * flicker)).clamp(0.0, 1.0);
                if heat < 0.12 {
                    continue;
                }
                let glyph = FIRE[((heat * FIRE.len() as f32) as usize).min(FIRE.len() - 1)];
                let color = if heat > 0.8 {
                    mix(
                        Color::Rgb(255, 230, 120),
                        Color::Rgb(255, 255, 255),
                        (heat - 0.8) * 5.0,
                    )
                } else if heat > 0.45 {
                    mix(
                        Color::Rgb(255, 110, 40),
                        Color::Rgb(255, 230, 120),
                        (heat - 0.45) / 0.35,
                    )
                } else {
                    mix(
                        Color::Rgb(110, 30, 20),
                        Color::Rgb(255, 110, 40),
                        heat / 0.45,
                    )
                };
                let mut style = fg(color);
                if heat > 0.45 {
                    style = style.add_modifier(Modifier::BOLD);
                }
                put(x, y, glyph, style);
            }
        }
    }
    // The ring: a flat shock racing out along the horizon, past the edges.
    let ring = blast / BLAST_S * w * 0.8;
    let fade = 1.0 - (blast / (BLAST_S + EMBERS_S)).clamp(0.0, 1.0);
    if fade > 0.0 {
        for dx in 0..scene.width {
            let d = (dx as f32 - scene.vx).abs();
            if d < ring && d > ring - 5.0 {
                let color = mix(Color::Rgb(60, 80, 140), Color::Rgb(220, 240, 255), fade);
                let glyph = if fade > 0.5 { '=' } else { '-' };
                scene.plot(
                    put,
                    dx as f32,
                    scene.vy,
                    glyph,
                    fg(color).add_modifier(Modifier::BOLD),
                );
            }
        }
    }
    // Debris thrown clear, drifting on after the fire is out.
    if (0.3..BLAST_S + EMBERS_S + DEBRIS_S).contains(&blast) {
        let fade = 1.0 - ((blast - BLAST_S - EMBERS_S) / DEBRIS_S).clamp(0.0, 1.0);
        for i in 0..48usize {
            let h = hash(i, 77);
            let angle = (h % 628) as f32 / 100.0;
            let speed = 6.0 + ((h >> 8) % 100) as f32 / 8.0;
            let d = speed * blast;
            let x = scene.vx + angle.cos() * d;
            let y = scene.vy - angle.sin() * d * 0.5;
            let glyph = if (h >> 16) % 3 == 0 { ':' } else { '.' };
            scene.plot(
                put,
                x,
                y,
                glyph,
                fg(dim(Color::Rgb(180, 170, 160), fade * 0.8)),
            );
        }
    }
}

/// A sprite drawn about its middle, one string per row.
fn sprite(
    scene: &Scene,
    put: Put<'_>,
    x: f32,
    y: f32,
    rows: &[&str],
    color: impl Fn(char) -> Color,
) {
    let half_rows = rows.len() as f32 / 2.0;
    for (r, row) in rows.iter().enumerate() {
        let half = row.chars().count() as f32 / 2.0;
        for (c, ch) in row.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            scene.plot(
                put,
                x - half + c as f32 + 0.5,
                y - half_rows + r as f32 + 0.5,
                ch,
                fg(color(ch)).add_modifier(Modifier::BOLD),
            );
        }
    }
}

/// A bolt of laser fire a fraction `k` of the way from one point to another.
fn bolt(scene: &Scene, put: Put<'_>, from: (f32, f32), to: (f32, f32), k: f32, color: Color) {
    for (i, tail) in [(0.0, 1.0), (-0.04, 0.6)] {
        let kk = k + i;
        if !(0.0..=1.0).contains(&kk) {
            continue;
        }
        let x = from.0 + (to.0 - from.0) * kk;
        let y = from.1 + (to.1 - from.1) * kk;
        let glyph = if (to.0 - from.0).abs() > 2.0 * (to.1 - from.1).abs() {
            '-'
        } else {
            '*'
        };
        scene.plot(
            put,
            x,
            y,
            glyph,
            fg(dim(color, tail)).add_modifier(Modifier::BOLD),
        );
    }
}

/// The X-wing from behind, S-foils open.
const X_WING: [&str; 3] = ["\\  |  /", ">=[O]=<", "/  |  \\"];
/// A TIE fighter from behind: two panels and the pod between.
const TIE: [&str; 1] = ["|=o=|"];

fn draw_fighters(scene: &Scene, seed: usize, t: f32, since: Option<f32>, put: Put<'_>) {
    let red = Color::Rgb(255, 70, 60);
    let green = Color::Rgb(90, 255, 110);
    let grey = Color::Rgb(200, 205, 220);
    let after = since.map_or(0.0, |s| (s - TORPEDO_S).max(0.0));
    // The X-wing, halfway down the trench, weaving between the walls and
    // riding up and down; once the shot is away it climbs out and peels off.
    let (xl, xr, fy, ty) = scene.floor(0.5);
    let phase = seed as f32 * 0.7;
    let u = 0.5 + 0.28 * (t * 2.1 + phase).sin();
    let hv = 0.5 + 0.18 * (t * 1.5 + phase).sin();
    let xw = xl + (xr - xl) * u + after * 4.0;
    let yw = fy - (fy - ty) * hv - after * 5.0;
    let port = (scene.vx, scene.vy);
    // Two TIE fighters on its tail, nearer the reader.
    let ties: Vec<(f32, f32)> = [-1.0f32, 1.0]
        .into_iter()
        .enumerate()
        .map(|(i, side)| {
            let (xl, xr, fy, ty) = scene.floor(0.68);
            let u = 0.5 + side * 0.22 + 0.06 * (t * 3.1 + i as f32).sin();
            let hv = 0.55 + side * 0.1 + 0.08 * (t * 1.9 + i as f32 * 2.0).cos();
            (xl + (xr - xl) * u, fy - (fy - ty) * hv)
        })
        .collect();
    // Fire first, fighters over it: a bolt leaving a cannon or landing on a
    // hull must not eat the ship.
    match since {
        None => {
            // Cannon fire down the trench in bursts; the TIEs and the rim
            // turrets firing back.
            let cycle = (t * 1.8 + phase).fract();
            if cycle < 0.4 {
                let k = cycle / 0.4;
                bolt(scene, put, (xw - 3.0, yw), port, k, red);
                bolt(scene, put, (xw + 3.0, yw), port, k, red);
            }
            for (i, &(x, y)) in ties.iter().enumerate() {
                let cycle = (t * 1.6 + i as f32 * 0.37 + phase).fract();
                if cycle < 0.35 {
                    bolt(scene, put, (x, y), (xw, yw), cycle / 0.35, green);
                }
            }
            for (i, s) in [0.25f32, 0.4, 0.6].into_iter().enumerate() {
                let (xl, xr, _, ty) = scene.floor(s);
                let side = if i % 2 == 0 { xl - 2.0 } else { xr + 2.0 };
                let cycle = (t * 1.3 + i as f32 * 0.41 + phase).fract();
                if cycle < 0.3 {
                    bolt(
                        scene,
                        put,
                        (side, ty - 1.0),
                        (xw + (xw - side) * 0.8, yw - 3.0),
                        cycle / 0.3,
                        green,
                    );
                }
            }
        }
        Some(s) if s < TORPEDO_S => {
            // The torpedoes, curving down into the port.
            let k = s / TORPEDO_S;
            let dip = (k * std::f32::consts::PI).sin() * 1.5;
            for side in [-3.0, 3.0] {
                let x = xw + side + (port.0 - xw - side) * k;
                let y = yw + (port.1 - yw) * k + dip;
                scene.plot(
                    put,
                    x,
                    y,
                    '*',
                    fg(mix(red, Color::Rgb(255, 220, 120), k)).add_modifier(Modifier::BOLD),
                );
            }
        }
        _ => {}
    }
    for &(x, y) in &ties {
        if let Some(gone) = since.map(|s| s - TORPEDO_S).filter(|g| *g >= 0.0) {
            // The blast takes them: a burst, then nothing.
            if gone < 0.5 {
                let glyph = if gone < 0.25 { '*' } else { '+' };
                let color = mix(
                    Color::Rgb(255, 230, 120),
                    Color::Rgb(120, 60, 40),
                    gone / 0.5,
                );
                for (dx, dy) in [(0.0, 0.0), (-2.0, 0.0), (2.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                    scene.plot(
                        put,
                        x + dx * (1.0 + gone * 4.0),
                        y + dy * (1.0 + gone * 2.0),
                        glyph,
                        fg(color).add_modifier(Modifier::BOLD),
                    );
                }
            }
            continue;
        }
        sprite(scene, put, x, y, &TIE, |c| match c {
            'o' => Color::Rgb(160, 200, 230),
            _ => Color::Rgb(130, 135, 160),
        });
    }
    if yw > -2.0 {
        sprite(scene, put, xw, yw, &X_WING, |c| match c {
            'O' => Color::Rgb(255, 255, 255),
            '=' => Color::Rgb(255, 150, 90),
            _ => grey,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: usize, height: usize, ms: u64, boom: Option<u64>) -> String {
        let mut grid = vec![vec![' '; width]; height];
        frame(0, width, height, ms, boom, &mut |x, y, c, _| grid[y][x] = c);
        grid.into_iter()
            .map(|row| row.into_iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_run_goes_down_the_trench_until_the_shot_goes_in() {
        let before = picture(120, 24, 3_000, None);
        assert!(
            before.contains(">=[O]=<"),
            "the X-wing is in the trench:\n{before}"
        );
        assert!(
            before.contains("|=o=|"),
            "the TIEs are on its tail:\n{before}"
        );
        assert!(before.contains('Y'), "turrets line the rim:\n{before}");
        assert!(before.contains('o'), "the port is ahead:\n{before}");
        // A second after the first word, the fire is spreading and the TIEs
        // are gone.
        let boom = Some(10_000);
        let burning = picture(120, 24, 10_000 + (TORPEDO_S * 1000.0) as u64 + 900, boom);
        assert!(!burning.contains("|=o=|"), "the TIEs are taken:\n{burning}");
        assert!(
            burning.matches(['#', '%', '@']).count() > 60,
            "fire fills the view:\n{burning}"
        );
        // Long after, only stars and drifting debris remain.
        let after = picture(120, 24, 10_000 + 12_000, boom);
        assert!(
            !after.contains(['#', '%', '@', '|']),
            "the station is gone:\n{after}"
        );
        assert!(after.contains('*'), "stars where it was:\n{after}");
        // Every frame stays inside the box.
        for ms in (0..12_000).step_by(700) {
            picture(40, 6, ms, Some(3_000));
            picture(16, 4, ms, None);
        }
    }
}
