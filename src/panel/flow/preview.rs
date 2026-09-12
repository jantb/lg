//! What a branch action would do, drawn as a small animated graph.
//!
//! The names of these actions say what they are called, not what they do to a
//! repository — "release" and "reset" both end in a force-push somewhere, and
//! only one of them throws history away. So each action gets a picture: the
//! branches it touches as lanes, the commits it moves between them as elbows,
//! and a marker travelling the route in the order the flow actually runs it.
//! Watching the marker is the explanation.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::{AppState, FlowAction, FlowRun};
use crate::ui::palette;

/// Fewest columns one commit takes, itself plus the dashes after it. The
/// diagram is drawn at this scale when the pane is tight, and stretched up
/// to `MAX_STRIDE` when there is room: a bigger picture is easier to read
/// and gives the marker a longer road to travel.
const MIN_STRIDE: usize = 3;
const MAX_STRIDE: usize = 16;

/// Fewest rows between one lane and the next: the lane, and one row under it
/// for the connectors to run down. Lanes are spread further apart when the pane
/// is tall, for the same reason the columns stretch — the panes this is drawn in
/// are far taller than the handful of rows a graph needs, and a diagram huddled
/// in the top corner wastes the room that would make it readable.
const MIN_ROW_STRIDE: usize = 2;
const MAX_ROW_STRIDE: usize = 8;

/// Fewest commits drawn past the last one a move lands on, so every lane runs
/// on afterwards instead of stopping dead at the merge. More are added when the
/// moves alone do not fill the pane's width, up to `MAX_TAIL_COMMITS` — past
/// that the picture is all quiet lane with the moves crowded into one corner,
/// which fills the pane without explaining any more of it.
const TAIL_COMMITS: usize = 2;
const MAX_TAIL_COMMITS: usize = 5;

/// Longest a lane label is drawn at. Branch names run long, and a label that
/// grows without limit pushes the track off the pane; the graph is the part
/// that explains, so the name is what gives way.
const MAX_LABEL: usize = 18;

/// Narrowest the diagram is worth drawing in. Below this the labels crowd the
/// track out and the picture stops explaining anything.
pub(super) const MIN_WIDTH: u16 = 34;

/// Rows the diagram and its caption need.
pub(super) const MIN_HEIGHT: u16 = 8;

/// A branch drawn as one row of the diagram.
struct Lane {
    label: String,
    color: Color,
}

/// Commits arriving in a lane, and whether what was there is kept.
struct Move {
    /// Lane they come from. `None` when nothing arrives and the destination is
    /// only being thrown away.
    from: Option<usize>,
    to: usize,
    /// Whether the destination's own history goes. This is the whole difference
    /// between a release and a reset, so it is drawn, not just written.
    discards: bool,
    /// Which step of the flow this happens on, so a run can put the marker
    /// where it has actually got to. Found in the flow's own step list rather
    /// than counted out here — a step whose wording changes should fail a test,
    /// not quietly leave the marker in the wrong place.
    step: Option<usize>,
}

/// The branches an action touches, what moves between them, and one line saying
/// it in words.
pub(super) struct Preview {
    lanes: Vec<Lane>,
    moves: Vec<Move>,
    caption: String,
    /// The steps the flow runs, the same list the progress pane narrates from.
    steps: Vec<String>,
    /// Columns one commit takes on a track, see `MIN_STRIDE`.
    stride: usize,
    /// Rows one lane takes, see `MIN_ROW_STRIDE`.
    row_stride: usize,
    /// Commits drawn after the last move, see `TAIL_COMMITS`.
    tail: usize,
}

/// Whether the picture is being drawn for the menu or for a run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Progress {
    /// Nothing is running: the marker walks the whole route on repeat and every
    /// move is drawn as it will be.
    Menu,
    /// A run stopped at this step: the marker travels the leg that step belongs
    /// to, and whatever the flow has not reached yet is drawn faint.
    Step(usize),
}

/// The colour a branch is drawn in, matching the deployment block: main is
/// magenta, the deploy branches keep their own colours, and anything else is
/// the working green of a feature branch.
fn branch_color(state: &AppState, name: &str) -> Color {
    let configured_base = crate::preferences::base_branch();
    let bare = name.strip_prefix("origin/").unwrap_or(name);
    if bare == configured_base.as_str() {
        return palette::LANE_MAIN;
    }
    for (env, color) in [
        (crate::git::ReleaseEnv::Dev, palette::LANE_DEV),
        (crate::git::ReleaseEnv::Test, palette::LANE_TEST),
    ] {
        if state.release_branch(env) == Some(bare) {
            return color;
        }
    }
    palette::LANE_FEATURE
}

impl Preview {
    fn new(caption: impl Into<String>, steps: Vec<String>) -> Self {
        Self {
            lanes: Vec::new(),
            moves: Vec::new(),
            caption: caption.into(),
            steps,
            stride: MIN_STRIDE,
            row_stride: MIN_ROW_STRIDE,
            tail: TAIL_COMMITS,
        }
    }

    /// Stretch the diagram into a pane `width` by `height`, beside labels
    /// `label_width` wide. Both scales are pushed as far as the room allows and
    /// never below their minimum, so a pane too small for even that is reported
    /// by `grid_cols` overrunning it.
    fn fit(&mut self, label_width: usize, width: u16, height: u16) {
        let track = usize::from(width).saturating_sub(label_width + 1);

        self.stride = MIN_STRIDE;
        for stride in (MIN_STRIDE..=MAX_STRIDE).rev() {
            self.stride = stride;
            if self.grid_cols() <= track {
                break;
            }
        }

        // The moves sit at the left of the track whatever the stride, so what
        // is left over after the widest stride that fits goes to the lanes
        // themselves: they run on to the edge of the pane rather than stopping
        // short of it.
        self.tail = TAIL_COMMITS;
        while self.tail < MAX_TAIL_COMMITS && self.grid_cols() + self.stride <= track {
            self.tail += 1;
        }

        // A terminal row is about twice as tall as a column is wide, so half
        // the stride is the spacing that makes a connector's drop look like a
        // commit's reach. Spread wider than that and a tall pane pulls the
        // lanes apart into something that reads as unrelated tracks.
        let widest_rows = (self.stride / 2).clamp(MIN_ROW_STRIDE, MAX_ROW_STRIDE);
        self.row_stride = MIN_ROW_STRIDE;
        for row_stride in (MIN_ROW_STRIDE..=widest_rows).rev() {
            self.row_stride = row_stride;
            if u16::try_from(self.grid_rows() + CAPTION_ROWS).unwrap_or(u16::MAX) <= height {
                break;
            }
        }
    }

    fn lane(&mut self, state: &AppState, name: &str) -> usize {
        self.lanes.push(Lane {
            label: name.to_string(),
            color: branch_color(state, name),
        });
        self.lanes.len() - 1
    }

    /// The step whose text contains `fragment`, which is how a move finds the
    /// operation it stands for in the flow's own list of them.
    fn step_at(&self, fragment: &str) -> Option<usize> {
        self.steps.iter().position(|step| step.contains(fragment))
    }

    fn merge(&mut self, from: usize, to: usize, at: &str) {
        let step = self.step_at(at);
        self.moves.push(Move {
            from: Some(from),
            to,
            discards: false,
            step,
        });
    }

    fn rebuild(&mut self, from: usize, to: usize, at: &str) {
        let step = self.step_at(at);
        self.moves.push(Move {
            from: Some(from),
            to,
            discards: true,
            step,
        });
    }

    fn drop_lane(&mut self, to: usize, at: &str) {
        let step = self.step_at(at);
        self.moves.push(Move {
            from: None,
            to,
            discards: true,
            step,
        });
    }

    /// Whether a move has happened yet. A move the flow has not reached is
    /// drawn faint, so a running picture reads as where the run has got to and
    /// not only as what it will eventually do.
    fn reached(&self, mv: &Move, progress: Progress) -> bool {
        match progress {
            Progress::Menu => true,
            Progress::Step(step) => mv.step.is_none_or(|at| at <= step),
        }
    }

    /// Where the marker sits, in grid coordinates.
    ///
    /// In the menu it walks the whole route on repeat. In a run it travels the
    /// leg of the step in progress, and while the flow is busy with a step the
    /// picture does not draw — stashing, fetching, checking out — it waits at
    /// the end of the last leg that finished.
    #[cfg(test)]
    fn marker(&self, progress: Progress, tick: usize) -> Option<(usize, usize)> {
        let (route, moving) = self.route(progress);
        let at = if moving { tick % route.len().max(1) } else { 0 };
        route.get(at).copied()
    }

    /// The cells the marker travels for this progress, and whether it is
    /// travelling them or waiting on the single cell returned.
    fn route(&self, progress: Progress) -> (Vec<(usize, usize)>, bool) {
        let step = match progress {
            Progress::Menu => return (self.path(), true),
            Progress::Step(step) => step,
        };
        let legs = self.legs();
        if let Some(idx) = self.moves.iter().position(|mv| mv.step == Some(step)) {
            return (legs[idx].clone(), true);
        }
        let done = self
            .moves
            .iter()
            .enumerate()
            .filter(|(_, mv)| mv.step.is_some_and(|at| at < step))
            .map(|(idx, _)| idx)
            .next_back();
        let waiting = match done {
            Some(idx) => legs[idx].last().copied(),
            // Nothing drawn has happened yet, so the marker waits where the
            // route starts rather than pretending to be further on.
            None => legs.first().and_then(|leg| leg.first()).copied(),
        };
        (waiting.into_iter().collect(), false)
    }

    /// The marker and its glow, at this reading of the animation clock: each
    /// lit cell of the route with how brightly, the head first at full.
    ///
    /// The marker's true position is a fraction of the way along the route,
    /// and every intensity is a continuous function of the distance from it.
    /// The cell ahead brightens as the marker approaches, the cell it is on
    /// holds full, and each cell behind dims with distance. So between two
    /// frames nothing jumps: the light slides from one cell into the next
    /// even though the glyphs can only sit on whole cells, and that sliding
    /// is what a character grid has to offer for motion.
    fn trail(&self, progress: Progress, clock_ms: u64) -> Vec<((usize, usize), f64)> {
        let (route, moving) = self.route(progress);
        if route.is_empty() {
            return Vec::new();
        }
        if !moving {
            return vec![(route[0], 1.0)];
        }
        let trail_cells = self.trail_cells();
        let position = clock_ms as f64 / self.cell_ms() as f64;
        let head = position.floor() as usize;
        let within = position - position.floor();
        let len = route.len();
        let mut cells = vec![(route[head % len], 1.0)];
        // In the menu the route loops, so the glow wraps with it; on a leg
        // of a real run it would spill into the legs on either side, which
        // are not where the flow is, so it is cut at both ends.
        let wraps = matches!(progress, Progress::Menu);
        if wraps || head + 1 < len {
            cells.push((route[(head + 1) % len], within));
        }
        for back in 1..=trail_cells {
            if !wraps && back > head {
                break;
            }
            let idx = (head + len - back % len) % len;
            let intensity = 1.0 - (within + back as f64) / (trail_cells as f64 + 1.0);
            cells.push((route[idx], intensity));
        }
        cells
    }

    /// Cells of glow behind the marker at this scale, see `TRAIL_CELLS`.
    fn trail_cells(&self) -> usize {
        TRAIL_CELLS * self.stride / MIN_STRIDE
    }

    /// How lit a cell has to be to be the marker rather than track under it,
    /// see `MARKER_HEAD_CELLS`.
    fn marker_cutoff(&self) -> f64 {
        1.0 - MARKER_HEAD_CELLS / (self.trail_cells() + 1) as f64
    }

    /// How long the marker rests on one cell at this scale, see
    /// `MARKER_CELL_MS`.
    fn cell_ms(&self) -> u64 {
        let scaled = MARKER_CELL_MS * MIN_STRIDE as u64 / self.stride as u64;
        scaled.max(1)
    }

    /// Commits each lane is drawn with: one per move, plus a tail so the lanes
    /// carry on past the last thing that happens to them.
    fn commits(&self) -> usize {
        self.moves.len() + self.tail
    }

    /// Column of commit `slot` on a lane's track.
    fn column(&self, slot: usize) -> usize {
        slot * self.stride
    }

    /// Grid row a lane's track sits on. Lanes are spaced out so the connectors
    /// between them have rows of their own to run down.
    fn row(&self, lane: usize) -> usize {
        lane * self.row_stride
    }

    /// Whether a grid row is a lane's track rather than the space between two.
    fn is_lane_row(&self, row: usize) -> bool {
        row.is_multiple_of(self.row_stride)
    }

    fn grid_rows(&self) -> usize {
        match self.lanes.len() {
            0 => 0,
            lanes => (lanes - 1) * self.row_stride + 1,
        }
    }

    fn grid_cols(&self) -> usize {
        self.column(self.commits().saturating_sub(1)) + 1
    }

    /// The route the marker walks, one leg per move, in the order the flow runs
    /// them — so a release visibly merges main before it merges the feature
    /// rather than showing both at once. Split by move rather than flattened,
    /// because a run walks only the leg of the step it is on.
    fn legs(&self) -> Vec<Vec<(usize, usize)>> {
        let last_col = self.grid_cols().saturating_sub(1);
        self.moves
            .iter()
            .enumerate()
            .map(|(idx, mv)| {
                let col = self.column(idx + 1);
                let to_row = self.row(mv.to);
                let Some(from) = mv.from else {
                    // Nothing arrives; the lane itself is what goes, so the
                    // marker runs the length of it.
                    return (0..=last_col).map(|x| (to_row, x)).collect();
                };
                let from_row = self.row(from);
                let mut leg: Vec<(usize, usize)> = (0..=col).map(|x| (from_row, x)).collect();
                let (lo, hi) = if from_row < to_row {
                    (from_row + 1, to_row)
                } else {
                    (to_row, from_row.saturating_sub(1))
                };
                let mut vertical: Vec<usize> = (lo..=hi).collect();
                if from_row > to_row {
                    vertical.reverse();
                }
                leg.extend(vertical.into_iter().map(|y| (y, col)));
                leg.extend((col..=last_col).map(|x| (to_row, x)));
                leg
            })
            .collect()
    }

    /// The whole route, end to end.
    fn path(&self) -> Vec<(usize, usize)> {
        self.legs().concat()
    }
}

/// The diagram for `run`, or `None` when there is nothing worth drawing.
///
/// Everything is read off the run rather than off the state: while a flow works
/// it checks other branches out, and a picture that followed the checkout would
/// redraw itself as something else halfway through.
pub(super) fn preview(state: &AppState, run: &FlowRun) -> Option<Preview> {
    let configured_base = crate::preferences::base_branch();
    if run.branch.is_empty() {
        return None;
    }
    let current = run.branch.clone();
    let steps = super::steps_for(run);
    let remote_main = format!("origin/{configured_base}");
    let created = run
        .input
        .clone()
        .unwrap_or_else(|| "new branch".to_string());

    Some(match run.action {
        FlowAction::MergeMain => {
            let mut preview = Preview::new(
                format!("{remote_main} lands in {current}, which is then pushed"),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let branch = preview.lane(state, &current);
            let remote = preview.lane(state, &format!("origin/{current}"));
            preview.merge(main, branch, &format!("merge {configured_base} into"));
            preview.merge(branch, remote, &format!("push {current}"));
            preview
        }
        FlowAction::ReleaseDev | FlowAction::ReleaseTest => {
            let target = run.target.clone()?;
            let mut preview = Preview::new(
                format!(
                    "{configured_base} and {current} merge into {target}, which is then pushed"
                ),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let feature = preview.lane(state, &format!("origin/{current}"));
            let deploy = preview.lane(state, &target);
            let remote = preview.lane(state, &format!("origin/{target}"));
            preview.merge(main, deploy, &format!("merge origin/{configured_base}"));
            preview.merge(feature, deploy, &format!("merge origin/{current}"));
            preview.merge(deploy, remote, "push HEAD to");
            preview
        }
        FlowAction::ResetDev | FlowAction::ResetTest => {
            let target = run.target.clone()?;
            let mut preview = Preview::new(
                format!(
                    "{target} is thrown away and rebuilt from {remote_main}, then force-pushed"
                ),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let deploy = preview.lane(state, &target);
            let remote = preview.lane(state, &format!("origin/{target}"));
            preview.rebuild(main, deploy, &format!("reset {target} to"));
            preview.rebuild(deploy, remote, "force push");
            preview
        }
        FlowAction::DiscardCheckout => {
            let mut preview = Preview::new(
                format!(
                    "everything in {current} that is not on its remote goes, untracked files included"
                ),
                steps,
            );
            let remote = preview.lane(state, &format!("origin/{current}"));
            let branch = preview.lane(state, &current);
            preview.rebuild(remote, branch, &format!("reset {current} to"));
            preview
        }
        FlowAction::NewFeature => {
            let mut preview = Preview::new(
                format!("a new branch starts from {remote_main} and is pushed"),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let branch = preview.lane(state, &created);
            let remote = preview.lane(state, &format!("origin/{created}"));
            preview.merge(main, branch, "create ");
            preview.merge(branch, remote, "push and set upstream");
            preview
        }
        FlowAction::TransferDiff => {
            let mut preview = Preview::new(
                format!(
                    "what {current} changed against {configured_base} is staged on a new branch"
                ),
                steps,
            );
            let from = preview.lane(state, &current);
            let branch = preview.lane(state, &created);
            preview.merge(from, branch, "apply diff");
            preview
        }
        FlowAction::CleanOrphans => {
            let mut preview = Preview::new(
                "local branches with no remote are deleted; tracked branches are left alone",
                steps,
            );
            preview.lane(state, "tracked");
            let orphan = preview.lane(state, "no upstream");
            preview.drop_lane(orphan, "delete orphan");
            preview
        }
    })
}

/// A label short enough to leave the track room, cut at the end because branch
/// names differ at the end far more often than at the start.
fn shorten(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL {
        return label.to_string();
    }
    let kept: String = label.chars().take(MAX_LABEL - 1).collect();
    format!("{kept}\u{2026}")
}

/// One cell of the drawn diagram.
#[derive(Clone, Copy)]
struct Cell {
    glyph: char,
    color: Color,
    dim: bool,
}

impl Cell {
    const BLANK: Self = Self {
        glyph: ' ',
        color: Color::DarkGray,
        dim: false,
    };
}

/// Draw the grid, without the marker. Moves a running flow has not reached yet
/// are drawn faint and colourless: the picture then says what has already been
/// done to the repository, not only what will be.
fn grid(preview: &Preview, progress: Progress) -> Vec<Vec<Cell>> {
    let cols = preview.grid_cols();
    let mut grid = vec![vec![Cell::BLANK; cols]; preview.grid_rows()];

    for (idx, lane) in preview.lanes.iter().enumerate() {
        let track = &mut grid[preview.row(idx)];
        for (col, cell) in track.iter_mut().enumerate() {
            *cell = Cell {
                glyph: if col % preview.stride == 0 {
                    '\u{25cf}'
                } else {
                    '\u{2500}'
                },
                color: lane.color,
                dim: false,
            };
        }
    }

    for (idx, mv) in preview.moves.iter().enumerate() {
        let col = preview.column(idx + 1);
        let to_row = preview.row(mv.to);
        let reached = preview.reached(mv, progress);
        let color = if reached {
            preview.lanes[mv.to].color
        } else {
            palette::LANE_PENDING
        };
        // History is only struck out once the flow has actually thrown it away.
        let lost_color = if reached {
            palette::LANE_LOST
        } else {
            palette::LANE_PENDING
        };

        let Some(from) = mv.from else {
            // A lane that is only deleted: the whole of it goes, so the whole
            // of it is struck out rather than a stretch of it.
            for (lost, cell) in grid[to_row].iter_mut().enumerate() {
                *cell = Cell {
                    glyph: if lost % preview.stride == 0 {
                        '\u{2717}'
                    } else {
                        ' '
                    },
                    color: lost_color,
                    dim: true,
                };
            }
            continue;
        };

        if mv.discards {
            // Everything the destination had before this point is what goes.
            for lost in (0..col).step_by(preview.stride) {
                grid[to_row][lost] = Cell {
                    glyph: '\u{2717}',
                    color: lost_color,
                    dim: true,
                };
            }
        }

        let from_row = preview.row(from);
        let (lo, hi) = if from_row < to_row {
            (from_row + 1, to_row.saturating_sub(1))
        } else {
            (to_row + 1, from_row.saturating_sub(1))
        };
        for row in lo..=hi.max(lo) {
            if row >= grid.len() || row == from_row || row == to_row {
                continue;
            }
            // A connector reaching past its neighbour crosses a lane on the way.
            // Drawing straight through would rub that lane out and leave a
            // branch looking like it stops here.
            let crosses_lane = preview.is_lane_row(row);
            grid[row][col] = Cell {
                glyph: if crosses_lane { '\u{253c}' } else { '\u{2502}' },
                color: if reached {
                    preview.lanes[from].color
                } else {
                    palette::LANE_PENDING
                },
                dim: !reached,
            };
        }

        grid[to_row][col] = Cell {
            glyph: if mv.discards { '\u{25c6}' } else { '\u{25cd}' },
            color: if mv.discards { lost_color } else { color },
            dim: !reached,
        };
    }

    grid
}

/// How long the marker rests on each cell of its route at `MIN_STRIDE`. A
/// stretched diagram is not a slower one: the marker's pace is one commit per
/// so long, not one character, so the time it rests on a cell shrinks as the
/// commits are drawn further apart and the route it loops stays the same length
/// in seconds however big the pane is.
pub(super) const MARKER_CELL_MS: u64 = 90;
/// How many cells of glow the marker leaves behind it at `MIN_STRIDE`; it grows
/// with the stride so the trail stays the same length in commits.
const TRAIL_CELLS: usize = 7;

/// Cells at the front of the trail drawn as the marker itself rather than as a
/// lit stretch of track. This is a count and not a fraction of the trail: a
/// stretched diagram earns a longer tail, but a head that grew with it would
/// read as a bar sliding along the lane instead of something travelling it.
const MARKER_HEAD_CELLS: f64 = 3.0;

/// Rows `lines` adds under the diagram: a blank one and the caption.
pub(super) const CAPTION_ROWS: usize = 2;

/// The diagram as lines, with the marker where `progress` and the animation
/// clock (`clock_ms`) put it.
///
/// `width` and `height` are what the pane can show; the diagram is stretched to
/// fill them, and one that would not fit even at its smallest is dropped rather
/// than drawn cut in half, since half a graph says the wrong thing.
pub(super) fn lines(
    state: &AppState,
    run: &FlowRun,
    progress: Progress,
    clock_ms: u64,
    width: u16,
    height: u16,
) -> Vec<Line<'static>> {
    let Some(mut preview) = preview(state, run) else {
        return Vec::new();
    };
    let labels: Vec<String> = preview
        .lanes
        .iter()
        .map(|lane| shorten(&lane.label))
        .collect();
    let label_width = labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0);
    preview.fit(label_width, width, height);
    let needed = label_width + 1 + preview.grid_cols();
    if u16::try_from(needed).unwrap_or(u16::MAX) > width {
        return vec![Line::from(Span::styled(
            preview.caption,
            Style::default().fg(palette::TEXT_IDLE),
        ))];
    }

    let cells = grid(&preview, progress);
    let trail = preview.trail(progress, clock_ms);
    let marker_cutoff = preview.marker_cutoff();

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(cells.len() + 2);
    for (row, cols) in cells.iter().enumerate() {
        let mut spans = Vec::with_capacity(cols.len() + 1);
        if preview.is_lane_row(row) {
            let idx = row / preview.row_stride;
            let lane = &preview.lanes[idx];
            spans.push(Span::styled(
                format!("{:>label_width$} ", labels[idx]),
                Style::default().fg(lane.color).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw(" ".repeat(label_width + 1)));
        }
        for (col, cell) in cols.iter().enumerate() {
            let glow = trail
                .iter()
                .find(|(at, _)| *at == (row, col))
                .map(|(_, intensity)| *intensity);
            let (glyph, style) = if let Some(intensity) = glow {
                // The marker's own glyph goes to the brightest few cells:
                // the head, the couple just behind it, and the cell it is
                // sliding into once the slide is nearly done. Everything
                // else keeps the route's glyph and is only lit, from
                // near-white just behind the head down to the resting accent
                // where the glow rejoins the diagram.
                let glyph = if intensity >= marker_cutoff {
                    '\u{25c9}'
                } else {
                    cell.glyph
                };
                let color = if intensity >= 1.0 {
                    palette::glow(0.8 + 0.2 * palette::breath(clock_ms, 700))
                } else {
                    palette::glow(intensity)
                };
                (
                    glyph,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )
            } else {
                let mut style = Style::default().fg(cell.color);
                if cell.dim {
                    style = style.add_modifier(Modifier::DIM);
                }
                (cell.glyph, style)
            };
            spans.push(Span::styled(glyph.to_string(), style));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        preview.caption,
        Style::default().fg(palette::TEXT_IDLE),
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pane with room to spare, the way the flow modal gives it; the tests
    /// about fitting name their own sizes.
    const ROOMY_HEIGHT: u16 = 24;

    /// A checkout where every branch action applies: a feature branch checked
    /// out and selected, and both deploy branches present.
    fn state_with_deploy_branches() -> AppState {
        let mut state = AppState::new();
        state.branch = Some("feature/parser".into());
        state.branches = vec![crate::git::Branch {
            name: "feature/parser".into(),
            is_current: true,
            upstream: Some("origin/feature/parser".into()),
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 1,
            last_commit_unix: None,
        }];
        state.branches_idx = 0;
        state.release_branches =
            crate::git::ReleaseBranches::new(Some("develop".to_string()), Some("test".to_string()));
        state
    }

    /// The run the menu would resolve for `action`, so the tests draw what the
    /// menu draws rather than a hand-built approximation of it.
    fn run(state: &AppState, action: FlowAction) -> FlowRun {
        super::super::flow_run(state, action)
    }

    fn text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The colours one glyph was drawn in, for the assertions that are about
    /// colour rather than shape. Every cell of the grid is its own span, so a
    /// span holding one glyph is one cell.
    fn colors(lines: &[Line<'static>], glyph: char) -> Vec<Color> {
        let wanted = glyph.to_string();
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter(|span| span.content.as_ref() == wanted)
            .filter_map(|span| span.style.fg)
            .collect()
    }

    /// Every action the menu can offer has to explain itself; one that draws
    /// nothing is a blank pane where the explanation should be.
    #[test]
    fn every_offered_action_has_something_to_show() {
        let state = state_with_deploy_branches();
        let offered = super::super::available_actions(&state);
        assert_eq!(
            offered.len(),
            FlowAction::ALL.len(),
            "this checkout should offer every action: {offered:?}"
        );
        for action in offered {
            let lines = lines(
                &state,
                &run(&state, action),
                Progress::Menu,
                0,
                80,
                ROOMY_HEIGHT,
            );
            assert!(
                !lines.is_empty(),
                "{action:?} should draw something at a usable width"
            );
            assert!(
                lines.len() > 1,
                "{action:?} should draw a graph, not just its caption"
            );
        }
    }

    /// The picture is the point: a release names both branches it merges, in
    /// the order it merges them.
    #[test]
    fn a_release_shows_main_and_the_feature_arriving_in_the_deploy_branch() {
        let state = state_with_deploy_branches();
        let drawn = text(&lines(
            &state,
            &run(&state, FlowAction::ReleaseTest),
            Progress::Menu,
            0,
            80,
            ROOMY_HEIGHT,
        ));

        assert!(drawn.contains("origin/main"), "{drawn}");
        assert!(drawn.contains(&shorten("origin/feature/parser")), "{drawn}");
        assert!(drawn.contains("test"), "{drawn}");
        assert!(drawn.contains("origin/test"), "{drawn}");
    }

    /// What separates a reset from a release is that the target's own history
    /// goes, so the drawing has to show that and not only say it.
    #[test]
    fn a_reset_marks_the_history_it_throws_away_and_a_release_does_not() {
        let state = state_with_deploy_branches();
        let reset = text(&lines(
            &state,
            &run(&state, FlowAction::ResetTest),
            Progress::Menu,
            0,
            80,
            ROOMY_HEIGHT,
        ));
        let release = text(&lines(
            &state,
            &run(&state, FlowAction::ReleaseTest),
            Progress::Menu,
            0,
            80,
            ROOMY_HEIGHT,
        ));

        assert!(
            reset.contains('\u{2717}'),
            "a reset should mark the commits it drops: {reset}"
        );
        assert!(
            !release.contains('\u{2717}'),
            "a release drops nothing: {release}"
        );
    }

    #[test]
    fn the_marker_moves_as_the_clock_ticks() {
        let state = state_with_deploy_branches();
        let first = text(&lines(
            &state,
            &run(&state, FlowAction::MergeMain),
            Progress::Menu,
            0,
            80,
            ROOMY_HEIGHT,
        ));
        let moved = (1..40)
            .map(|tick| {
                text(&lines(
                    &state,
                    &run(&state, FlowAction::MergeMain),
                    Progress::Menu,
                    tick * MARKER_CELL_MS,
                    80,
                    ROOMY_HEIGHT,
                ))
            })
            .any(|frame| frame != first);
        assert!(moved, "the diagram should animate: {first}");
    }

    /// It walks the whole route and comes back, rather than running off the end.
    #[test]
    fn the_marker_visits_every_step_and_loops() {
        let state = state_with_deploy_branches();
        let preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
        let path = preview.path();
        assert!(path.len() > 4, "the route should have steps to walk");

        let rows = preview.grid_rows();
        let cols = preview.grid_cols();
        for (row, col) in &path {
            assert!(*row < rows && *col < cols, "({row},{col}) is off the grid");
        }
        assert_eq!(
            path[0],
            path[path.len() % path.len().max(1)],
            "the route should be walked from its start"
        );
    }

    /// A pane too narrow for the graph gets the sentence instead of a picture
    /// chopped in half, which would read as a different flow entirely.
    #[test]
    fn a_narrow_pane_falls_back_to_the_caption() {
        let state = state_with_deploy_branches();
        let lines = lines(
            &state,
            &run(&state, FlowAction::ReleaseTest),
            Progress::Menu,
            0,
            12,
            ROOMY_HEIGHT,
        );
        assert_eq!(lines.len(), 1, "no room for a graph");
        assert!(text(&lines).contains("merge into"), "{:?}", text(&lines));
    }

    /// A long branch name gives way rather than pushing the track off the pane,
    /// but a name that fits is left exactly as it is.
    #[test]
    fn long_branch_names_are_cut_and_short_ones_are_not() {
        assert_eq!(shorten("origin/test"), "origin/test");
        let long = shorten("origin/feature/send-cv-t-alvminnelig");
        assert_eq!(long.chars().count(), MAX_LABEL);
        assert!(long.ends_with('\u{2026}'), "{long}");
        assert!(long.starts_with("origin/feature"), "{long}");
    }

    #[test]
    fn a_repository_with_no_branch_draws_nothing() {
        let state = AppState::new();
        assert!(
            lines(
                &state,
                &run(&state, FlowAction::MergeMain),
                Progress::Menu,
                0,
                80,
                ROOMY_HEIGHT
            )
            .is_empty()
        );
    }

    /// Every move has to know which step it happens on, or a running flow puts
    /// the marker somewhere the flow is not. The step is found by matching the
    /// flow's own wording, so this is what catches a step being reworded.
    #[test]
    fn every_move_happens_on_a_step_the_flow_reports() {
        let state = state_with_deploy_branches();
        for action in super::super::available_actions(&state) {
            let run = run(&state, action);
            let preview = preview(&state, &run).expect("a picture for every action");
            for (idx, mv) in preview.moves.iter().enumerate() {
                assert!(
                    mv.step.is_some(),
                    "{action:?} move {idx} matches none of {:?}",
                    preview.steps
                );
            }
        }
    }

    /// While a flow runs, the marker is where the flow is: travelling the leg of
    /// the step in progress, waiting at the start until the run reaches anything
    /// the picture draws, and holding where it landed once the last move is done.
    #[test]
    fn the_marker_follows_the_step_a_run_is_on() {
        let state = state_with_deploy_branches();
        let preview = preview(&state, &run(&state, FlowAction::ReleaseTest)).expect("preview");
        let legs = preview.legs();
        let first = preview.moves[0].step.expect("the first merge has a step");
        let last = preview
            .moves
            .last()
            .and_then(|mv| mv.step)
            .expect("the push has a step");

        // A release stashes, pushes and checks out before it merges anything, so
        // at the first step there is nowhere for the marker to have got to yet.
        assert_eq!(
            preview.marker(Progress::Step(0), 7),
            legs[0].first().copied(),
            "the marker should wait at the start of the route"
        );

        let on_leg = preview
            .marker(Progress::Step(first), 3)
            .expect("a marker while merging");
        assert!(
            legs[0].contains(&on_leg),
            "{on_leg:?} is not on the leg of the merge in progress"
        );
        let moved = (0..legs[0].len())
            .map(|tick| preview.marker(Progress::Step(first), tick))
            .any(|at| at != preview.marker(Progress::Step(first), 0));
        assert!(
            moved,
            "the marker should travel the leg while the step runs"
        );

        assert_eq!(
            preview.marker(Progress::Step(last + 1), 0),
            legs.last().and_then(|leg| leg.last()).copied(),
            "once the last move is done the marker holds where it landed"
        );
    }

    /// A reset only throws history away when it gets there. Struck out in red
    /// from the first step, the picture would be describing a repository the
    /// flow has not touched yet.
    #[test]
    fn history_goes_red_only_once_the_flow_has_thrown_it_away() {
        let state = state_with_deploy_branches();
        let run = run(&state, FlowAction::ResetTest);
        let at = super::super::steps_for(&run)
            .iter()
            .position(|step| step.contains("reset test to"))
            .expect("a reset step");

        let waiting = colors(
            &lines(&state, &run, Progress::Step(0), 0, 80, ROOMY_HEIGHT),
            '\u{2717}',
        );
        let done = colors(
            &lines(&state, &run, Progress::Step(at), 0, 80, ROOMY_HEIGHT),
            '\u{2717}',
        );

        assert!(
            !waiting.is_empty(),
            "the reset should show what it will drop"
        );
        assert!(
            waiting.iter().all(|color| *color == palette::LANE_PENDING),
            "nothing is lost yet, so nothing should be red: {waiting:?}"
        );
        assert!(
            done.contains(&palette::LANE_LOST),
            "the history the reset just dropped should be red: {done:?}"
        );
    }

    /// The deploy branches keep the colours the deployment block gives them, so
    /// the same branch is the same colour wherever it is drawn.
    #[test]
    fn branches_keep_their_colours() {
        let state = state_with_deploy_branches();
        assert_eq!(branch_color(&state, "origin/main"), palette::LANE_MAIN);
        assert_eq!(branch_color(&state, "test"), palette::LANE_TEST);
        assert_eq!(branch_color(&state, "origin/develop"), palette::LANE_DEV);
        assert_eq!(
            branch_color(&state, "feature/parser"),
            palette::LANE_FEATURE
        );
    }

    #[test]
    fn the_glow_slides_between_frames_instead_of_jumping() {
        let state = state_with_deploy_branches();
        let preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
        let at_cell = preview.trail(Progress::Menu, 10 * MARKER_CELL_MS);
        let later = preview.trail(Progress::Menu, 10 * MARKER_CELL_MS + MARKER_CELL_MS / 2);
        let head = preview.marker(Progress::Menu, 10).unwrap();
        let next = preview.marker(Progress::Menu, 11).unwrap();
        let intensity = |cells: &[((usize, usize), f64)], at| {
            cells
                .iter()
                .find(|(cell, _)| *cell == at)
                .map(|(_, intensity)| *intensity)
                .unwrap_or(0.0)
        };

        assert_eq!(at_cell[0], (head, 1.0), "the head leads at full");
        assert_eq!(intensity(&later, head), 1.0, "and holds for the step");
        assert!(
            intensity(&later, next) > intensity(&at_cell, next),
            "the cell ahead should brighten as the marker approaches"
        );
        // Only cells the route passes once: one it crosses twice is lit by
        // whichever pass is brighter, which is not what is measured here.
        let route = preview.path();
        let passed_once = |cell: &(usize, usize)| route.iter().filter(|c| *c == cell).count() == 1;
        let tail: Vec<_> = at_cell
            .iter()
            .filter(|(cell, _)| *cell != head && *cell != next && passed_once(cell))
            .collect();
        assert!(!tail.is_empty(), "the head should have a tail");
        for (cell, before) in tail {
            assert!(
                intensity(&later, *cell) < *before,
                "the tail should dim as the step wears on, not hold and jump"
            );
        }
    }

    /// The tail shows where the run has been, and before the first cell of a
    /// leg it has been nowhere on that leg.
    #[test]
    fn a_running_leg_has_no_tail_behind_its_start() {
        let state = state_with_deploy_branches();
        let preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
        let first = preview
            .moves
            .iter()
            .filter_map(|mv| mv.step)
            .min()
            .expect("a drawn step");
        let legs = preview.legs();
        let start = legs[0][0];
        let ahead = legs[0][1];
        for (cell, _) in preview.trail(Progress::Step(first), 0) {
            assert!(
                cell == start || cell == ahead,
                "{cell:?} is behind the start of the leg"
            );
        }
    }

    #[test]
    fn the_diagram_grows_into_the_room_it_is_given() {
        let state = state_with_deploy_branches();
        let mut preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
        preview.fit(10, 200, 40);
        let (wide, tall) = (preview.grid_cols(), preview.grid_rows());
        preview.fit(10, 25, 8);
        let (narrow, short) = (preview.grid_cols(), preview.grid_rows());
        assert!(wide > narrow, "a wide pane should get a wider picture");
        assert!(tall > short, "a tall pane should get a taller picture");
        assert!(
            u16::try_from(10 + 1 + narrow).unwrap() <= 25,
            "and a small one should still fit across"
        );
        assert!(
            u16::try_from(short + CAPTION_ROWS).unwrap() <= 8,
            "and down"
        );
    }

    /// The picture is drawn to be watched, so growing it must not turn the loop
    /// into a slideshow: the marker's pace is one commit per so long, whatever
    /// scale the commits are drawn at.
    #[test]
    fn a_bigger_picture_is_not_a_slower_one() {
        let state = state_with_deploy_branches();
        let mut preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
        let loop_ms = |preview: &Preview| preview.path().len() as u64 * preview.cell_ms();
        preview.fit(10, 25, 8);
        let small = loop_ms(&preview);
        preview.fit(10, 90, 40);
        let big = loop_ms(&preview);
        assert!(
            big < small * 2,
            "a diagram {} times the size should not loop {} times slower",
            preview.stride,
            big / small.max(1)
        );
    }

    /// Room left under a diagram that could have used it is room wasted: the
    /// lanes run to the edge of the pane rather than stopping short of it.
    #[test]
    fn the_lanes_run_the_width_of_the_pane() {
        let state = state_with_deploy_branches();
        let mut preview = preview(&state, &run(&state, FlowAction::TransferDiff)).expect("preview");
        preview.fit(10, 90, 40);
        assert!(
            preview.grid_cols() + 10 + 1 > 90 - preview.stride,
            "{} columns leaves more than a commit of the 90 unused",
            preview.grid_cols()
        );
    }
}
