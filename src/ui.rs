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
    highlight_diff_line_for_syntax(line, None)
}

pub fn highlight_diff_text(text: &str) -> Vec<Line<'_>> {
    let mut syntax = None;
    text.split('\n')
        .map(|line| {
            if let Some(next) = diff_line_syntax(line) {
                syntax = Some(next);
            }
            highlight_diff_line_for_syntax(line, syntax)
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Syntax {
    Kotlin,
    Rust,
}

fn highlight_diff_line_for_syntax(line: &str, syntax: Option<Syntax>) -> Line<'_> {
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
        let mut spans = vec![Span::styled(
            "+",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(highlight_code(
            rest,
            syntax,
            Style::default().fg(Color::LightGreen),
        ));
        return Line::from(spans);
    }
    if let Some(rest) = line.strip_prefix('-') {
        let mut spans = vec![Span::styled(
            "-",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )];
        spans.extend(highlight_code(
            rest,
            syntax,
            Style::default().fg(Color::LightRed),
        ));
        return Line::from(spans);
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
    Line::from(highlight_code(line, syntax, Style::default()))
}

fn diff_line_syntax(line: &str) -> Option<Syntax> {
    if let Some(path) = line.strip_prefix("+++ b/") {
        return path_syntax(path);
    }
    if let Some(path) = line.strip_prefix("diff --git ") {
        let path = path.split_whitespace().nth(1)?.strip_prefix("b/")?;
        return path_syntax(path);
    }
    None
}

fn path_syntax(path: &str) -> Option<Syntax> {
    if path.ends_with(".rs") {
        Some(Syntax::Rust)
    } else if path.ends_with(".kt") || path.ends_with(".kts") {
        Some(Syntax::Kotlin)
    } else {
        None
    }
}

fn highlight_code(code: &str, syntax: Option<Syntax>, default_style: Style) -> Vec<Span<'static>> {
    let Some(syntax) = syntax else {
        return vec![Span::styled(code.to_string(), default_style)];
    };

    let mut spans = Vec::new();
    let mut chars = code.char_indices().peekable();
    let mut plain_start = 0usize;
    while let Some((idx, ch)) = chars.next() {
        if ch == '/' && chars.peek().is_some_and(|(_, next)| *next == '/') {
            push_plain_code(&mut spans, code, plain_start, idx, default_style);
            spans.push(Span::styled(
                code[idx..].to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            return spans;
        }
        if ch == '"' {
            push_plain_code(&mut spans, code, plain_start, idx, default_style);
            let end = string_end(code, idx + ch.len_utf8());
            spans.push(Span::styled(
                code[idx..end].to_string(),
                Style::default().fg(Color::LightYellow),
            ));
            while chars.peek().is_some_and(|(next_idx, _)| *next_idx < end) {
                chars.next();
            }
            plain_start = end;
            continue;
        }
        if is_ident_start(ch) {
            let mut end = idx + ch.len_utf8();
            while let Some((next_idx, next)) = chars.peek().copied() {
                if !is_ident_continue(next) {
                    break;
                }
                chars.next();
                end = next_idx + next.len_utf8();
            }
            let ident = &code[idx..end];
            if let Some(style) = keyword_style(ident, syntax) {
                push_plain_code(&mut spans, code, plain_start, idx, default_style);
                spans.push(Span::styled(ident.to_string(), style));
                plain_start = end;
            }
        }
    }
    push_plain_code(&mut spans, code, plain_start, code.len(), default_style);
    spans
}

fn push_plain_code(
    spans: &mut Vec<Span<'static>>,
    code: &str,
    start: usize,
    end: usize,
    style: Style,
) {
    if start < end {
        spans.push(Span::styled(code[start..end].to_string(), style));
    }
}

fn string_end(code: &str, start: usize) -> usize {
    let mut escaped = false;
    for (idx, ch) in code[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return start + idx + ch.len_utf8();
        }
    }
    code.len()
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn keyword_style(word: &str, syntax: Syntax) -> Option<Style> {
    let keyword = match syntax {
        Syntax::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "use"
                | "where"
                | "while"
        ),
        Syntax::Kotlin => matches!(
            word,
            "as" | "class"
                | "data"
                | "else"
                | "false"
                | "fun"
                | "if"
                | "in"
                | "interface"
                | "is"
                | "null"
                | "object"
                | "override"
                | "private"
                | "return"
                | "suspend"
                | "true"
                | "val"
                | "var"
                | "when"
                | "while"
        ),
    };
    keyword.then_some(
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )
}

pub fn highlight_log_line(line: &str) -> Line<'_> {
    let (graph, content) = split_log_graph_prefix(line);
    let mut spans = log_graph_spans(graph);
    spans.extend(log_content_spans(content));
    Line::from(spans)
}

fn split_log_graph_prefix(line: &str) -> (&str, &str) {
    let split_at = line
        .char_indices()
        .find_map(|(idx, ch)| (!is_log_graph_char(ch)).then_some(idx))
        .unwrap_or(line.len());
    line.split_at(split_at)
}

fn is_log_graph_char(ch: char) -> bool {
    matches!(ch, ' ' | '*' | '|' | '/' | '\\' | '_' | '-')
}

fn log_graph_spans(graph: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (col, ch) in graph.chars().enumerate() {
        let color = if ch == '*' {
            Color::Yellow
        } else if ch == ' ' {
            Color::DarkGray
        } else {
            log_graph_color(col)
        };
        let style = if ch == '*' {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans
}

fn log_content_spans(content: &str) -> Vec<Span<'static>> {
    if let Some(rest) = content.strip_prefix("commit ") {
        let mut spans = vec![Span::styled(
            "commit ".to_string(),
            Style::default().fg(Color::DarkGray),
        )];
        if let Some((sha, decoration)) = rest.split_once(' ') {
            spans.push(Span::styled(
                sha.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                decoration.to_string(),
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        return spans;
    }

    if let Some(rest) = content.strip_prefix("Merge:") {
        return vec![
            Span::styled(
                "Merge:".to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::LightBlue)),
        ];
    }
    if let Some(rest) = content.strip_prefix("Author:") {
        return vec![
            Span::styled(
                "Author:".to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::Gray)),
        ];
    }
    if let Some(rest) = content.strip_prefix("Date:") {
        return vec![
            Span::styled(
                "Date:".to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_string(), Style::default().fg(Color::Gray)),
        ];
    }
    vec![Span::styled(content.to_string(), Style::default())]
}

fn log_graph_color(col: usize) -> Color {
    const COLORS: &[Color] = &[
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::LightGreen,
        Color::Yellow,
        Color::Cyan,
        Color::Magenta,
        Color::Green,
    ];
    COLORS[col % COLORS.len()]
}
