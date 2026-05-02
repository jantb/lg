use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders},
};

use crate::{
    config::{BORDER_COLOR, LEFT_COLUMN_WIDTH, STATUS_BAR_HEIGHT},
    state::SPINNER_FRAMES,
};

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

/// A block with a LightBlue border and the given title.
pub fn bordered(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(title)
}

/// Center a `w × h` rectangle within `area`.
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
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
    let left_width = left_column_width(rows[0].width, requested_left_width);
    let cols =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Min(0)]).split(rows[0]);
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
        status,
        environments,
        files,
        branches,
        commits,
        main: cols[1],
        footer: rows[1],
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
        Constraint::Length(5),
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

/// Framed block for numbered panels.
/// `n` = panel number shown in title, `focused` controls border colour,
/// `count` = optional `(current, total)` shown bottom-right.
pub fn framed<'a>(
    n: u8,
    title: &'a str,
    focused: bool,
    count: Option<(usize, usize)>,
) -> Block<'a> {
    framed_with_activity(n, title, focused, count, 0, false)
}

pub fn framed_with_activity<'a>(
    n: u8,
    title: &'a str,
    focused: bool,
    count: Option<(usize, usize)>,
    tick: usize,
    active: bool,
) -> Block<'a> {
    let (border_color, title_style) = if focused {
        (
            if active {
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            },
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::Gray),
        )
    };

    let title_text = if focused {
        let pulse = if active {
            SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
        } else if tick % 2 == 0 {
            "\u{25cf}"
        } else {
            "\u{25cb}"
        };
        format!("{pulse} [{n}] {title}")
    } else {
        format!("[{n}] {title}")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_color)
        .title(Span::styled(title_text, title_style));

    if let Some((cur, total)) = count {
        let count_text = format!("{cur} of {total}");
        block.title_bottom(
            Line::from(Span::styled(
                count_text,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ))
            .alignment(Alignment::Right),
        )
    } else {
        block
    }
}

/// Colorize a single diff line into a styled `Line`.
pub fn highlight_diff_line(line: &str) -> Line<'_> {
    if matches!(line, "Message:" | "Files changed:" | "Patch:") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if line.starts_with("commit ") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if line.starts_with("Author:") || line.starts_with("Date:") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Gray)));
    }
    if line.starts_with("+++") || line.starts_with("---") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(rest) = line.strip_prefix('+') {
        return Line::from(vec![
            Span::styled(
                "+",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::LightGreen)),
        ]);
    }
    if let Some(rest) = line.strip_prefix('-') {
        return Line::from(vec![
            Span::styled(
                "-",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::LightRed)),
        ]);
    }
    if line.starts_with("@@") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Cyan)));
    }
    if line.starts_with("diff --git ") {
        return Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(Span::styled(line, Style::default()))
}
