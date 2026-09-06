//! Something to look at while the model reads the diff. Prefill on a local
//! model is a silent minute: nothing streams back, so the message pane is an
//! empty box with a timer on it. This fills the box with a small animated
//! scene of what is going on, filling the whole box: the language's mascot,
//! a small three-dimensional figure turning on the spot, feeds the diff token
//! by token into a neural network that pulses as it thinks, against one of a
//! few backgrounds (code raining down, a starry night, the diff scrolling
//! past), with a caption reporting what the model is up to. Everything moves
//! on the millisecond clock, so it is as smooth as the terminal can draw.
//! None of it is true, all of it is more fun than a timer.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::solid;

/// The language most of the staged files are written in, judged by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Kotlin,
    Java,
    Go,
    Python,
    JavaScript,
    TypeScript,
    Other,
}

impl Language {
    fn of_path(path: &str) -> Option<Self> {
        let ext = path.rsplit_once('.')?.1;
        Some(match ext {
            "rs" => Self::Rust,
            "kt" | "kts" => Self::Kotlin,
            "java" => Self::Java,
            "go" => Self::Go,
            "py" => Self::Python,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "tsx" | "mts" => Self::TypeScript,
            _ => return None,
        })
    }

    /// The language with the most files among `paths`. A commit touching
    /// three Kotlin files and one Markdown file is a Kotlin commit; one that
    /// touches nothing recognisable gets the generic scene.
    pub fn dominant<'a>(paths: impl IntoIterator<Item = &'a str>) -> Self {
        let mut counts: Vec<(Language, usize)> = Vec::new();
        for lang in paths.into_iter().filter_map(Self::of_path) {
            match counts.iter_mut().find(|(l, _)| *l == lang) {
                Some((_, n)) => *n += 1,
                None => counts.push((lang, 1)),
            }
        }
        // First seen wins a tie, so the pick is stable across redraws.
        counts
            .iter()
            .fold(
                None,
                |best: Option<(Language, usize)>, &(l, n)| match best {
                    Some((_, m)) if m >= n => best,
                    _ => Some((l, n)),
                },
            )
            .map_or(Self::Other, |(l, _)| l)
    }

    /// One line about what the model is doing that only makes sense for this
    /// language. Takes its turn among the general captions.
    fn quip(self) -> &'static str {
        match self {
            Self::Rust => "arguing with the borrow checker",
            Self::Kotlin => "unwrapping a nullable, carefully",
            Self::Java => "instantiating an AbstractCommitMessageFactory",
            Self::Go => "checking if err != nil",
            Self::Python => "counting the indentation",
            Self::JavaScript => "awaiting a promise, hopefully",
            Self::TypeScript => "narrowing the type of \"commit\"",
            Self::Other => "reading the diff twice",
        }
    }

    /// The mascot as a three-dimensional figure at time `t` seconds, ready
    /// to be turned and lit.
    fn figure(self, t: f32) -> Vec<solid::Part> {
        match self {
            Self::Rust => solid::ferris(t),
            Self::Kotlin => solid::kodee(t),
            Self::Java => solid::duke(t),
            Self::Go => solid::gopher(t),
            Self::Python => solid::snake(t),
            Self::JavaScript => solid::monitor(t, Color::Rgb(247, 223, 30), 'J'),
            Self::TypeScript => solid::monitor(t, Color::Rgb(90, 160, 240), 'T'),
            Self::Other => solid::robot(t),
        }
    }
}

/// What the model is supposedly doing, one line at a time.
const CAPTIONS: &[&str] = &[
    "tokenizing the diff",
    "attending to every hunk at once",
    "weighing 'fix' against 'refactor'",
    "consulting the conventions",
    "picking a verb",
    "resisting the word 'various'",
    "counting to 72 characters",
    "almost there, probably",
];

/// The backgrounds a wait can be set against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backdrop {
    /// Code glyphs raining down.
    Rain,
    /// Stars twinkling under a moon, with the odd shooting star.
    Night,
    /// The diff itself scrolling up past the reader.
    Diff,
}

const BACKDROPS: [Backdrop; 3] = [Backdrop::Rain, Backdrop::Night, Backdrop::Diff];

/// The figure's picture is this many rows tall at most, and never fewer than
/// the minimum; its width follows from the cell shape.
const FIGURE_MAX_ROWS: usize = 22;
const FIGURE_MIN_ROWS: usize = 8;
/// Radians the figure turns per second: one full turn in about seven.
const TURN_RATE: f32 = 0.9;
/// Milliseconds a caption stays up before the next one.
const CAPTION_MS: u64 = 1_700;
/// The stream of tokens between the mascot and the network, as a repeating
/// pattern of dense, faint, and empty cells.
const TOKEN_PATTERN: &str = "\u{25aa}\u{25ab}\u{25aa}\u{b7} \u{25ab}\u{25aa}\u{25aa}\u{b7}\u{25ab} \u{25aa}\u{b7}\u{25ab}\u{25aa} \u{b7}\u{25aa}\u{25ab}\u{25aa}\u{25aa} \u{b7}";
/// Cells the stream moves per second.
const STREAM_SPEED: f32 = 12.0;
/// The narrowest the stream may be before the pipeline is dropped.
const MIN_STREAM_WIDTH: usize = 10;
/// Rows of tokens in the stream.
const STREAM_ROWS: usize = 3;
/// Nodes per layer of the network, left to right; all odd so the wires run
/// through the middle node of every layer.
const LAYERS: [usize; 4] = [3, 5, 5, 3];
/// Rows between nodes in a layer, and cells between layers.
const NODE_STEP: usize = 2;
const LAYER_STEP: usize = 5;
/// Seconds for the wave of activity to cross the network once.
const WAVE_PERIOD: f32 = 1.4;
/// The glyphs raining down the background.
const RAIN_GLYPHS: &[char] = &[
    '{', '}', '(', ')', '[', ']', ';', '=', '<', '>', '+', '-', '*', '/', '&', '|', '!', '?', ':',
    '.', '0', '1', 'f', 'n', 'x',
];
/// How many columns out of this many carry rain.
const RAIN_DENSITY: u64 = 3;
const RAIN_TRAIL: usize = 7;
/// Rows a drop falls per second, at the slower of its two speeds.
const RAIN_SPEED: f32 = 9.0;
/// One cell in this many is a star.
const STAR_DENSITY: u64 = 14;
/// Milliseconds between shooting stars.
const SHOOTING_STAR_PERIOD_MS: u64 = 9_000;
/// Rows per second the diff scrolls.
const DIFF_SPEED: f32 = 4.0;

type Cell = (char, Style);

/// A grid of styled cells the scene is painted onto, then read out as rows.
struct Canvas {
    cells: Vec<Vec<Cell>>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            cells: vec![vec![(' ', Style::default()); width]; height],
        }
    }

    fn put(&mut self, x: usize, y: usize, c: char, style: Style) {
        if let Some(cell) = self.cells.get_mut(y).and_then(|row| row.get_mut(x)) {
            *cell = (c, style);
        }
    }

    /// Blank a rectangle, for a figure to be drawn over the backdrop.
    fn clear(&mut self, x: usize, y: usize, w: usize, h: usize) {
        for row in y..y + h {
            for col in x..x + w {
                self.put(col, row, ' ', Style::default());
            }
        }
    }

    fn text(&mut self, x: usize, y: usize, text: &str, style: Style) {
        for (i, c) in text.chars().enumerate() {
            self.put(x + i, y, c, style);
        }
    }

    fn lines(self) -> Vec<Line<'static>> {
        self.cells
            .into_iter()
            .map(|row| {
                // Runs of one style become one span; a span per cell would
                // be thousands of allocations a frame for nothing.
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut run = String::new();
                let mut run_style = Style::default();
                for (c, style) in row {
                    if style != run_style && !run.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut run), run_style));
                    }
                    run_style = style;
                    run.push(c);
                }
                if !run.is_empty() {
                    spans.push(Span::styled(run, run_style));
                }
                Line::from(spans)
            })
            .collect()
    }
}

/// A small, fast, deterministic hash for scattering stars, rain and glyphs.
fn hash(a: usize, b: usize) -> u64 {
    let mut h = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (b as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 29)
}

/// A colour scaled towards black by `k` in 0..=1.
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

/// The colour `k` of the way from `a` to `b`.
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

/// A fully saturated colour at `hue` turns round the wheel.
fn hue(hue: f32) -> Color {
    let h = (hue.rem_euclid(1.0)) * 6.0;
    let x = (1.0 - ((h % 2.0) - 1.0).abs()) * 255.0;
    let x = x as u8;
    match h as u8 {
        0 => Color::Rgb(255, x, 60),
        1 => Color::Rgb(x, 255, 60),
        2 => Color::Rgb(60, 255, x),
        3 => Color::Rgb(60, x, 255),
        4 => Color::Rgb(x, 60, 255),
        _ => Color::Rgb(255, 60, x),
    }
}

/// Where the parts go in a box of one size.
struct Plan {
    width: usize,
    height: usize,
    mascot_w: usize,
    mascot_h: usize,
    mascot_x: usize,
    /// The row the mascot stands on and the backdrop stops at.
    ground: usize,
    /// The stream and network, if there is room for them beside the mascot:
    /// where the stream starts, how wide it is, where the network starts.
    pipeline: Option<(usize, usize, usize)>,
    caption: bool,
}

impl Plan {
    fn fit(lang: Language, width: usize, height: usize) -> Option<Self> {
        let caption_w = CAPTIONS
            .iter()
            .chain(std::iter::once(&lang.quip()))
            .map(|text| text.chars().count() + 3)
            .max()
            .unwrap_or(0);
        let caption = caption_w <= width;
        // Ground line, blank, caption below the figure.
        let below = 1 + if caption { 2 } else { 0 };
        if FIGURE_MIN_ROWS + below > height {
            return None;
        }
        let ground = height - below;
        // The figure takes as many rows as it can up to its limit, and is a
        // little over twice as wide as tall since cells are twice as tall as
        // they are wide.
        let mascot_h = ground.min(FIGURE_MAX_ROWS);
        let mascot_w = mascot_h * 2 + 4;
        if mascot_w > width {
            return None;
        }

        let network_w = (LAYERS.len() - 1) * LAYER_STEP + 1;
        let gap = 3;
        let pipeline = (mascot_w + gap + MIN_STREAM_WIDTH + gap + network_w <= width).then(|| {
            let stream_x = mascot_w + gap;
            let network_x = width - network_w;
            (stream_x, network_x - gap - stream_x, network_x)
        });
        let mascot_x = if pipeline.is_some() {
            0
        } else {
            (width - mascot_w) / 2
        };
        Some(Self {
            width,
            height,
            mascot_w,
            mascot_h,
            mascot_x,
            ground,
            pipeline,
            caption,
        })
    }
}

/// The scene at clock `ms`, filling a box `width` by `height` cells. `seed`
/// picks the backdrop, and stays the same for one wait so the picture does
/// not change under the reader. The mascot stands on a ground line at the
/// bottom left, tokens stream from it across whatever width is left into the
/// network at the right, and the caption sits under the ground. Every row is
/// the full width so centring the block does not shift anything as the parts
/// animate. Empty when the box cannot hold the figure: a crab squeezed into
/// two rows is worse than nothing.
pub fn scene(lang: Language, seed: usize, ms: u64, width: u16, height: u16) -> Vec<Line<'static>> {
    let Some(plan) = Plan::fit(lang, width as usize, height as usize) else {
        return Vec::new();
    };
    let mut canvas = Canvas::new(plan.width, plan.height);
    let t = ms as f32 / 1000.0;

    match BACKDROPS[seed % BACKDROPS.len()] {
        Backdrop::Rain => draw_rain(&mut canvas, plan.width, plan.ground, t),
        Backdrop::Night => draw_night(&mut canvas, plan.width, plan.ground, ms),
        Backdrop::Diff => draw_diff(&mut canvas, plan.width, plan.ground, t),
    }
    canvas.text(
        0,
        plan.ground,
        &"\u{2594}".repeat(plan.width),
        Style::default().fg(Color::Rgb(90, 92, 120)),
    );

    let mascot_y = plan.ground - plan.mascot_h;
    // The figures stand in front of the backdrop, so each footprint is
    // cleared first: stars showing through a crab read as holes in the crab.
    canvas.clear(plan.mascot_x, mascot_y, plan.mascot_w, plan.mascot_h);
    draw_figure(
        &mut canvas,
        lang,
        plan.mascot_x,
        mascot_y,
        plan.mascot_w,
        plan.mascot_h,
        t,
    );

    if let Some((stream_x, stream_w, network_x)) = plan.pipeline {
        let stream_y = (mascot_y + plan.mascot_h / 2).saturating_sub(STREAM_ROWS / 2);
        canvas.clear(stream_x, stream_y, stream_w, STREAM_ROWS);
        draw_stream(&mut canvas, stream_x, stream_y, stream_w, t);
        let network_w = (LAYERS.len() - 1) * LAYER_STEP + 1;
        let network_h = (LAYERS.iter().copied().max().unwrap_or(1) - 1) * NODE_STEP + 1;
        let network_y = plan.ground.saturating_sub(network_h) / 2;
        canvas.clear(
            network_x.saturating_sub(1),
            network_y.saturating_sub(1),
            network_w + 2,
            network_h + 2,
        );
        draw_network(&mut canvas, network_x, network_y, t);
    }

    if plan.caption {
        let text = caption(lang, ms);
        let x = (plan.width - text.chars().count()) / 2;
        // The caption drifts round the colour wheel, a full turn a minute.
        canvas.text(
            x,
            plan.height - 1,
            &text,
            Style::default()
                .fg(hue(t / 60.0))
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        );
    }
    canvas.lines()
}

/// Code falling down the background above the ground: a third of the
/// columns carry a drop, each at its own offset, a bright head with a
/// fading tail of glyphs behind it.
fn draw_rain(canvas: &mut Canvas, width: usize, ground: usize, t: f32) {
    if ground == 0 {
        return;
    }
    let span = ground + RAIN_TRAIL;
    for x in 0..width {
        let seed = hash(x, 7);
        if seed % RAIN_DENSITY != 0 {
            continue;
        }
        // Some drops fall at one speed, some at half again, so the rain has
        // depth; the head's position is continuous, its cell the floor of it.
        let speed = RAIN_SPEED * (1.0 + 0.5 * ((seed >> 8) % 2) as f32);
        let fall = t * speed + ((seed >> 16) % 1_000) as f32;
        let head = fall as usize % span;
        let generation = fall as usize / span;
        for back in 0..RAIN_TRAIL {
            let Some(y) = head.checked_sub(back) else {
                break;
            };
            if y >= ground {
                continue;
            }
            let glyph = RAIN_GLYPHS[(hash(x, y + generation) % RAIN_GLYPHS.len() as u64) as usize];
            // Brightness falls off along the tail and flickers a little.
            let fade = 1.0 - back as f32 / RAIN_TRAIL as f32;
            let flicker = 0.85 + 0.15 * (t * 17.0 + x as f32).sin();
            let color = if back == 0 {
                Color::Rgb(170, 255, 200)
            } else {
                dim(Color::Rgb(70, 200, 120), fade * flicker)
            };
            let mut style = Style::default().fg(color);
            if back == 0 {
                style = style.add_modifier(Modifier::BOLD);
            }
            canvas.put(x, y, glyph, style);
        }
    }
}

/// A night sky: stars that brighten and fade at their own pace, a crescent
/// moon, and now and then a shooting star crossing from the top left.
fn draw_night(canvas: &mut Canvas, width: usize, ground: usize, ms: u64) {
    let t = ms as f32 / 1000.0;
    for y in 0..ground {
        for x in 0..width {
            let seed = hash(x, y);
            if seed % STAR_DENSITY != 0 {
                continue;
            }
            // Each star twinkles on its own period and phase; the glyph
            // follows the brightness so a star swells as it brightens.
            let period = 1.5 + ((seed >> 8) % 100) as f32 / 40.0;
            let phase = ((seed >> 16) % 628) as f32 / 100.0;
            let bright = 0.5 + 0.5 * (t * std::f32::consts::TAU / period + phase).sin();
            let glyph = match bright {
                b if b > 0.9 => '\u{2726}',
                b if b > 0.7 => '*',
                b if b > 0.4 => '+',
                _ => '\u{b7}',
            };
            let color = mix(Color::Rgb(90, 90, 140), Color::Rgb(255, 255, 225), bright);
            canvas.put(x, y, glyph, Style::default().fg(color));
        }
    }
    let moon = [
        r"   _.--.  ",
        r"  /  .' \ ",
        r" |  (    |",
        r"  \  '._/ ",
        r"   '--'   ",
    ];
    if width > 40 && ground > moon.len() + 1 {
        let moon_x = width.saturating_sub(moon[0].len() + (LAYERS.len() - 1) * LAYER_STEP + 8);
        let glow = 0.9 + 0.1 * (t * 0.7).sin();
        let style = Style::default()
            .fg(dim(Color::Rgb(250, 245, 205), glow))
            .add_modifier(Modifier::BOLD);
        for (i, row) in moon.iter().enumerate() {
            for (j, c) in row.chars().enumerate() {
                if c != ' ' {
                    canvas.put(moon_x + j, 1 + i, c, style);
                }
            }
        }
    }
    // The shooting star: a bright head with a fading tail, crossing two cells
    // right and one down every fifty milliseconds for the first part of the
    // period.
    let in_period = ms % SHOOTING_STAR_PERIOD_MS;
    let step = (in_period / 50) as usize;
    let start_x = ((ms / SHOOTING_STAR_PERIOD_MS) as usize * 37) % width.max(1);
    if step < ground.min(width / 2) {
        for back in 0..5 {
            let Some(s) = step.checked_sub(back) else {
                break;
            };
            let x = start_x + s * 2;
            let y = s;
            let (glyph, color) = match back {
                0 => ('\u{2726}', Color::Rgb(255, 255, 255)),
                1 => ('*', Color::Rgb(230, 230, 255)),
                _ => (
                    '\u{b7}',
                    dim(Color::Rgb(170, 170, 220), 1.0 - back as f32 / 6.0),
                ),
            };
            if y < ground {
                canvas.put(x, y, glyph, Style::default().fg(color));
            }
        }
    }
}

/// The diff scrolling up behind everything: lines of added and removed code
/// in the colours a diff is read in, drawn as runs of glyphs since the real
/// diff is the model's to read, not ours to show.
fn draw_diff(canvas: &mut Canvas, width: usize, ground: usize, t: f32) {
    let offset = (t * DIFF_SPEED) as usize;
    for y in 0..ground {
        let line = y + offset;
        let seed = hash(line, 3);
        let (sign, color) = match seed % 7 {
            0 | 1 => ('+', Color::Rgb(70, 160, 90)),
            2 => ('-', Color::Rgb(170, 70, 80)),
            3 => continue,
            _ => (' ', Color::Rgb(75, 78, 100)),
        };
        // Lines nearer the top have been read and fade; the newest arrive
        // bright at the bottom.
        let depth = 0.55 + 0.45 * (y as f32 / ground.max(1) as f32);
        let indent = 1 + (seed >> 8) as usize % 4 * 4;
        let len = 6 + (seed >> 16) as usize % (width * 7 / 10).max(1);
        let style = Style::default().fg(dim(color, depth));
        canvas.put(0, y, sign, style);
        let mut x = indent + 1;
        let end = (x + len).min(width.saturating_sub(1));
        while x < end {
            let word = 2 + (hash(line, x) % 6) as usize;
            for _ in 0..word {
                if x >= end {
                    break;
                }
                let glyph = RAIN_GLYPHS[(hash(line, x + 99) % RAIN_GLYPHS.len() as u64) as usize];
                canvas.put(x, y, glyph, style);
                x += 1;
            }
            x += 1;
        }
    }
}

/// The language's figure, turned and lit for the moment `t`, drawn into a
/// `w` by `h` window with its bottom on the ground. It turns steadily and
/// moves on its own clock, so at any frame rate it is where it should be.
fn draw_figure(
    canvas: &mut Canvas,
    lang: Language,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    t: f32,
) {
    let grid = solid::render(&lang.figure(t), t * TURN_RATE, w, h);
    for (r, row) in grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if let Some((ch, color)) = cell {
                canvas.put(
                    x + c,
                    y + r,
                    *ch,
                    Style::default().fg(*color).add_modifier(Modifier::BOLD),
                );
            }
        }
    }
}

/// Rows of tokens flowing right into the network, each row a little out of
/// phase with its neighbours so the flow reads as a river rather than a
/// conveyor belt. Colour cools along the way, and a shimmer runs down the
/// stream so it glitters as it moves.
fn draw_stream(canvas: &mut Canvas, x: usize, y: usize, width: usize, t: f32) {
    let pattern: Vec<char> = TOKEN_PATTERN.chars().collect();
    let n = pattern.len();
    let travelled = (t * STREAM_SPEED) as usize;
    for row in 0..STREAM_ROWS {
        let phase = row * 5;
        for col in 0..width.saturating_sub(1) {
            let c = pattern[(col + phase + n * 1_000 - travelled % n) % n];
            let along = col as f32 / width.max(1) as f32;
            let base = mix(Color::Rgb(120, 140, 255), Color::Rgb(140, 255, 255), along);
            let shimmer = 0.7 + 0.3 * (t * 6.0 - col as f32 * 0.4 + row as f32).sin();
            let style = match c {
                '\u{25aa}' => Style::default()
                    .fg(dim(base, shimmer))
                    .add_modifier(Modifier::BOLD),
                '\u{25ab}' => Style::default().fg(dim(base, shimmer * 0.8)),
                ' ' => continue,
                _ => Style::default().fg(Color::Rgb(90, 95, 130)),
            };
            canvas.put(x + col, y + row, c, style);
        }
        canvas.put(
            x + width - 1,
            y + row,
            '\u{25b8}',
            Style::default()
                .fg(Color::Rgb(140, 255, 255))
                .add_modifier(Modifier::BOLD),
        );
    }
}

/// The network: layers of nodes joined by wires. A wave of activity runs
/// through it left to right; a node glows in proportion to how close the
/// wave is, from grey through ember to white-hot, and the wires light as it
/// crosses them.
fn draw_network(canvas: &mut Canvas, x: usize, y: usize, t: f32) {
    let tallest = LAYERS.iter().copied().max().unwrap_or(1);
    let mid = y + (tallest - 1) * NODE_STEP / 2;
    // The wave's position in layers, wrapping with a gap before it returns.
    let span = LAYERS.len() as f32 + 1.5;
    let wave = (t / WAVE_PERIOD).fract() * span;
    let cold = Color::Rgb(100, 100, 135);
    let ember = Color::Rgb(255, 120, 70);
    let hot = Color::Rgb(255, 250, 170);
    for (layer, &count) in LAYERS.iter().enumerate() {
        let col_x = x + layer * LAYER_STEP;
        let top = y + (tallest - count) * NODE_STEP / 2;
        for n in 0..count {
            // Each node fires a little after its neighbour above.
            let here = layer as f32 + n as f32 * 0.12;
            let heat = (1.0 - (wave - here).abs() / 1.2).clamp(0.0, 1.0);
            let (glyph, color) = if heat > 0.75 {
                ('\u{25c9}', mix(ember, hot, (heat - 0.75) * 4.0))
            } else if heat > 0.3 {
                ('\u{25cf}', mix(cold, ember, (heat - 0.3) / 0.45))
            } else {
                ('\u{25cb}', mix(cold, cold, 0.0))
            };
            let mut style = Style::default().fg(color);
            if heat > 0.75 {
                style = style.add_modifier(Modifier::BOLD);
            }
            canvas.put(col_x, top + n * NODE_STEP, glyph, style);
        }
        if layer + 1 < LAYERS.len() {
            let here = layer as f32 + 0.5;
            let heat = (1.0 - (wave - here).abs() / 0.9).clamp(0.0, 1.0);
            let style = Style::default().fg(mix(Color::Rgb(80, 80, 110), hot, heat));
            for dx in 1..LAYER_STEP {
                canvas.put(col_x + dx, mid, '\u{2500}', style);
            }
        }
    }
}

/// The caption at clock `ms`: the general lines in turn, with this language's
/// own line slipped in among them, and a trailing ellipsis that grows.
fn caption(lang: Language, ms: u64) -> String {
    let slot = (ms / CAPTION_MS) as usize;
    let n = CAPTIONS.len() + 1;
    let text = match slot % n {
        i if i == n - 1 => lang.quip(),
        i => CAPTIONS[i],
    };
    let dots = (ms % CAPTION_MS * 4 / CAPTION_MS) as usize;
    format!("{text}{:<3}", ".".repeat(dots))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    const ALL: [Language; 8] = [
        Language::Rust,
        Language::Kotlin,
        Language::Java,
        Language::Go,
        Language::Python,
        Language::JavaScript,
        Language::TypeScript,
        Language::Other,
    ];

    #[test]
    fn the_language_with_most_files_wins() {
        let paths = ["README.md", "a/Main.kt", "b/Other.kt", "c/lib.rs"];
        assert_eq!(Language::dominant(paths), Language::Kotlin);
        assert_eq!(Language::dominant(["notes.txt"]), Language::Other);
        assert_eq!(Language::dominant([]), Language::Other);
    }

    #[test]
    fn every_scene_fills_the_box_it_was_given_and_holds_still() {
        for lang in ALL {
            for seed in 0..BACKDROPS.len() {
                for (w, h) in [(90u16, 16u16), (120, 40), (200, 60)] {
                    for frame in 0..40 {
                        let ms = frame * 230;
                        let lines = scene(lang, seed, ms, w, h);
                        assert_eq!(
                            lines.len(),
                            h as usize,
                            "{lang:?} fills {w}x{h} top to bottom"
                        );
                        for line in &lines {
                            assert_eq!(
                                line.width(),
                                w as usize,
                                "{lang:?} seed {seed} at {ms}ms fills {w} across: {}",
                                text_of(std::slice::from_ref(line))
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_scene_moves_between_frames_and_says_what_the_model_is_doing() {
        let first = text_of(&scene(Language::Rust, 0, 0, 110, 30));
        // Eight milliseconds on, the figure has turned a hair and the light on
        // it has moved: a frame rate that high must not draw the same picture
        // twice.
        let next_frame = scene(Language::Rust, 0, 8, 110, 30);
        assert_ne!(
            scene(Language::Rust, 0, 0, 110, 30),
            next_frame,
            "8ms later is a new frame"
        );
        assert!(
            first.contains("\u{25b8}"),
            "tokens flow into the network:\n{first}"
        );
        assert!(first.contains("\u{25cb}"), "the network is drawn:\n{first}");
        assert!(
            first.contains('O'),
            "Ferris is drawn, eyes and all:\n{first}"
        );
        assert!(first.contains("tokenizing the diff"), "{first}");

        let seen: std::collections::HashSet<String> = (0..(CAPTIONS.len() as u64 + 1))
            .map(|slot| {
                caption(Language::Rust, slot * CAPTION_MS)
                    .trim_end_matches(['.', ' '])
                    .to_owned()
            })
            .collect();
        assert!(seen.contains("arguing with the borrow checker"), "{seen:?}");
        assert!(
            seen.len() > CAPTIONS.len(),
            "every caption gets its turn: {seen:?}"
        );
    }

    #[test]
    fn different_seeds_give_different_backdrops_behind_the_same_mascot() {
        let scenes: Vec<String> = (0..BACKDROPS.len())
            .map(|seed| text_of(&scene(Language::Go, seed, 1_440, 110, 30)))
            .collect();
        for (i, a) in scenes.iter().enumerate() {
            assert!(a.contains('O'), "the gopher is in scene {i}:\n{a}");
            for b in &scenes[i + 1..] {
                assert_ne!(a, b, "seeds pick different backdrops");
            }
        }
    }

    #[test]
    fn a_box_too_small_gets_nothing_rather_than_a_cropped_crab() {
        assert!(scene(Language::Rust, 0, 0, 70, 2).is_empty());
        assert!(scene(Language::Rust, 0, 0, 8, 12).is_empty());
        assert!(scene(Language::Rust, 0, 0, 0, 0).is_empty());
    }

    #[test]
    fn a_narrow_box_keeps_the_mascot_and_drops_the_pipeline() {
        let lines = scene(Language::Go, 1, 7_000, 40, 16);
        assert_eq!(lines.len(), 16);
        assert!(lines.iter().all(|line| line.width() == 40));
        let text = text_of(&lines);
        assert!(text.contains('O'), "the gopher is still there:\n{text}");
        assert!(
            !text.contains("\u{25b8}"),
            "no room for the token stream:\n{text}"
        );
    }
}
