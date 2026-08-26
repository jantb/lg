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
use crate::state::{AppState, FlowAction};

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
}

/// The branches an action touches, what moves between them, and one line saying
/// it in words.
pub(super) struct Preview {
    lanes: Vec<Lane>,
    moves: Vec<Move>,
    caption: String,
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
    fn new(caption: impl Into<String>) -> Self {
        Self {
            lanes: Vec::new(),
            moves: Vec::new(),
            caption: caption.into(),
        }
    }

    fn lane(&mut self, state: &AppState, name: &str) -> usize {
        self.lanes.push(Lane {
            label: name.to_string(),
            color: branch_color(state, name),
        });
        self.lanes.len() - 1
    }

    fn merge(&mut self, from: usize, to: usize) {
        self.moves.push(Move {
            from: Some(from),
            to,
            discards: false,
        });
    }

    fn rebuild(&mut self, from: usize, to: usize) {
        self.moves.push(Move {
            from: Some(from),
            to,
            discards: true,
        });
    }

    fn drop_lane(&mut self, to: usize) {
        self.moves.push(Move {
            from: None,
            to,
            discards: true,
        });
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

    /// Where the marker is at each step of the animation, in grid coordinates.
    ///
    /// The route runs one move at a time, in the order the flow runs them, so a
    /// release visibly merges main before it merges the feature rather than
    /// showing both at once.
    fn path(&self) -> Vec<(usize, usize)> {
        let mut path = Vec::new();
        let last_col = self.grid_cols().saturating_sub(1);
        for (idx, mv) in self.moves.iter().enumerate() {
            let col = Self::column(idx + 1);
            let to_row = Self::row(mv.to);
            let Some(from) = mv.from else {
                // Nothing arrives; the lane itself is what goes, so the marker
                // runs the length of it.
                path.extend((0..=last_col).map(|x| (to_row, x)));
                continue;
            };
            let from_row = Self::row(from);
            path.extend((0..=col).map(|x| (from_row, x)));
            let (lo, hi) = if from_row < to_row {
                (from_row + 1, to_row)
            } else {
                (to_row, from_row.saturating_sub(1))
            };
            let mut vertical: Vec<usize> = (lo..=hi).collect();
            if from_row > to_row {
                vertical.reverse();
            }
            path.extend(vertical.into_iter().map(|y| (y, col)));
            path.extend((col..=last_col).map(|x| (to_row, x)));
        }
        path
    }
}

/// The diagram for `action`, or `None` when there is nothing worth drawing.
pub(super) fn preview(state: &AppState, action: FlowAction) -> Option<Preview> {
    let current = state.branch.clone()?;
    let remote_main = format!("origin/{BRANCH_MAIN}");
    let target = || {
        action
            .release_env()
            .and_then(|env| state.release_branch(env))
            .map(str::to_string)
    };

    Some(match action {
        FlowAction::MergeMain => {
            let mut preview = Preview::new(format!(
                "{remote_main} lands in {current}, which is then pushed"
            ));
            let main = preview.lane(state, &remote_main);
            let branch = preview.lane(state, &current);
            let remote = preview.lane(state, &format!("origin/{current}"));
            preview.merge(main, branch);
            preview.merge(branch, remote);
            preview
        }
        FlowAction::ReleaseDev | FlowAction::ReleaseTest => {
            let target = target()?;
            let mut preview = Preview::new(format!(
                "{BRANCH_MAIN} and {current} merge into {target}, which is then pushed"
            ));
            let main = preview.lane(state, &remote_main);
            let feature = preview.lane(state, &format!("origin/{current}"));
            let deploy = preview.lane(state, &target);
            let remote = preview.lane(state, &format!("origin/{target}"));
            preview.merge(main, deploy);
            preview.merge(feature, deploy);
            preview.merge(deploy, remote);
            preview
        }
        FlowAction::ResetDev | FlowAction::ResetTest => {
            let target = target()?;
            let mut preview = Preview::new(format!(
                "{target} is thrown away and rebuilt from {remote_main}, then force-pushed"
            ));
            let main = preview.lane(state, &remote_main);
            let deploy = preview.lane(state, &target);
            let remote = preview.lane(state, &format!("origin/{target}"));
            preview.rebuild(main, deploy);
            preview.rebuild(deploy, remote);
            preview
        }
        FlowAction::DiscardCheckout => {
            let mut preview = Preview::new(format!(
                "everything in {current} that is not on its remote goes, untracked files included"
            ));
            let remote = preview.lane(state, &format!("origin/{current}"));
            let branch = preview.lane(state, &current);
            preview.rebuild(remote, branch);
            preview
        }
        FlowAction::NewFeature => {
            let mut preview = Preview::new(format!(
                "a new branch starts from {remote_main} and is pushed"
            ));
            let main = preview.lane(state, &remote_main);
            let branch = preview.lane(state, "new branch");
            let remote = preview.lane(state, "origin/new branch");
            preview.merge(main, branch);
            preview.merge(branch, remote);
            preview
        }
        FlowAction::TransferDiff => {
            let source = super::selected_feature_branch(state)?;
            let mut preview = Preview::new(format!(
                "what {source} changed against {BRANCH_MAIN} is staged on a new branch"
            ));
            let from = preview.lane(state, &source);
            let branch = preview.lane(state, "new branch");
            preview.merge(from, branch);
            preview
        }
        FlowAction::CleanOrphans => {
            let mut preview = Preview::new(
                "local branches with no remote are deleted; tracked branches are left alone",
            );
            preview.lane(state, "tracked");
            let orphan = preview.lane(state, "no upstream");
            preview.drop_lane(orphan);
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

/// Draw the grid, without the marker.
fn grid(preview: &Preview) -> Vec<Vec<Cell>> {
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
        let color = preview.lanes[mv.to].color;

        let Some(from) = mv.from else {
            // A lane that is only deleted: the whole of it goes, so the whole
            // of it is struck out rather than a stretch of it.
            for (lost, cell) in grid[to_row].iter_mut().enumerate() {
                *cell = Cell {
                    glyph: if lost % STRIDE == 0 { '\u{2717}' } else { ' ' },
                    color: Color::Red,
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
                    color: Color::Red,
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
                color: preview.lanes[from].color,
                dim: false,
            };
        }

        grid[to_row][col] = Cell {
            glyph: if mv.discards { '\u{25c6}' } else { '\u{25cd}' },
            color: if mv.discards { Color::Red } else { color },
            dim: false,
        };
    }

    grid
}

/// The diagram as lines, with the marker wherever `tick` has reached.
///
/// `width` is what the pane can show; a diagram that would not fit is dropped
/// rather than drawn cut in half, since half a graph says the wrong thing.
pub(super) fn lines(
    state: &AppState,
    action: FlowAction,
    tick: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let Some(preview) = preview(state, action) else {
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

    let cells = grid(&preview);
    let path = preview.path();
    let marker = path.get(tick % path.len().max(1)).copied();

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
            let lines = lines(&state, action, 0, 80);
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
        let drawn = text(&lines(&state, FlowAction::ReleaseTest, 0, 80));

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
        let reset = text(&lines(&state, FlowAction::ResetTest, 0, 80));
        let release = text(&lines(&state, FlowAction::ReleaseTest, 0, 80));

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
        let first = text(&lines(&state, FlowAction::MergeMain, 0, 80));
        let moved = (1..40)
            .map(|tick| text(&lines(&state, FlowAction::MergeMain, tick, 80)))
            .any(|frame| frame != first);
        assert!(moved, "the diagram should animate: {first}");
    }

    /// It walks the whole route and comes back, rather than running off the end.
    #[test]
    fn the_marker_visits_every_step_and_loops() {
        let state = state_with_deploy_branches();
        let preview = preview(&state, FlowAction::MergeMain).expect("preview");
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
        let lines = lines(&state, FlowAction::ReleaseTest, 0, 12);
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
        assert!(lines(&state, FlowAction::MergeMain, 0, 80).is_empty());
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
