//! Something to look at while the model reads the diff. Prefill on a local
//! model is a silent minute: nothing streams back, so the message pane is an
//! empty box with a timer on it. This fills the box with a small animated
//! scene of what is going on, filling the whole box: the language's mascot,
//! a small three-dimensional figure facing the reader, waving, shifting its
//! stance and blinking, feeds the diff token by token into a neural network
//! that pulses as it thinks, against one of a few backgrounds (code raining
//! down, a starry night, the diff scrolling past, an arcade round, the trench
//! run), with a caption reporting what the model is up to. Everything moves
//! on the millisecond clock, so it is as smooth as the terminal can draw.
//! None of it is true, all of it is more fun than a timer.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::{arena, solid, trench};
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

/// The backgrounds a wait can be set against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backdrop {
    /// Code glyphs raining down.
    Rain,
    /// Stars twinkling under a moon, with the odd shooting star.
    Night,
    /// The diff itself scrolling up past the reader.
    Diff,
    /// Light cycles and a snake playing out a round on the grid.
    Arena,
    /// The trench run: the Death Star, which goes up when the first word
    /// of the message flies in.
    Trench,
}

const BACKDROPS: [Backdrop; 5] = [
    Backdrop::Rain,
    Backdrop::Night,
    Backdrop::Diff,
    Backdrop::Arena,
    Backdrop::Trench,
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
/// stream is kept short so the words leaving the network have room.
const MIN_STREAM_WIDTH: usize = 8;
const MAX_STREAM_WIDTH: usize = 16;
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
/// The moon is not a drawing but a lit sphere worked out cell by cell:
/// these glyphs, faint to dense, stand for the light coming off it.
const MOON_RAMP: &[char] = &['.', ':', '-', '=', '+', '*', '#', '%', '@'];
/// Rows the disc spans, at most and at least. Under the minimum a sphere
/// has too few cells to read as round, so the sky is left moonless.
const MOON_MAX_ROWS: usize = 18;
const MOON_MIN_ROWS: usize = 8;
/// How much light the bright highlands throw back, and how much of that is
/// left at the limb: enough dimming to round the ball off, not so much that
/// its edge blurs into the sky.
const MOON_HIGHLAND: f32 = 0.95;
const MOON_LIMB: f32 = 0.78;
/// The near side's dark plains, as middle, radius and depth on the unit
/// disc with x to the right and y up. The moon shows us one face, so these
/// are placed where they are seen rather than turned into view.
const MOON_MARIA: &[(f32, f32, f32, f32)] = &[
    (-0.55, 0.10, 0.34, 0.46),  // Oceanus Procellarum
    (-0.28, 0.42, 0.26, 0.50),  // Mare Imbrium
    (0.10, 0.36, 0.18, 0.48),   // Mare Serenitatis
    (0.28, 0.17, 0.20, 0.48),   // Mare Tranquillitatis
    (0.61, 0.29, 0.10, 0.44),   // Mare Crisium
    (0.44, -0.01, 0.14, 0.42),  // Mare Fecunditatis
    (0.32, -0.17, 0.10, 0.38),  // Mare Nectaris
    (-0.02, 0.17, 0.09, 0.34),  // Mare Vaporum
    (-0.16, -0.34, 0.15, 0.36), // Mare Nubium
    (-0.37, -0.31, 0.11, 0.38), // Mare Humorum
];
/// The ray craters, laid out the same way: Tycho in the south with its
/// splash, and Copernicus above it.
const MOON_CRATERS: &[(f32, f32, f32, f32)] = &[
    (-0.11, -0.61, 0.11, 0.08), // Tycho
    (-0.27, 0.16, 0.07, 0.10),  // Copernicus
];
/// Where the moon would rather hang: this far in from the right edge, so it
/// is over the sky left of where the network sits below and the picture is
/// not all weight on one side. This is a preference, not a fit: a sky with
/// no room for it puts the moon as far right as it goes and no further.
const MOON_MARGIN: usize = (TREE_LEVELS - 1) * MAX_LEVEL_STEP + 8;
/// The fit, on the other hand: this much sky beside the moon or none at
/// all, since a moon with the sky crowded around it is worse than a clear
/// night.
const MOON_SKY: usize = 32;
/// The row the moon hangs from.
const MOON_Y: usize = 1;
/// Rows per second the diff scrolls.
const DIFF_SPEED: f32 = 4.0;
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
        boom,
    } = show;
    let plan = Plan::fit(lang, seed, width, height)?;
    let mut canvas = Canvas::new(plan.width, plan.height);
    let t = ms as f32 / 1000.0;

    match BACKDROPS[seed % BACKDROPS.len()] {
        Backdrop::Rain => draw_rain(&mut canvas, plan.width, plan.ground, t),
        Backdrop::Night => draw_night(&mut canvas, plan.width, plan.ground, ms),
        Backdrop::Diff => draw_diff(&mut canvas, feed, plan.width, plan.ground, t),
        Backdrop::Arena => {
            arena::frame(seed, plan.width, plan.ground, ms, &mut |x, y, c, style| {
                canvas.put(x, y, c, style)
            })
        }
        Backdrop::Trench => trench::frame(
            seed,
            plan.width,
            plan.ground,
            ms,
            boom,
            &mut |x, y, c, style| canvas.put(x, y, c, style),
        ),
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

    let mut exit = (mascot_x + plan.mascot_w / 2, band_y);
    if let Some(pipe) = &plan.pipeline {
        let network = Network::new(&pipe.tree, pipe.tree_y, t, feed);
        let root_row = pipe.tree_y + pipe.tree.nodes[0].row;
        exit = (pipe.tree.x + pipe.tree.width() + 1, root_row);
        draw_stream(&mut canvas, pipe.stream_x, root_row, pipe.stream_w, t, feed);
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
mod tests {
    use super::*;

    /// A show with no diff behind it: the stream falls back to abstract
    /// tokens, which is what most of these tests are looking at.
    fn show(lang: Language, seed: usize, ms: u64) -> Show<'static> {
        static EMPTY: std::sync::OnceLock<Feed> = std::sync::OnceLock::new();
        Show {
            lang,
            seed,
            ms,
            feed: EMPTY.get_or_init(Feed::default),
            boom: None,
        }
    }

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

    const ALL: [Language; 7] = [
        Language::Rust,
        Language::Kotlin,
        Language::Java,
        Language::Go,
        Language::Python,
        Language::TypeScript,
        Language::Other,
    ];

    #[test]
    fn the_language_with_most_files_wins() {
        let paths = ["README.md", "a/Main.kt", "b/Other.kt", "c/lib.rs"];
        assert_eq!(Language::dominant(paths), Language::Kotlin);
        assert_eq!(Language::dominant(["notes.txt"]), Language::Other);
        // JavaScript has no figure of its own; its commits get the generic
        // one rather than a stand-in.
        assert_eq!(Language::dominant(["app.js", "b.jsx"]), Language::Other);
        assert_eq!(Language::dominant([]), Language::Other);
    }

    /// A diff with the awkward things a real one has in it: a tab, a line
    /// far longer than any terminal, characters that take two columns and
    /// ones that take none. The scene draws these, so the box must still
    /// come out square.
    const AWKWARD_DIFF: &str = "diff --git a/x.rs b/x.rs\n@@ -1,4 +1,4 @@\n\
         \x20fn wake() {\n\
         -\tlet sleepy = 1; // \u{5e73}\u{4eee}\u{540d} \u{1f980} e\u{301}\n\
         +\tlet sparkly = 2;\n\
         +    // and a line that runs on and on and on and on and on and on and \
         on and on and on and on and on and on and on and on and on past any \
         terminal anyone has ever sat in front of\n";

    #[test]
    fn every_scene_fills_the_box_it_was_given_and_holds_still() {
        // With nothing behind the scene, and with a real diff behind it: the
        // diff backdrop lays out lines that came out of a repository, not
        // glyphs of its own choosing.
        for feed in [Feed::default(), Feed::from_diff(AWKWARD_DIFF)] {
            for lang in ALL {
                for seed in 0..BACKDROPS.len() {
                    for (w, h) in [(90u16, 16u16), (120, 40), (200, 60)] {
                        for frame in 0..40 {
                            let ms = frame * 230;
                            let show = Show {
                                lang,
                                seed,
                                ms,
                                feed: &feed,
                                boom: None,
                            };
                            let lines = scene(show, w, h, true);
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
    }

    #[test]
    fn the_scene_moves_between_frames_and_says_what_the_model_is_doing() {
        let first = text_of(&scene(show(Language::Rust, 0, 0), 110, 30, true));
        // Eight milliseconds on, the figure has turned a hair and the light on
        // it has moved: a frame rate that high must not draw the same picture
        // twice.
        let next_frame = scene(show(Language::Rust, 0, 8), 110, 30, true);
        assert_ne!(
            scene(show(Language::Rust, 0, 0), 110, 30, true),
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
            .map(|seed| text_of(&scene(show(Language::Go, seed, 1_440), 110, 30, true)))
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
        assert!(scene(show(Language::Rust, 0, 0), 70, 2, true).is_empty());
        assert!(scene(show(Language::Rust, 0, 0), 8, 12, true).is_empty());
        assert!(scene(show(Language::Rust, 0, 0), 0, 0, true).is_empty());
    }

    #[test]
    fn a_flight_leaves_its_place_in_the_text_empty_until_it_lands() {
        let text = vec!["feat: add flights".to_owned()];
        let flight = |age_ms| Flight {
            row: 0,
            col: 10,
            text: "flights".to_owned(),
            age_ms,
        };
        let (w, h) = (110, 30);
        // Just left the model: the word is somewhere in the box, not yet
        // where it belongs.
        let early = text_of(&stage(
            show(Language::Rust, 0, 100),
            w,
            h,
            &text,
            &[flight(0)],
        ));
        assert_eq!(early.lines().count(), h as usize);
        assert!(early.starts_with("feat: add "), "{early}");
        assert!(!early.starts_with("feat: add flights"), "{early}");
        assert!(early.contains("flights"), "the word is in flight:\n{early}");
        // Landed and cooling: in place.
        let late = text_of(&stage(
            show(Language::Rust, 0, 500),
            w,
            h,
            &text,
            &[flight(FLIGHT_MS)],
        ));
        assert!(late.starts_with("feat: add flights"), "{late}");
        // Long gone: plain text, nothing else drawn for it.
        let done = text_of(&stage(
            show(Language::Rust, 0, 900),
            w,
            h,
            &text,
            &[flight(FLIGHT_TOTAL_MS)],
        ));
        assert!(done.starts_with("feat: add flights"), "{done}");
        assert_eq!(done.matches("flights").count(), 1, "{done}");
    }

    #[test]
    fn a_flight_moves_between_frames_and_stands_out_from_the_text() {
        let text = vec!["fix: it".to_owned()];
        let flight = |age_ms| Flight {
            row: 0,
            col: 5,
            text: "it".to_owned(),
            age_ms,
        };
        // Each letter in flight has a colour of its own, so the word is
        // found cell by cell rather than span by span.
        let at = |ms: u64, age: u64| {
            let lines = stage(show(Language::Go, 1, ms), 110, 30, &text, &[flight(age)]);
            lines.iter().enumerate().find_map(|(row, line)| {
                let cells: Vec<(char, Style)> = line
                    .spans
                    .iter()
                    .flat_map(|span| span.content.chars().map(move |c| (c, span.style)))
                    .collect();
                cells
                    .windows(2)
                    .position(|w| w[0].0 == 'i' && w[1].0 == 't')
                    .map(|col| (row, col, cells[col].1))
            })
        };
        let a = at(0, 0).expect("in the picture at take-off");
        let b = at(150, 150).expect("in the picture midway");
        let c = at(300, 300).expect("in the picture near landing");
        assert!(a != b && b != c, "the word moves: {a:?} {b:?} {c:?}");
        assert!(
            a.2.add_modifier.contains(Modifier::BOLD) && a.2.fg.is_some(),
            "in flight it is coloured and bold, unlike the text: {a:?}"
        );
        let landed = at(FLIGHT_TOTAL_MS, FLIGHT_TOTAL_MS).expect("in place at the end");
        assert_eq!((landed.0, landed.1), (0, 5));
        assert_eq!(
            landed.2,
            Style::default(),
            "settled text is plain: {landed:?}"
        );
    }

    #[test]
    fn without_a_scene_the_text_still_fills_the_box() {
        let text = vec!["feat: tiny".to_owned()];
        let flight = Flight {
            row: 0,
            col: 6,
            text: "tiny".to_owned(),
            age_ms: 0,
        };
        let lines = stage(show(Language::Rust, 0, 0), 20, 3, &text, &[flight]);
        assert_eq!(lines.len(), 3);
        let shown = text_of(&lines);
        assert!(shown.starts_with("feat: tiny"), "{shown}");
    }

    #[test]
    fn a_narrow_box_keeps_the_mascot_and_drops_the_pipeline() {
        let lines = scene(show(Language::Go, 1, 7_000), 40, 16, true);
        assert_eq!(lines.len(), 16);
        assert!(lines.iter().all(|line| line.width() == 40));
        let text = text_of(&lines);
        assert!(text.contains('O'), "the gopher is still there:\n{text}");
        assert!(
            !text.contains("\u{25b8}"),
            "no room for the token stream:\n{text}"
        );
    }

    #[test]
    fn the_stream_carries_the_diff_the_model_is_reading() {
        let feed = Feed::from_diff(
            "diff --git a/x.rs b/x.rs\nindex 111..222 100644\n@@ -1 +1 @@\n\
             -let sleepy = 1;\n+let sparkly = 2;\n",
        );
        // Headers are not code and do not go down the stream.
        let text: String = feed.chars.iter().map(|&(c, _)| c).collect();
        assert!(!text.contains("100644"), "{text}");
        assert!(text.contains("sparkly"), "{text}");

        // Over a few seconds the whole of a short diff goes past.
        let seen: String = (0..80)
            .map(|frame| {
                let show = Show {
                    lang: Language::Rust,
                    seed: 0,
                    ms: frame * 90,
                    feed: &feed,
                    boom: None,
                };
                text_of(&scene(show, 130, 30, false))
            })
            .collect();
        assert!(seen.contains("sleepy"), "the diff is legible in the stream");
        assert!(seen.contains("sparkly"), "both sides of it go in");

        // With no diff to show the stream still runs, on abstract tokens.
        let bare = text_of(&scene(show(Language::Rust, 0, 0), 130, 30, false));
        assert!(
            bare.contains(TOKEN_GLYPHS[0]) || bare.contains(TOKEN_GLYPHS[1]),
            "{bare}"
        );
    }

    #[test]
    fn the_diff_backdrop_scrolls_the_change_itself_past_the_reader() {
        let seed = BACKDROPS
            .iter()
            .position(|b| *b == Backdrop::Diff)
            .expect("the diff is one of the backdrops");
        let feed = Feed::from_diff(
            "diff --git a/x.rs b/x.rs\nindex 111..222 100644\n@@ -1,2 +1,2 @@\n\
             \x20fn wake() {\n-    let sleepy = 1;\n+\tlet sparkly = 2;\n",
        );
        let seen: String = (0..40)
            .map(|frame| {
                let show = Show {
                    lang: Language::Rust,
                    seed,
                    ms: frame * 120,
                    feed: &feed,
                    boom: None,
                };
                text_of(&scene(show, 130, 30, false))
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(seen.contains("let sparkly = 2;"), "{seen}");
        assert!(seen.contains("let sleepy = 1;"), "both sides show");
        assert!(seen.contains("fn wake() {"), "and the context around them");

        // And the code is highlighted where it lands, not painted one flat
        // colour: the keyword, the name and the marker are all told apart.
        let row = (0..40)
            .flat_map(|frame| {
                let show = Show {
                    lang: Language::Rust,
                    seed,
                    ms: frame * 120,
                    feed: &feed,
                    boom: None,
                };
                scene(show, 130, 30, false)
            })
            .find(|line| text_of(std::slice::from_ref(line)).contains("let sparkly = 2;"))
            .expect("a row of the backdrop carries the added line");
        let colors: std::collections::HashSet<Option<Color>> = row
            .spans
            .iter()
            .filter(|span| !span.content.trim().is_empty())
            .map(|span| span.style.fg)
            .collect();
        assert!(colors.len() > 2, "the line is highlighted: {colors:?}");

        // Nothing to show falls back to a stand-in rather than a blank sky.
        let bare = scene(show(Language::Rust, seed, 0), 130, 30, false);
        let sky = text_of(&bare[..6]);
        assert!(!sky.trim().is_empty(), "the sky is not left blank:\n{sky}");
    }

    #[test]
    fn the_trench_run_blows_the_station_when_the_first_word_flies_in() {
        let seed = BACKDROPS
            .iter()
            .position(|b| *b == Backdrop::Trench)
            .expect("the trench run is one of the backdrops");
        let mut show = show(Language::Rust, seed, 4_000);
        let waiting = text_of(&scene(show, 130, 34, true));
        assert!(waiting.contains(">=[O]=<"), "the run is on:\n{waiting}");
        show.ms = 8_000;
        show.boom = Some(6_000);
        let burning = text_of(&scene(show, 130, 34, false));
        let sky: String = burning.lines().take(12).collect::<Vec<_>>().join("\n");
        assert!(
            sky.matches(['#', '%', '@']).count() > 60,
            "the station is going up:\n{sky}"
        );
        assert_ne!(waiting, burning);
    }

    #[test]
    fn the_arena_backdrop_plays_out_a_round_above_the_mascot() {
        let seed = BACKDROPS
            .iter()
            .position(|b| *b == Backdrop::Arena)
            .expect("the arena is one of the backdrops");
        // The arena builds up as the wait goes on, so run the clock the way
        // a real wait would rather than jumping to one moment.
        let mut last = String::new();
        for frame in 0..600u64 {
            last = text_of(&scene(
                show(Language::Rust, seed, frame * 50),
                120,
                34,
                false,
            ));
        }
        let sky: String = last.lines().take(12).collect::<Vec<_>>().join("\n");
        assert!(
            sky.contains(['>', '<', '^', 'v']),
            "riders are out there:\n{sky}"
        );
        assert!(
            sky.contains('\u{2500}') || sky.contains('\u{2502}'),
            "and they have left trails:\n{sky}"
        );
        assert!(sky.contains('@'), "the snake is out too:\n{sky}");
    }
}
