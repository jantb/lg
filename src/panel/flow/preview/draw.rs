//! The preview drawn: lanes and elbows laid on a grid of cells, and the
//! marker travelling the route.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::{AppState, FlowRun};
use crate::ui::palette;

use super::actions::{preview, shorten};
use super::{Preview, Progress};

/// One cell of the drawn diagram.
#[derive(Clone, Copy)]
pub(super) struct Cell {
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
pub(super) fn grid(preview: &Preview, progress: Progress) -> Vec<Vec<Cell>> {
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
pub(in crate::panel::flow) const MARKER_CELL_MS: u64 = 90;

/// How many cells of glow the marker leaves behind it at `MIN_STRIDE`; it grows
/// with the stride so the trail stays the same length in commits.
pub(super) const TRAIL_CELLS: usize = 7;

/// Cells at the front of the trail drawn as the marker itself rather than as a
/// lit stretch of track. This is a count and not a fraction of the trail: a
/// stretched diagram earns a longer tail, but a head that grew with it would
/// read as a bar sliding along the lane instead of something travelling it.
pub(super) const MARKER_HEAD_CELLS: f64 = 3.0;

/// Rows `lines` adds under the diagram: a blank one and the caption.
pub(in crate::panel::flow) const CAPTION_ROWS: usize = 2;

/// The diagram as lines, with the marker where `progress` and the animation
/// clock (`clock_ms`) put it.
///
/// `width` and `height` are what the pane can show; the diagram is stretched to
/// fill them, and one that would not fit even at its smallest is dropped rather
/// than drawn cut in half, since half a graph says the wrong thing.
pub(in crate::panel::flow) fn lines(
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
