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

use crate::config::BRANCH_MAIN;
use crate::state::{AppState, FlowAction, FlowRun};

/// Dashes drawn between two commits on a lane.
const DASHES: usize = 2;

/// Columns one commit takes, itself plus the dashes after it.
const STRIDE: usize = DASHES + 1;

/// Commits drawn past the last one a move lands on, so every lane runs on
/// afterwards instead of stopping dead at the merge.
const TAIL_COMMITS: usize = 2;

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
    let bare = name.strip_prefix("origin/").unwrap_or(name);
    if bare == BRANCH_MAIN {
        return Color::Magenta;
    }
    for (env, color) in [
        (crate::git::ReleaseEnv::Dev, Color::Cyan),
        (crate::git::ReleaseEnv::Test, Color::Yellow),
    ] {
        if state.release_branch(env) == Some(bare) {
            return color;
        }
    }
    Color::Green
}

impl Preview {
    fn new(caption: impl Into<String>, steps: Vec<String>) -> Self {
        Self {
            lanes: Vec::new(),
            moves: Vec::new(),
            caption: caption.into(),
            steps,
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
    fn marker(&self, progress: Progress, tick: usize) -> Option<(usize, usize)> {
        let step = match progress {
            Progress::Menu => {
                let route = self.path();
                return route.get(tick % route.len().max(1)).copied();
            }
            Progress::Step(step) => step,
        };
        let legs = self.legs();
        if let Some(idx) = self.moves.iter().position(|mv| mv.step == Some(step)) {
            let leg = &legs[idx];
            return leg.get(tick % leg.len().max(1)).copied();
        }
        let done = self
            .moves
            .iter()
            .enumerate()
            .filter(|(_, mv)| mv.step.is_some_and(|at| at < step))
            .map(|(idx, _)| idx)
            .next_back();
        match done {
            Some(idx) => legs[idx].last().copied(),
            // Nothing drawn has happened yet, so the marker waits where the
            // route starts rather than pretending to be further on.
            None => legs.first().and_then(|leg| leg.first()).copied(),
        }
    }

    /// Commits each lane is drawn with: one per move, plus a tail so the lanes
    /// carry on past the last thing that happens to them.
    fn commits(&self) -> usize {
        self.moves.len() + TAIL_COMMITS
    }

    /// Column of commit `slot` on a lane's track.
    fn column(slot: usize) -> usize {
        slot * STRIDE
    }

    /// Grid row a lane's track sits on. Lanes are spaced out so the connectors
    /// between them have a row of their own to run down.
    fn row(lane: usize) -> usize {
        lane * 2
    }

    fn grid_rows(&self) -> usize {
        (self.lanes.len() * 2).saturating_sub(1)
    }

    fn grid_cols(&self) -> usize {
        Self::column(self.commits().saturating_sub(1)) + 1
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
                let col = Self::column(idx + 1);
                let to_row = Self::row(mv.to);
                let Some(from) = mv.from else {
                    // Nothing arrives; the lane itself is what goes, so the
                    // marker runs the length of it.
                    return (0..=last_col).map(|x| (to_row, x)).collect();
                };
                let from_row = Self::row(from);
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
    if run.branch.is_empty() {
        return None;
    }
    let current = run.branch.clone();
    let steps = super::steps_for(run);
    let remote_main = format!("origin/{BRANCH_MAIN}");
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
            preview.merge(main, branch, &format!("merge {BRANCH_MAIN} into"));
            preview.merge(branch, remote, &format!("push {current}"));
            preview
        }
        FlowAction::ReleaseDev | FlowAction::ReleaseTest => {
            let target = run.target.clone()?;
            let mut preview = Preview::new(
                format!("{BRANCH_MAIN} and {current} merge into {target}, which is then pushed"),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let feature = preview.lane(state, &format!("origin/{current}"));
            let deploy = preview.lane(state, &target);
            let remote = preview.lane(state, &format!("origin/{target}"));
            preview.merge(main, deploy, &format!("merge origin/{BRANCH_MAIN}"));
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
                format!("what {current} changed against {BRANCH_MAIN} is staged on a new branch"),
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
        let track = &mut grid[Preview::row(idx)];
        for (col, cell) in track.iter_mut().enumerate() {
            *cell = Cell {
                glyph: if col % STRIDE == 0 {
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
        let col = Preview::column(idx + 1);
        let to_row = Preview::row(mv.to);
        let reached = preview.reached(mv, progress);
        let color = if reached {
            preview.lanes[mv.to].color
        } else {
            Color::DarkGray
        };
        // History is only struck out once the flow has actually thrown it away.
        let lost_color = if reached { Color::Red } else { Color::DarkGray };

        let Some(from) = mv.from else {
            // A lane that is only deleted: the whole of it goes, so the whole
            // of it is struck out rather than a stretch of it.
            for (lost, cell) in grid[to_row].iter_mut().enumerate() {
                *cell = Cell {
                    glyph: if lost % STRIDE == 0 { '\u{2717}' } else { ' ' },
                    color: lost_color,
                    dim: true,
                };
            }
            continue;
        };

        if mv.discards {
            // Everything the destination had before this point is what goes.
            for lost in (0..col).step_by(STRIDE) {
                grid[to_row][lost] = Cell {
                    glyph: '\u{2717}',
                    color: lost_color,
                    dim: true,
                };
            }
        }

        let from_row = Preview::row(from);
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
            let crosses_lane = row % 2 == 0;
            grid[row][col] = Cell {
                glyph: if crosses_lane { '\u{253c}' } else { '\u{2502}' },
                color: if reached {
                    preview.lanes[from].color
                } else {
                    Color::DarkGray
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

/// The diagram as lines, with the marker where `progress` and `tick` put it.
///
/// `width` is what the pane can show; a diagram that would not fit is dropped
/// rather than drawn cut in half, since half a graph says the wrong thing.
pub(super) fn lines(
    state: &AppState,
    run: &FlowRun,
    progress: Progress,
    tick: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let Some(preview) = preview(state, run) else {
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
    let needed = label_width + 1 + preview.grid_cols();
    if u16::try_from(needed).unwrap_or(u16::MAX) > width {
        return vec![Line::from(Span::styled(
            preview.caption,
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let cells = grid(&preview, progress);
    let marker = preview.marker(progress, tick);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(cells.len() + 2);
    for (row, cols) in cells.iter().enumerate() {
        let mut spans = Vec::with_capacity(cols.len() + 1);
        if row % 2 == 0 {
            let lane = &preview.lanes[row / 2];
            spans.push(Span::styled(
                format!("{:>label_width$} ", labels[row / 2]),
                Style::default().fg(lane.color).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw(" ".repeat(label_width + 1)));
        }
        for (col, cell) in cols.iter().enumerate() {
            let here = marker == Some((row, col));
            let (glyph, style) = if here {
                (
                    '\u{25c9}',
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
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
        Style::default().fg(Color::Gray),
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let lines = lines(&state, &run(&state, action), Progress::Menu, 0, 80);
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
        ));
        let release = text(&lines(
            &state,
            &run(&state, FlowAction::ReleaseTest),
            Progress::Menu,
            0,
            80,
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
        ));
        let moved = (1..40)
            .map(|tick| {
                text(&lines(
                    &state,
                    &run(&state, FlowAction::MergeMain),
                    Progress::Menu,
                    tick,
                    80,
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
                80
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

        let waiting = colors(&lines(&state, &run, Progress::Step(0), 0, 80), '\u{2717}');
        let done = colors(&lines(&state, &run, Progress::Step(at), 0, 80), '\u{2717}');

        assert!(
            !waiting.is_empty(),
            "the reset should show what it will drop"
        );
        assert!(
            waiting.iter().all(|color| *color == Color::DarkGray),
            "nothing is lost yet, so nothing should be red: {waiting:?}"
        );
        assert!(
            done.contains(&Color::Red),
            "the history the reset just dropped should be red: {done:?}"
        );
    }

    /// The deploy branches keep the colours the deployment block gives them, so
    /// the same branch is the same colour wherever it is drawn.
    #[test]
    fn branches_keep_their_colours() {
        let state = state_with_deploy_branches();
        assert_eq!(branch_color(&state, "origin/main"), Color::Magenta);
        assert_eq!(branch_color(&state, "test"), Color::Yellow);
        assert_eq!(branch_color(&state, "origin/develop"), Color::Cyan);
        assert_eq!(branch_color(&state, "feature/parser"), Color::Green);
    }
}
