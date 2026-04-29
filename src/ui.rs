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
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
    let cols = Layout::horizontal([Constraint::Length(LEFT_COLUMN_WIDTH), Constraint::Min(0)])
        .split(rows[0]);
    let lefts = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(cols[0]);
    LayoutRects {
        status: lefts[0],
        environments: lefts[1],
        files: lefts[2],
        branches: lefts[3],
        commits: lefts[4],
        main: cols[1],
        footer: rows[1],
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
    let style = if matches!(line, "Message:" | "Files changed:" | "Patch:") {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("commit ") {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("Author:") || line.starts_with("Date:") {
        Style::default().fg(Color::Gray)
    } else if line.starts_with("+++") || line.starts_with("---") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else if line.starts_with("diff --git ") {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::from(Span::styled(line, style))
}
