use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::config::{LEFT_COLUMN_WIDTH, STATUS_BAR_HEIGHT};

pub const LEFT_PANEL_COUNT: usize = 5;
pub type LeftPanelHeights = [u16; LEFT_PANEL_COUNT];

/// Split area into header (1 line), body, and status bar.
pub fn split_main(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(STATUS_BAR_HEIGHT),
        ])
        .split(area);
    (chunks[0], chunks[1], chunks[2])
}

/// Center a `w x h` rectangle within `area`.
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect {
        x,
        y,
        width: w.min(area.width),
        height: h.min(area.height),
    }
}

pub struct LayoutRects {
    pub header: Rect,
    pub status: Rect,
    pub environments: Rect,
    pub files: Rect,
    pub branches: Rect,
    pub commits: Rect,
    pub main: Rect,
    pub footer: Rect,
}

pub fn split_layout(area: Rect) -> LayoutRects {
    split_layout_with_environments(area, true)
}

pub fn split_layout_with_environments(area: Rect, show_environments: bool) -> LayoutRects {
    split_layout_with_width(area, show_environments, None)
}

pub fn split_layout_with_width(
    area: Rect,
    show_environments: bool,
    requested_left_width: Option<u16>,
) -> LayoutRects {
    split_layout_with_sizes(area, show_environments, requested_left_width, None)
}

pub fn split_layout_with_sizes(
    area: Rect,
    show_environments: bool,
    requested_left_width: Option<u16>,
    requested_left_panel_heights: Option<LeftPanelHeights>,
) -> LayoutRects {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);
    let left_width = left_column_width(rows[1].width, requested_left_width);
    let cols =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Min(0)]).split(rows[1]);
    let left_heights = normalize_left_panel_heights(
        cols[0].height,
        show_environments,
        requested_left_panel_heights,
    );

    let mut y = cols[0].y;
    let status = Rect {
        height: left_heights[0],
        ..cols[0]
    };
    y = y.saturating_add(status.height);
    let environments = Rect {
        y,
        height: left_heights[1],
        ..cols[0]
    };
    y = y.saturating_add(environments.height);
    let files = Rect {
        y,
        height: left_heights[2],
        ..cols[0]
    };
    y = y.saturating_add(files.height);
    let branches = Rect {
        y,
        height: left_heights[3],
        ..cols[0]
    };
    y = y.saturating_add(branches.height);
    let commits = Rect {
        y,
        height: left_heights[4],
        ..cols[0]
    };

    LayoutRects {
        header: rows[0],
        status,
        environments,
        files,
        branches,
        commits,
        main: cols[1],
        footer: rows[2],
    }
}

pub fn clamp_left_column_width(total_width: u16, requested_width: u16) -> u16 {
    let min_main_width = 40.min(total_width / 2);
    requested_width
        .min(total_width.saturating_sub(min_main_width))
        .max(24.min(total_width))
}

fn left_column_width(total_width: u16, requested_width: Option<u16>) -> u16 {
    clamp_left_column_width(total_width, requested_width.unwrap_or(LEFT_COLUMN_WIDTH))
}

pub fn default_left_panel_heights(total_height: u16, show_environments: bool) -> LeftPanelHeights {
    let area = Rect {
        x: 0,
        y: 0,
        width: 1,
        height: total_height,
    };
    let lefts = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(13),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(area);
    if show_environments {
        [
            lefts[0].height,
            lefts[1].height,
            lefts[2].height,
            lefts[3].height,
            lefts[4].height,
        ]
    } else {
        [
            lefts[0].height,
            0,
            lefts[1].height.saturating_add(lefts[2].height),
            lefts[3].height,
            lefts[4].height,
        ]
    }
}

pub fn normalize_left_panel_heights(
    total_height: u16,
    show_environments: bool,
    requested: Option<LeftPanelHeights>,
) -> LeftPanelHeights {
    let mut heights =
        requested.unwrap_or_else(|| default_left_panel_heights(total_height, show_environments));
    if !show_environments {
        heights[2] = heights[1].saturating_add(heights[2]);
        heights[1] = 0;
    }

    let visible = if show_environments {
        &[0usize, 1, 2, 3, 4][..]
    } else {
        &[0usize, 2, 3, 4][..]
    };
    let min_height = left_panel_min_height(total_height, show_environments);
    for idx in visible {
        heights[*idx] = heights[*idx].max(min_height);
    }
    if !show_environments {
        heights[1] = 0;
    }

    let mut sum = visible
        .iter()
        .fold(0u16, |sum, idx| sum.saturating_add(heights[*idx]));
    while sum > total_height {
        let mut changed = false;
        for idx in visible.iter().rev() {
            if sum <= total_height {
                break;
            }
            if heights[*idx] > min_height {
                heights[*idx] -= 1;
                sum -= 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    while sum < total_height {
        heights[2] = heights[2].saturating_add(1);
        sum += 1;
    }

    heights
}

pub fn left_panel_min_height(total_height: u16, show_environments: bool) -> u16 {
    let visible_count = if show_environments { 5 } else { 4 };
    if total_height as usize >= visible_count * 3 {
        3
    } else if total_height as usize >= visible_count {
        1
    } else {
        0
    }
}
