//! Something to look at while the model reads the diff. Prefill on a local
//! model is a silent minute: nothing streams back, so the message pane is an
//! empty box with a timer on it. This fills the box with a small animated
//! scene of what is going on, filling the whole box: the language's mascot,
//! a small three-dimensional figure facing the reader, waving, shifting its
//! stance and blinking, feeds the diff token by token into a neural network
//! that pulses as it thinks, against one of a few backgrounds (code raining
//! down, a starry night, the diff scrolling past, a round of GLTron, a game
//! of Snake, the trench run, a Wolfenstein maze or a Doom base walked in the
//! first person), with a caption reporting what the model is up
//! to. Everything moves on the millisecond clock, so it is as smooth as the
//! terminal can draw. None of it is true, all of it is more fun than a timer.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::solid;
use crate::ui::Token;

mod backdrop;
mod canvas;
mod feed;
mod plan;
mod stream;
use backdrop::*;
use canvas::*;
pub use feed::Feed;
use feed::*;
use plan::*;
use stream::*;

/// The language most of the staged files are written in, judged by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Kotlin,
    Java,
    Go,
    Python,
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
            Self::TypeScript => "feat(ts): ",
            Self::Other => "feat: ",
        }
    }

    /// Whether the mascot is a creature that can move about, or an object
    /// that only turns.
    fn is_alive(self) -> bool {
        !matches!(self, Self::TypeScript)
    }

    /// The mascot as a three-dimensional figure at time `t` seconds, its
    /// eyes on `gaze`, ready to be posed and lit.
    fn figure(self, t: f32, gaze: solid::Gaze) -> Vec<solid::Part> {
        match self {
            Self::Rust => solid::ferris(t, gaze),
            Self::Kotlin => solid::kodee(t, gaze),
            Self::Java => solid::duke(t),
            Self::Go => solid::gopher(t, gaze),
            Self::Python => solid::snake(t, gaze),
            Self::TypeScript => solid::monitor(t, Color::Rgb(90, 160, 240), TS_CODE),
            Self::Other => solid::robot(t, gaze),
        }
    }
}

/// The code being typed on the TypeScript monitor. Short lines: the screen
/// is a dozen cells across at the biggest.
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

/// The figure's picture is this many rows tall at most, and never fewer than
/// the minimum; its width follows from the cell shape.
const FIGURE_MAX_ROWS: usize = 22;
const FIGURE_MIN_ROWS: usize = 8;
/// Radians either side of straight on the figure's stance may turn to: far
/// enough to shift its weight, never far enough to look away.
const TURN_RANGE: f32 = 0.55;
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
/// widest it is allowed to be: the width beyond that goes to the tree. The
/// stream is kept short so the words leaving the network have room, but wide
/// enough that a run of the code going in is readable in it.
const MIN_STREAM_WIDTH: usize = 10;
const MAX_STREAM_WIDTH: usize = 24;
/// Rows of code the stream carries at once, when the band has room for them
/// either side of the wire. An odd number: the wire's row and as many above
/// it as below.
const STREAM_ROWS: usize = 5;
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
/// Cells kept clear to the right of the network: the runway the words of
/// the message take off along as they leave the leaves for the text.
const NETWORK_MARGIN: usize = 12;
/// The colours the sides of a diff are read in.
const DIFF_ADDED: Color = Color::Rgb(70, 160, 90);
const DIFF_REMOVED: Color = Color::Rgb(170, 70, 80);
const DIFF_CONTEXT: Color = Color::Rgb(75, 78, 100);
/// The backdrop's own register for the code itself: the hues the diff pane
/// highlights in, held back to where they can sit behind a scene.
const CODE_KEYWORD: Color = Color::Rgb(198, 162, 88);
const CODE_TYPE: Color = Color::Rgb(118, 190, 200);
const CODE_FUNCTION: Color = Color::Rgb(200, 140, 190);
const CODE_LITERAL: Color = Color::Rgb(206, 196, 122);
const CODE_COMMENT: Color = Color::Rgb(96, 102, 120);
const CODE_PLAIN: Color = Color::Rgb(150, 156, 168);
/// How far a character's colour is pulled towards its side of the diff.
const SIDE_TINT: f32 = 0.45;

type Cell = (char, Style);

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
/// is streaming in above the scene, that would be two messages. `feed` is
/// the diff the model is reading, which is what flows down the stream.
pub fn scene(show: Show<'_>, width: u16, height: u16, writing: bool) -> Vec<Line<'static>> {
    paint(show, width as usize, height as usize, writing)
        .map_or_else(Vec::new, |(canvas, _)| canvas.lines())
}

/// A run of the message that has just come out of the model and is still on
/// its way from the network to its place in the text above the scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flight {
    /// Row and column in the text where the run lands.
    pub row: usize,
    pub col: usize,
    pub text: String,
    /// Milliseconds since the run left the model.
    pub age_ms: u64,
}

/// Milliseconds a run takes to fly from the network to the text.
pub const FLIGHT_MS: u64 = 380;
/// Milliseconds a run glows after landing, cooling into plain text.
const LANDING_GLOW_MS: u64 = 400;
/// After this long a run is plain text and can be forgotten.
pub const FLIGHT_TOTAL_MS: u64 = FLIGHT_MS + LANDING_GLOW_MS;
/// Rows the flight path bulges upward above the straight line, at its most.
const FLIGHT_ARC: f32 = 1.5;

/// A seed for a new wait: what backdrop it gets and how the game in it
/// plays out both follow from the seed, so one that never changed would
/// show the same round every time. Drawn from the clock's nanoseconds and
/// a count of the seeds handed out, so two waits started in the same
/// instant still differ.
pub fn fresh_seed() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static HANDED_OUT: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as usize);
    let n = HANDED_OUT.fetch_add(1, Ordering::Relaxed);
    // Stir the bits so consecutive seeds do not just differ in the low
    // ones, which is all a backdrop pick by remainder would see.
    let mut x = nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x
}

/// What is on show for one generation: the language of the commit, the
/// seed that picked its backdrop, the clock, and the diff going into the
/// network. Everything but the box it is drawn in.
#[derive(Debug, Clone, Copy)]
pub struct Show<'a> {
    pub lang: Language,
    pub seed: usize,
    pub ms: u64,
    pub feed: &'a Feed,
    /// When the first word of the message arrived, on the same clock: the
    /// moment the wait paid off, which some backdrops make an event of.
    pub boom: Option<u64>,
}

/// The message pane while the model is writing: the `text` streamed so far
/// on the top rows and the scene underneath, with each newly arrived run of
/// text flying from the network out to where it belongs, glowing as it
/// lands. Text that is still in flight is not yet at its destination, so
/// its cells there are blank until it gets there. The whole box is filled,
/// `height` rows of `width` cells; the text takes as many rows as it has,
/// the scene what is left, or nothing when that is too little.
pub fn stage(
    show: Show<'_>,
    width: u16,
    height: u16,
    text: &[String],
    flights: &[Flight],
) -> Vec<Line<'static>> {
    let (width, height) = (width as usize, height as usize);
    let mut canvas = Canvas::new(width, height);
    let text_rows = text.len().min(height);
    let plain = Style::default();
    for (row, line) in text.iter().enumerate().take(text_rows) {
        for (col, c) in line.chars().enumerate().take(width) {
            let flying = flights.iter().any(|f| {
                f.age_ms < FLIGHT_MS
                    && f.row == row
                    && (f.col..f.col + f.text.chars().count()).contains(&col)
            });
            if !flying {
                canvas.put(col, row, c, plain);
            }
        }
    }
    let scene_y = text_rows + usize::from(text_rows > 0);
    let scene_rows = height.saturating_sub(scene_y);
    let exit = paint(show, width, scene_rows, text_rows == 0).map(|(scene, exit)| {
        canvas.blit(&scene, scene_y);
        (exit.0 as f32, (exit.1 + scene_y) as f32)
    });
    for flight in flights {
        draw_flight(&mut canvas, flight, exit, text_rows, show.ms);
    }
    canvas.lines()
}

/// Turns of the colour wheel per second a run in flight cycles through.
const RAINBOW_RATE: f32 = 0.6;
/// How far round the wheel each character is from the one before it.
const RAINBOW_SPREAD: f32 = 0.05;

/// One run on its way, or cooling where it has just landed. In flight it is
/// the model's: bold and cycling through the rainbow, so there is no
/// mistaking generated text for the message as it stands. Landed, it cools
/// from its last colour into plain text. With no scene to come from the run
/// simply appears where it belongs and cools there.
fn draw_flight(
    canvas: &mut Canvas,
    flight: &Flight,
    exit: Option<(f32, f32)>,
    text_rows: usize,
    ms: u64,
) {
    if flight.age_ms >= FLIGHT_TOTAL_MS || flight.row >= text_rows {
        return;
    }
    let plain = Color::Rgb(220, 220, 240);
    let (tx, ty) = (flight.col as f32, flight.row as f32);
    let flying = flight.age_ms < FLIGHT_MS;
    let (x, y) = match exit.filter(|_| flying) {
        Some((ex, ey)) => {
            let k = flight.age_ms as f32 / FLIGHT_MS as f32;
            // Ease in and out, and lift the path into an arc so the run
            // rises out of the network before it settles into the line.
            let s = k * k * (3.0 - 2.0 * k);
            let arc = (k * std::f32::consts::PI).sin() * FLIGHT_ARC;
            (ex + (tx - ex) * s, ey + (ty - ey) * s - arc)
        }
        None => (tx, ty),
    };
    // The run takes off from the far right of the network; keep all of it
    // in the box on its way rather than clip its tail off.
    let len = flight.text.chars().count();
    let x = (x.round().max(0.0) as usize).min(canvas.width().saturating_sub(len));
    let y = y.round().max(0.0) as usize;
    // The wheel keeps turning after landing, from where it was when the run
    // touched down, so the colour cools without a jump.
    let landed_ms = ms.saturating_sub(flight.age_ms.saturating_sub(FLIGHT_MS));
    let turn = landed_ms as f32 / 1000.0 * RAINBOW_RATE;
    let cooled = if flying {
        0.0
    } else {
        (flight.age_ms - FLIGHT_MS) as f32 / LANDING_GLOW_MS as f32
    };
    for (i, c) in flight.text.chars().enumerate() {
        if c.is_whitespace() {
            continue;
        }
        let color = mix(hue(turn + i as f32 * RAINBOW_SPREAD), plain, cooled);
        let mut style = Style::default().fg(color);
        if flying {
            style = style.add_modifier(Modifier::BOLD);
        }
        canvas.put(x + i, y, c, style);
    }
}

/// The scene painted onto a canvas of its own, and the point the message
/// leaves it from: the leaves of the network, or the top of the mascot's
/// head when the box is too narrow for a network.
fn paint(
    show: Show<'_>,
    width: usize,
    height: usize,
    writing: bool,
) -> Option<(Canvas, (usize, usize))> {
    let Show {
        lang,
        seed,
        ms,
        feed,
        ..
    } = show;
    let plan = Plan::fit(lang, seed, width, height)?;
    let mut canvas = Canvas::new(plan.width, plan.height);
    let t = ms as f32 / 1000.0;

    Backdrop::pick(seed).draw(&mut canvas, show, plan.width, plan.ground);
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

    let mut exit = (mascot_x + plan.mascot_w / 2, band_y);
    if let Some(pipe) = &plan.pipeline {
        let network = Network::new(&pipe.tree, pipe.tree_y, t, feed);
        let root_row = pipe.tree_y + pipe.tree.nodes[0].row;
        exit = (pipe.tree.x + pipe.tree.width() + 1, root_row);
        let lanes = lanes(feed, pipe.stream_y, pipe.stream_h, root_row);
        draw_stream(
            &mut canvas,
            pipe.stream_x,
            pipe.stream_w,
            &lanes,
            root_row,
            t,
            feed,
        );
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
    Some((canvas, exit))
}

/// The language's figure, posed and lit for the moment `t`, drawn into a
/// `w` by `h` window with its bottom on the ground. It faces the reader,
/// its eyes following them as it shifts its stance, and moves on its own
/// clock, so at any frame rate it is where it should be.
fn draw_figure(
    canvas: &mut Canvas,
    lang: Language,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    t: f32,
) {
    let pose = solid::pose(t, TURN_RANGE, lang.is_alive());
    let grid = solid::render_posed(&lang.figure(t, solid::Gaze::at(t, pose.yaw)), pose, w, h);
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
mod tests;
