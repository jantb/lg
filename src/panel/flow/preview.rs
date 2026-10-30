//! What a branch action would do, drawn as a small animated graph.
//!
//! The names of these actions say what they are called, not what they do to a
//! repository — "release" and "reset" both end in a force-push somewhere, and
//! only one of them throws history away. So each action gets a picture: the
//! branches it touches as lanes, the commits it moves between them as elbows,
//! and a marker travelling the route in the order the flow actually runs it.
//! Watching the marker is the explanation.

use ratatui::style::Color;

use crate::state::AppState;
use crate::ui::palette;

use draw::{MARKER_HEAD_CELLS, TRAIL_CELLS};

mod actions;
mod draw;

pub(super) use actions::preview;
pub(super) use draw::{CAPTION_ROWS, MARKER_CELL_MS, lines};

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

#[cfg(test)]
mod tests;
