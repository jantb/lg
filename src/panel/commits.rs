use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

use crate::{
    state::{AppState, Pane},
    ui,
};

const MERGE_MARKER: char = '\u{23e3}';
const MERGE_CONNECTOR: &str = "\u{2500}\u{256e}";

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let count = if state.commits.is_empty() {
        None
    } else {
        Some((state.commits_idx + 1, state.commits.len()))
    };
    let title = state
        .commits_ref
        .as_deref()
        .map(|branch| format!("Commits: {branch}"))
        .unwrap_or_else(|| "Commits".to_string());
    let block = ui::framed_with_activity(
        4,
        &title,
        focused,
        count,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let graph_width = visible_graph_width(
        area.width,
        state
            .commits
            .iter()
            .map(graph_display_width)
            .max()
            .unwrap_or(1),
    );

    let items: Vec<ListItem> = state
        .commits
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let selected = focused && idx == state.commits_idx;
            let subject_style = if state.unpushed_shas.contains(&c.sha) {
                Style::default().fg(Color::Red)
            } else if c.is_first_parent {
                Style::default()
            } else {
                Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
            };
            let author_style = if c.is_first_parent {
                Style::default()
                    .fg(author_color(&c.author))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let mut spans = vec![
                Span::styled(
                    format!("{} ", c.sha),
                    selected_style(Style::default().fg(Color::DarkGray), selected),
                ),
                Span::styled(
                    format!("{:<2} ", c.author_short),
                    selected_style(author_style, selected),
                ),
            ];
            spans.extend(
                graph_spans(c, graph_width)
                    .into_iter()
                    .map(|span| selected_span(span, selected)),
            );
            spans.extend([Span::styled(
                c.subject.clone(),
                selected_style(subject_style, selected),
            )]);
            let line = Line::from(spans);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(block);

    let mut list_state = ListState::default();
    if focused && !state.commits.is_empty() {
        list_state.select(Some(state.commits_idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !state.commits.is_empty() && state.commits_idx + 1 < state.commits.len() {
                state.commits_idx += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.commits_idx > 0 {
                state.commits_idx -= 1;
            }
        }
        KeyCode::Enter => {
            state.focus = Pane::Main;
        }
        _ => {}
    }
    Ok(())
}

fn visible_graph_width(area_width: u16, max_graph_width: usize) -> usize {
    let content_width = area_width.saturating_sub(2) as usize;
    let fixed_columns = 8 + 1 + 2 + 1;
    content_width
        .saturating_sub(fixed_columns + 12)
        .clamp(1, max_graph_width.clamp(1, 14))
}

fn graph_spans(commit: &crate::git::Commit, width: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut visible_col = 0usize;
    let mut skip_graph_padding = 0usize;
    let graph = if commit.graph.trim().is_empty() {
        "*"
    } else {
        commit.graph.as_str()
    };

    for ch in graph.chars() {
        if skip_graph_padding > 0 && ch == ' ' {
            skip_graph_padding -= 1;
            continue;
        }
        if visible_col >= width {
            break;
        }
        let is_merge_marker = ch == '*' && commit.parent_count > 1;
        let symbol = if is_merge_marker {
            MERGE_MARKER
        } else {
            graph_symbol(ch)
        };
        let color = if is_merge_marker {
            Color::Yellow
        } else if ch == '*' {
            commit_marker_color(commit)
        } else {
            graph_column_color(visible_col)
        };
        let style = if commit.is_first_parent || ch == '*' {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        spans.push(Span::styled(symbol.to_string(), style));
        visible_col += 1;

        if ch == '*' && commit.parent_count > 1 {
            let before_connector = visible_col;
            visible_col = push_merge_connector(&mut spans, visible_col, width);
            skip_graph_padding = visible_col.saturating_sub(before_connector);
        }
    }

    for _ in visible_col..width {
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(" "));
    spans
}

fn graph_display_width(commit: &crate::git::Commit) -> usize {
    let graph_width = if commit.graph.trim().is_empty() {
        1
    } else {
        commit.graph.chars().count()
    };
    if commit.parent_count > 1 {
        graph_width.max(1 + MERGE_CONNECTOR.chars().count())
    } else {
        graph_width
    }
}

fn push_merge_connector(
    spans: &mut Vec<Span<'static>>,
    mut visible_col: usize,
    width: usize,
) -> usize {
    for ch in MERGE_CONNECTOR.chars() {
        if visible_col >= width {
            break;
        }
        spans.push(Span::styled(ch.to_string(), merge_connector_style()));
        visible_col += 1;
    }
    visible_col
}

fn merge_connector_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn graph_symbol(ch: char) -> char {
    match ch {
        '*' => '\u{25cb}',
        '|' => '\u{2502}',
        '/' => '\u{2571}',
        '\\' => '\u{2572}',
        '-' | '_' => '\u{2500}',
        other => other,
    }
}

fn selected_span(span: Span<'static>, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(span.content, selected_style(span.style, true))
    } else {
        span
    }
}

fn selected_style(style: Style, selected: bool) -> Style {
    if selected {
        style.bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn commit_marker_color(commit: &crate::git::Commit) -> Color {
    if commit.is_first_parent {
        Color::LightGreen
    } else {
        Color::LightMagenta
    }
}

fn graph_column_color(col: usize) -> Color {
    const COLORS: &[Color] = &[
        Color::LightGreen,
        Color::LightMagenta,
        Color::LightCyan,
        Color::Yellow,
        Color::Cyan,
        Color::Magenta,
        Color::LightBlue,
        Color::LightYellow,
    ];
    COLORS[col % COLORS.len()]
}

fn author_color(author: &str) -> Color {
    const COLORS: &[Color] = &[
        Color::Cyan,
        Color::Yellow,
        Color::Green,
        Color::Magenta,
        Color::Blue,
        Color::LightCyan,
        Color::LightYellow,
        Color::LightGreen,
        Color::LightMagenta,
        Color::LightBlue,
    ];
    let hash = author.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    COLORS[hash as usize % COLORS.len()]
}
