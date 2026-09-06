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

    /// How the commit message this language's commits usually open: what the
    /// network can be seen writing before the model has said anything.
    fn opening(self) -> &'static str {
        match self {
            Self::Rust => "feat(rust): ",
            Self::Kotlin => "feat(kotlin): ",
            Self::Java => "feat(java): ",
            Self::Go => "feat(go): ",
            Self::Python => "feat(python): ",
            Self::JavaScript => "feat(js): ",
            Self::TypeScript => "feat(ts): ",
            Self::Other => "feat: ",
        }
    }

    /// Whether the mascot is a creature that can move about, or an object
    /// that only turns.
    fn is_alive(self) -> bool {
        !matches!(self, Self::JavaScript | Self::TypeScript)
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
            Self::JavaScript => solid::monitor(t, Color::Rgb(247, 223, 30), JS_CODE),
            Self::TypeScript => solid::monitor(t, Color::Rgb(90, 160, 240), TS_CODE),
            Self::Other => solid::robot(t),
        }
    }
}

/// The code being typed on the JavaScript and TypeScript monitors. Short
/// lines: the screen is a dozen cells across at the biggest.
const JS_CODE: &[&str] = &[
    "const msg =",
    "  await llm(",
    "    diff);",
    "if (!msg) {",
    "  retry();",
    "}",
    "log(msg);",
];
const TS_CODE: &[&str] = &[
    "type Msg =",
    "  string;",
    "const m: Msg",
    "  = await",
    "  gen(diff);",
    "if (!m) fail",
    "export {m};",
];

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
/// The glyphs a token can be drawn as, from dense to faint.
const TOKEN_GLYPHS: &[char] = &[
    '\u{25aa}', '\u{2022}', '\u{25b4}', '\u{25be}', '\u{25ab}', '\u{25e6}', '\u{2218}', '\u{b7}',
];
/// Every token slot in this many is empty, so the stream has gaps.
const TOKEN_GAP: u64 = 3;
/// Cells the stream moves per second.
const STREAM_SPEED: f32 = 12.0;
/// The narrowest the stream may be before the pipeline is dropped, and the
/// widest it is allowed to be: the width beyond that goes to the tree.
const MIN_STREAM_WIDTH: usize = 10;
const MAX_STREAM_WIDTH: usize = 22;
/// The network is a tree lying on its side, root at the left where the
/// stream comes in, leaves at the right. This many levels at most; a low
/// band gets fewer.
const TREE_LEVELS: usize = 7;
const TREE_MIN_LEVELS: usize = 3;
/// Rows between leaves when there is room; in a short box the leaves sit on
/// consecutive rows instead.
const LEAF_STEP: usize = 2;
/// Cells between levels, at the least and at the most: the tree spreads out
/// to fill the width it is given.
const LEVEL_STEP: usize = 6;
const MAX_LEVEL_STEP: usize = 10;
/// Cells kept clear to the right of the network so the leaves do not sit on
/// the border and look cut off.
const NETWORK_MARGIN: usize = 2;
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

/// Where the parts go in a box of one size. The backdrop fills the top down
/// to the ground line; under it is a black band with the mascot at the left,
/// a stream of tokens running from it across the middle into the root of the
/// network at the right, and the message the network is writing under the
/// network; the caption sits at the very bottom.
struct Plan {
    width: usize,
    height: usize,
    mascot_w: usize,
    mascot_h: usize,
    /// The row the backdrop stops at; the band starts on the row below.
    ground: usize,
    /// The stream and network, if there is room for them beside the mascot.
    pipeline: Option<Pipeline>,
    caption: bool,
}

#[derive(Debug, Clone)]
struct Pipeline {
    /// Where the stream starts and how wide it is.
    stream_x: usize,
    stream_w: usize,
    tree: Tree,
    /// Top row of the tree.
    tree_y: usize,
    /// The row the network writes its output on, when there is one to spare.
    output_row: Option<usize>,
}

/// One node of the network.
#[derive(Debug, Clone)]
struct Node {
    level: usize,
    /// Row relative to the top of the tree.
    row: usize,
    children: Vec<usize>,
}

/// The network's shape: an irregular tree, grown from a seed. Each node has
/// one to three branches, some stop short, and no two waits grow the same
/// one. Leaves take the rows in order; a node sits in the middle of the rows
/// its leaves take.
#[derive(Debug, Clone)]
struct Tree {
    nodes: Vec<Node>,
    levels: usize,
    /// Column of the root, and cells between levels.
    x: usize,
    level_step: usize,
    height: usize,
}

impl Tree {
    /// Grow a tree with at most `levels` levels whose leaves, `leaf_step`
    /// rows apart, fit in `max_height` rows. `None` if not even a root and
    /// two leaves fit.
    fn grow(seed: usize, levels: usize, leaf_step: usize, max_height: usize) -> Option<Self> {
        let max_leaves = max_height.checked_sub(1)? / leaf_step + 1;
        if max_leaves < 2 || levels < 2 {
            return None;
        }
        let mut nodes = vec![Node {
            level: 0,
            row: 0,
            children: Vec::new(),
        }];
        // Leaves still allowed beyond the one every node already is.
        let mut spare = max_leaves - 1;
        Self::branch(&mut nodes, 0, seed, levels, &mut spare);
        let mut next_row = 0;
        Self::place(&mut nodes, 0, leaf_step, &mut next_row);
        let height = nodes.iter().map(|n| n.row).max().unwrap_or(0) + 1;
        Some(Self {
            nodes,
            levels,
            x: 0,
            level_step: LEVEL_STEP,
            height,
        })
    }

    /// Give node `id` its children and grow them in turn, within the leaf
    /// budget.
    fn branch(nodes: &mut Vec<Node>, id: usize, seed: usize, levels: usize, spare: &mut usize) {
        let level = nodes[id].level;
        if level + 1 >= levels || *spare == 0 {
            return;
        }
        let h = hash(seed.wrapping_mul(7919).wrapping_add(id), 5);
        let mut want = match h % 10 {
            0..=1 => 1,
            2..=7 => 2,
            _ => 3,
        };
        // Some branches stop short of the last level; the root never does,
        // and always forks.
        if level == 0 {
            want = want.max(2);
        } else if level >= 2 && (h >> 8) % 5 == 0 {
            return;
        }
        let want = want.min(*spare + 1);
        *spare -= want - 1;
        let first = nodes.len();
        for _ in 0..want {
            nodes.push(Node {
                level: level + 1,
                row: 0,
                children: Vec::new(),
            });
        }
        nodes[id].children = (first..first + want).collect();
        for child in first..first + want {
            Self::branch(nodes, child, seed, levels, spare);
        }
    }

    /// Rows: leaves in order, each node in the middle of its leaves.
    fn place(nodes: &mut [Node], id: usize, leaf_step: usize, next_row: &mut usize) {
        let children = nodes[id].children.clone();
        if children.is_empty() {
            nodes[id].row = *next_row;
            *next_row += leaf_step;
            return;
        }
        for &c in &children {
            Self::place(nodes, c, leaf_step, next_row);
        }
        let first = nodes[children[0]].row;
        let last = nodes[*children.last().unwrap_or(&children[0])].row;
        nodes[id].row = (first + last) / 2;
    }

    fn width(&self) -> usize {
        (self.levels - 1) * self.level_step + 1
    }

    fn col(&self, level: usize) -> usize {
        self.x + level * self.level_step
    }
}

/// Cells between the end of the stream and the root.
const WIRE_GAP: usize = 3;
/// Rows of backdrop the band leaves above itself when the box allows.
const MIN_SKY: usize = 5;

impl Plan {
    /// The figure's box is a little over twice as wide as tall in cells, as
    /// cells are twice as tall as they are wide: square on screen.
    fn mascot_width(rows: usize) -> usize {
        rows * 2 + 4
    }

    fn fit(lang: Language, seed: usize, width: usize, height: usize) -> Option<Self> {
        let caption_w = CAPTIONS
            .iter()
            .chain(std::iter::once(&lang.quip()))
            .map(|text| text.chars().count() + 3)
            .max()
            .unwrap_or(0);
        let caption = caption_w <= width;
        // Ground line above the band, blank and caption below it.
        let below = 1 + if caption { 2 } else { 0 };
        let usable = height.checked_sub(below)?;
        if usable < FIGURE_MIN_ROWS {
            return None;
        }
        // The band is as tall as the figure, which takes what it can while
        // leaving some sky, and shrinks to the width if it must.
        let mut mascot_h = usable
            .saturating_sub(MIN_SKY)
            .clamp(FIGURE_MIN_ROWS, FIGURE_MAX_ROWS.min(usable));
        while Self::mascot_width(mascot_h) > width {
            mascot_h -= 1;
            if mascot_h < FIGURE_MIN_ROWS {
                return None;
            }
        }
        let mascot_w = Self::mascot_width(mascot_h);
        let ground = height - below - mascot_h;
        let band_y = ground + 1;
        // The biggest tree the band holds, spread out if it can be; ideally
        // with a row left under it for the output. The stream takes only
        // what it needs of the width; the tree gets the rest, level by level.
        let stream_x = mascot_w + 1;
        let for_tree = width
            .checked_sub(stream_x + MIN_STREAM_WIDTH + WIRE_GAP + NETWORK_MARGIN)?
            .checked_sub(1)?;
        let pipeline = [2, 0].into_iter().find_map(|spare| {
            let band = mascot_h.checked_sub(spare)?;
            // Levels the width allows, and no more than the leaves can
            // fill: a tall thin tree with one leaf a level is a stick.
            let max_leaves = (band - 1) / LEAF_STEP + 1;
            let most_levels = (for_tree / LEVEL_STEP + 1)
                .min(TREE_LEVELS)
                .min((max_leaves / 2).max(TREE_MIN_LEVELS));
            (TREE_MIN_LEVELS..=most_levels)
                .rev()
                .flat_map(|levels| [(levels, LEAF_STEP), (levels, 1)])
                .find_map(|(levels, leaf_step)| {
                    let mut tree = Tree::grow(seed, levels, leaf_step, band)?;
                    // A tree that could not fork enough to use its levels
                    // is no better than a smaller one; let that be tried.
                    if tree.nodes.iter().map(|n| n.level).max().unwrap_or(0) + 1 < levels {
                        return None;
                    }
                    // Spread the levels to fill what the stream does not
                    // need, then give the stream the rest.
                    let widest = width
                        .checked_sub(stream_x + MAX_STREAM_WIDTH + WIRE_GAP + NETWORK_MARGIN + 1)?;
                    tree.level_step = (widest / (levels - 1)).clamp(LEVEL_STEP, MAX_LEVEL_STEP);
                    tree.x = width.checked_sub(tree.width() + NETWORK_MARGIN)?;
                    let stream_w = tree.x.checked_sub(stream_x + WIRE_GAP)?;
                    if stream_w < MIN_STREAM_WIDTH {
                        return None;
                    }
                    // The stream leaves the figure a little below its
                    // middle, where its body is, so the root sits there and
                    // the tree hangs off it as far as the band allows.
                    let root_target = band_y + mascot_h * 3 / 5;
                    let lowest = band_y + mascot_h - spare - tree.height;
                    let tree_y = root_target
                        .saturating_sub(tree.nodes[0].row)
                        .clamp(band_y, lowest);
                    Some(Pipeline {
                        stream_x,
                        stream_w,
                        tree,
                        tree_y,
                        output_row: (spare > 0).then_some(band_y + mascot_h - 1),
                    })
                })
        });
        Some(Self {
            width,
            height,
            mascot_w,
            mascot_h,
            ground,
            pipeline,
            caption,
        })
    }
}

/// The scene at clock `ms`, filling a box `width` by `height` cells. `seed`
/// picks the backdrop, and stays the same for one wait so the picture does
/// not change under the reader. The sky is the backdrop; under the ground
/// line, on black, the mascot stands at the left and feeds a stream of
/// tokens into the root of the network at the right. Each token that arrives
/// fires a pulse down one path of the tree, and as pulses reach the leaves
/// the network starts writing the message underneath. Every row is the full
/// width so centring the block does not shift anything as the parts animate.
/// Empty when the box cannot hold the figure: a crab squeezed into two rows
/// is worse than nothing.
/// `writing` shows the network starting on the message; once the real one
/// is streaming in above the scene, that would be two messages.
pub fn scene(
    lang: Language,
    seed: usize,
    ms: u64,
    width: u16,
    height: u16,
    writing: bool,
) -> Vec<Line<'static>> {
    let Some(plan) = Plan::fit(lang, seed, width as usize, height as usize) else {
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

    let band_y = plan.ground + 1;
    let mascot_x = if plan.pipeline.is_some() {
        0
    } else {
        (plan.width - plan.mascot_w) / 2
    };
    draw_figure(
        &mut canvas,
        lang,
        mascot_x,
        band_y,
        plan.mascot_w,
        plan.mascot_h,
        t,
    );

    if let Some(pipe) = &plan.pipeline {
        let network = Network::new(&pipe.tree, pipe.tree_y, t);
        let root_row = pipe.tree_y + pipe.tree.nodes[0].row;
        draw_stream(&mut canvas, pipe.stream_x, root_row, pipe.stream_w, t);
        let style = Style::default().fg(mix(
            Color::Rgb(80, 80, 110),
            Color::Rgb(255, 250, 170),
            network.root_heat(),
        ));
        for x in pipe.stream_x + pipe.stream_w..pipe.tree.x {
            canvas.put(x, root_row, '\u{2500}', style);
        }
        network.draw(&mut canvas);
        if let Some(row) = pipe.output_row.filter(|_| writing) {
            let room = plan.width.saturating_sub(pipe.tree.x + 1);
            draw_output(
                &mut canvas,
                lang,
                pipe.tree.x,
                row,
                room,
                network.landed,
                network.landing,
                t,
            );
        }
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
        let moon_x = width.saturating_sub(moon[0].len() + (TREE_LEVELS - 1) * MAX_LEVEL_STEP + 8);
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
/// Draw the mascot into its box, moving as `solid::pose` says.
fn draw_figure(
    canvas: &mut Canvas,
    lang: Language,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    t: f32,
) {
    let grid = solid::render_posed(
        &lang.figure(t),
        solid::pose(t, TURN_RATE, lang.is_alive()),
        w,
        h,
    );
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

/// The token in slot `slot` of stream `row` at time `t`, or none for a gap.
/// Each slot is hashed for its glyph and its own hue, so the stream is a
/// jumble rather than a pattern, and every token glitters at its own pace.
/// The hue also drifts with time and along the stream, so the whole thing
/// slowly cycles through the rainbow.
fn token(slot: usize, row: usize, along: f32, t: f32) -> Option<(char, Style)> {
    let seed = hash(slot, row + 11);
    if seed % TOKEN_GAP == 0 {
        return None;
    }
    let glyph = TOKEN_GLYPHS[((seed >> 8) % TOKEN_GLYPHS.len() as u64) as usize];
    let own_hue = ((seed >> 16) % 1_000) as f32 / 1_000.0;
    let color = hue(own_hue * 0.35 + along * 0.5 + t * 0.12);
    // Each token twinkles on its own period; the brightest go white-hot.
    let rate = 3.0 + ((seed >> 24) % 50) as f32 / 10.0;
    let phase = ((seed >> 32) % 628) as f32 / 100.0;
    let sparkle = 0.5 + 0.5 * (t * rate + phase).sin();
    let dense = glyph == TOKEN_GLYPHS[0] || glyph == TOKEN_GLYPHS[1];
    let color = if sparkle > 0.85 {
        mix(color, Color::Rgb(255, 255, 255), (sparkle - 0.85) * 5.0)
    } else {
        dim(color, 0.55 + 0.45 * sparkle)
    };
    let mut style = Style::default().fg(color);
    if dense || sparkle > 0.85 {
        style = style.add_modifier(Modifier::BOLD);
    }
    Some((glyph, style))
}

/// When slot `slot` reaches the arrowhead at the end of the stream: the slot
/// there at time `t` is `STREAM_ORIGIN + t * STREAM_SPEED + 1`.
fn arrival(slot: usize) -> f32 {
    (slot as f32 - STREAM_ORIGIN as f32 - 1.0) / STREAM_SPEED
}

/// Whether the token in `slot` fires a pulse when it reaches the network:
/// only the dense ones do, so the paths through the tree stay distinct.
fn token_fires(slot: usize) -> bool {
    let seed = hash(slot, 11);
    seed % TOKEN_GAP != 0 && (seed >> 8) % (TOKEN_GLYPHS.len() as u64) < 2
}

/// The stream of tokens flowing right into the root of the network, one
/// line of them ending in an arrowhead at the wire.
fn draw_stream(canvas: &mut Canvas, x: usize, y: usize, width: usize, t: f32) {
    let travelled = (t * STREAM_SPEED) as usize;
    for col in 0..width.saturating_sub(1) {
        let slot = STREAM_ORIGIN + travelled + width - col;
        let along = col as f32 / width.max(1) as f32;
        if let Some((c, style)) = token(slot, 0, along, t) {
            canvas.put(x + col, y, c, style);
        }
    }
    canvas.put(
        x + width - 1,
        y,
        '\u{25b8}',
        Style::default()
            .fg(hue(t * 0.12 + 0.5))
            .add_modifier(Modifier::BOLD),
    );
}

/// Slots are counted from here so the arithmetic never goes below zero.
const STREAM_ORIGIN: usize = 1_000_000;
/// Seconds a pulse takes to cross one level of the tree.
const PULSE_LEVEL_TIME: f32 = 0.28;

/// The network at one moment: the tree with the pulses running through it.
/// Every dense token that has reached the end of the stream lately is a
/// pulse somewhere in the tree, following a path of its own from the root to
/// one leaf, so different branches light up as the tokens go in.
struct Network<'a> {
    tree: &'a Tree,
    y: usize,
    /// How hot each node is, 0..=1.
    heat: Vec<f32>,
    /// How hot the branch into each node is.
    branches: Vec<f32>,
    /// Tokens that have come all the way through so far.
    landed: usize,
    /// How recently the last one landed, 1 at the moment it does, fading.
    landing: f32,
}

impl<'a> Network<'a> {
    fn new(tree: &'a Tree, y: usize, t: f32) -> Self {
        let mut heat = vec![0.0f32; tree.nodes.len()];
        let mut branches = vec![0.0f32; tree.nodes.len()];
        let travel = tree.levels as f32 * PULSE_LEVEL_TIME;
        let travelled = (t * STREAM_SPEED) as usize;
        // The slot at the arrowhead right now, and the ones just before it
        // that may still be crossing the tree.
        let newest = STREAM_ORIGIN + travelled + 1;
        let in_flight = (travel * STREAM_SPEED) as usize + 2;
        let mut landed = 0;
        let mut landing = 0.0f32;
        for slot in newest.saturating_sub(in_flight)..=newest {
            if !token_fires(slot) {
                continue;
            }
            let age = t - arrival(slot);
            if age < 0.0 {
                continue;
            }
            if age > travel {
                landed += 1;
                landing = landing.max(1.0 - (age - travel) / 0.5);
                continue;
            }
            let position = age / PULSE_LEVEL_TIME;
            let mut node = 0;
            loop {
                let level = tree.nodes[node].level as f32;
                let glow = (1.0 - (position - level).abs()).clamp(0.0, 1.0);
                heat[node] = heat[node].max(glow);
                let children = &tree.nodes[node].children;
                if children.is_empty() {
                    break;
                }
                // Which branch, decided by the token itself.
                let pick = (hash(slot, tree.nodes[node].level + 40) >> 8) as usize % children.len();
                node = children[pick];
                let crossing = (1.0 - (position - level - 0.5).abs() / 0.7).clamp(0.0, 1.0);
                branches[node] = branches[node].max(crossing);
            }
        }
        // Everything that landed before the window counts too.
        landed += newest
            .saturating_sub(in_flight)
            .saturating_sub(STREAM_ORIGIN)
            / 6;
        Self {
            tree,
            y,
            heat,
            branches,
            landed,
            landing,
        }
    }

    fn root_heat(&self) -> f32 {
        self.heat[0]
    }

    fn draw(&self, canvas: &mut Canvas) {
        let cold = Color::Rgb(100, 100, 135);
        let ember = Color::Rgb(255, 120, 70);
        let hot = Color::Rgb(255, 250, 170);
        let branch_cold = Color::Rgb(70, 70, 100);
        let tree = self.tree;
        let style = |k: f32| {
            let mut st = Style::default().fg(mix(branch_cold, hot, k));
            if k > 0.3 {
                st = st.add_modifier(Modifier::BOLD);
            }
            st
        };
        // Branches first, nodes over them. Each parent runs a stub right to
        // an elbow column, a bar there spans its children's rows, and each
        // child gets a stub from the bar; the bar's cells are joined with
        // whichever corner or tee the lines through them call for.
        for node in &tree.nodes {
            if node.children.is_empty() {
                continue;
            }
            let px = tree.col(node.level);
            let elbow = px + tree.level_step / 2;
            let cx = tree.col(node.level + 1);
            let py = self.y + node.row;
            let rows: Vec<(usize, f32)> = node
                .children
                .iter()
                .map(|&c| (self.y + tree.nodes[c].row, self.branches[c]))
                .collect();
            let stub = rows.iter().map(|r| r.1).fold(0.0, f32::max);
            for x in px + 1..elbow {
                canvas.put(x, py, '\u{2500}', style(stub));
            }
            let lo = rows.iter().map(|r| r.0).min().unwrap_or(py).min(py);
            let hi = rows.iter().map(|r| r.0).max().unwrap_or(py).max(py);
            for yy in lo..=hi {
                let up = yy > lo;
                let down = yy < hi;
                let left = yy == py;
                let child = rows.iter().find(|r| r.0 == yy);
                let right = child.is_some();
                let glyph = match (up, down, left, right) {
                    (false, true, false, true) => '\u{256d}',
                    (true, false, false, true) => '\u{2570}',
                    (true, true, false, true) => '\u{251c}',
                    (true, true, true, false) => '\u{2524}',
                    (true, true, true, true) => '\u{253c}',
                    (false, true, true, false) => '\u{256e}',
                    (true, false, true, false) => '\u{256f}',
                    (false, true, true, true) => '\u{252c}',
                    (true, false, true, true) => '\u{2534}',
                    (true, true, false, false) => '\u{2502}',
                    _ => '\u{2500}',
                };
                // A cell of the bar is as hot as the hottest branch running
                // through it: one whose child row is on the far side of it.
                let k = rows
                    .iter()
                    .filter(|r| (r.0.min(py)..=r.0.max(py)).contains(&yy))
                    .map(|r| r.1)
                    .fold(0.0, f32::max);
                canvas.put(elbow, yy, glyph, style(k));
                if let Some(&(_, kh)) = child {
                    for x in elbow + 1..cx {
                        canvas.put(x, yy, '\u{2500}', style(kh));
                    }
                }
            }
        }
        for (id, node) in tree.nodes.iter().enumerate() {
            let heat = self.heat[id];
            let leaf = node.children.is_empty();
            let (glyph, color) = if heat > 0.75 {
                ('\u{25c9}', mix(ember, hot, (heat - 0.75) * 4.0))
            } else if heat > 0.3 {
                ('\u{25cf}', mix(cold, ember, (heat - 0.3) / 0.45))
            } else if leaf {
                ('\u{25c7}', cold)
            } else {
                ('\u{25cb}', cold)
            };
            let mut style = Style::default().fg(color);
            if heat > 0.75 {
                style = style.add_modifier(Modifier::BOLD);
            }
            canvas.put(tree.col(node.level), self.y + node.row, glyph, style);
        }
    }
}

/// Tokens through the network per character of output.
const TOKENS_PER_CHAR: usize = 2;

/// The start of the commit message the network is writing, one character
/// per couple of tokens through it, with a cursor blinking at the end. The
/// newest character flares as the pulse that wrote it lands. Only the
/// opening is guessable before the model has spoken; the real message takes
/// this line's place the moment it does.
#[allow(clippy::too_many_arguments)]
fn draw_output(
    canvas: &mut Canvas,
    lang: Language,
    x: usize,
    y: usize,
    room: usize,
    landed: usize,
    landing: f32,
    t: f32,
) {
    let text = lang.opening();
    let typed = (landed / TOKENS_PER_CHAR).min(text.chars().count());
    let shown: String = text.chars().take(typed).collect();
    let plain = Color::Rgb(220, 220, 240);
    let style = Style::default().fg(plain).add_modifier(Modifier::BOLD);
    for (i, c) in shown.chars().enumerate().take(room.saturating_sub(1)) {
        let st = if i + 1 == typed {
            Style::default()
                .fg(mix(plain, Color::Rgb(255, 200, 120), landing))
                .add_modifier(Modifier::BOLD)
        } else {
            style
        };
        canvas.put(x + i, y, c, st);
    }
    if ((t * 3.0).sin() > 0.0 || landing > 0.5) && typed < room {
        canvas.put(x + typed, y, '\u{258c}', style);
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
                        let lines = scene(lang, seed, ms, w, h, true);
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
        let first = text_of(&scene(Language::Rust, 0, 0, 110, 30, true));
        // Eight milliseconds on, the figure has turned a hair and the light on
        // it has moved: a frame rate that high must not draw the same picture
        // twice.
        let next_frame = scene(Language::Rust, 0, 8, 110, 30, true);
        assert_ne!(
            scene(Language::Rust, 0, 0, 110, 30, true),
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
            .map(|seed| text_of(&scene(Language::Go, seed, 1_440, 110, 30, true)))
            .collect();
        for (i, a) in scenes.iter().enumerate() {
            // The gopher may have its back turned at this instant, so look
            // for its shaded body rather than its eyes: no backdrop uses the
            // dense shading glyphs.
            assert!(
                a.contains(['#', '%', '@']),
                "the gopher is in scene {i}:\n{a}"
            );
            for b in &scenes[i + 1..] {
                assert_ne!(a, b, "seeds pick different backdrops");
            }
        }
    }

    #[test]
    fn a_box_too_small_gets_nothing_rather_than_a_cropped_crab() {
        assert!(scene(Language::Rust, 0, 0, 70, 2, true).is_empty());
        assert!(scene(Language::Rust, 0, 0, 8, 12, true).is_empty());
        assert!(scene(Language::Rust, 0, 0, 0, 0, true).is_empty());
    }

    #[test]
    fn a_narrow_box_keeps_the_mascot_and_drops_the_pipeline() {
        let lines = scene(Language::Go, 1, 7_000, 40, 16, true);
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
