use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};

use super::scroll;

use crate::{
    graph::{self, Pipe, SELECTED_COLOR},
    state::{AppState, Pane, clamp_index},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let selected_idx = selected_commit_index(state);
    let count = selected_idx.map(|idx| (idx + 1, state.commits.len()));
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

    let pipe_sets = graph::pipe_sets(&state.commits);
    let hash_width = visible_hash_width(&state.commits);
    let max_pipe_width = pipe_sets
        .iter()
        .map(|set| pipe_set_display_width(set))
        .max()
        .unwrap_or(2);
    let graph_width = visible_graph_width(area.width, max_pipe_width, hash_width);

    let selected_sha = if focused {
        selected_idx.and_then(|idx| state.commits.get(idx).map(|c| c.sha.as_str()))
    } else {
        None
    };

    let items: Vec<ListItem> = state
        .commits
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let is_selected_row = focused && Some(idx) == selected_idx;
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
                Style::default().fg(author_color(&c.author))
            };
            let mut spans = vec![
                Span::styled(
                    format!("{:<hash_width$} ", c.sha),
                    hash_style(is_selected_row),
                ),
                Span::styled(
                    format!("{:<2} ", c.author_short),
                    selected_style(author_style, is_selected_row),
                ),
            ];
            let prev_sha = idx
                .checked_sub(1)
                .and_then(|i| state.commits.get(i))
                .map(|c| c.sha.as_str());
            spans.extend(
                graph_spans(
                    &pipe_sets[idx],
                    selected_sha,
                    prev_sha,
                    graph_width,
                    c.is_first_parent,
                )
                .into_iter()
                .map(|span| selected_span(span, is_selected_row)),
            );
            spans.push(Span::styled(
                c.subject.clone(),
                selected_style(subject_style, is_selected_row),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).block(block);

    let offset = visible_scroll_offset(state, area);
    let mut list_state = scroll::list_state(focused.then_some(selected_idx).flatten(), offset);

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    let selected_idx = selected_commit_index(state);
    state.commits_scroll_offset = scroll::selection_scroll_offset(
        selected_idx,
        state.commits.len(),
        scroll::list_viewport_height(area.height),
        state.commits_scroll_offset,
    );
}

fn visible_scroll_offset(state: &AppState, area: Rect) -> usize {
    let selected_idx = selected_commit_index(state);
    scroll::selection_scroll_offset(
        selected_idx,
        state.commits.len(),
        scroll::list_viewport_height(area.height),
        state.commits_scroll_offset,
    )
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    state.commits_idx = selected_commit_index(state).unwrap_or(0);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.commits_idx = next_selectable_commit(state, state.commits_idx)
                .unwrap_or(state.commits_idx.min(state.commits.len().saturating_sub(1)));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.commits_idx =
                prev_selectable_commit(state, state.commits_idx).unwrap_or(state.commits_idx);
        }
        KeyCode::Enter => {
            state.focus = Pane::Main;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn selected_commit_index(state: &AppState) -> Option<usize> {
    let idx = clamp_index(state.commits_idx, state.commits.len())?;
    if !state.commits[idx].is_graph_row() {
        return Some(idx);
    }
    state
        .commits
        .iter()
        .enumerate()
        .find_map(|(idx, commit)| (!commit.is_graph_row()).then_some(idx))
}

fn next_selectable_commit(state: &AppState, idx: usize) -> Option<usize> {
    state
        .commits
        .iter()
        .enumerate()
        .skip(idx.saturating_add(1))
        .find_map(|(candidate, commit)| (!commit.is_graph_row()).then_some(candidate))
}

fn prev_selectable_commit(state: &AppState, idx: usize) -> Option<usize> {
    state
        .commits
        .iter()
        .enumerate()
        .take(idx)
        .rev()
        .find_map(|(candidate, commit)| (!commit.is_graph_row()).then_some(candidate))
}

fn visible_hash_width(commits: &[crate::git::Commit]) -> usize {
    commits
        .iter()
        .map(|commit| commit.sha.chars().count())
        .max()
        .unwrap_or(8)
        .max(8)
}

fn pipe_set_display_width(pipes: &[Pipe]) -> usize {
    let max_pos = pipes
        .iter()
        .map(|p| p.from_pos.max(p.to_pos))
        .max()
        .unwrap_or(0);
    // Each lane uses 2 chars (symbol + connector), trailing connector trimmed.
    2 * (max_pos as usize + 1).max(1)
}

fn visible_graph_width(area_width: u16, max_graph_width: usize, hash_width: usize) -> usize {
    let content_width = area_width.saturating_sub(2) as usize;
    let fixed_columns = hash_width + 1 + 2 + 1;
    content_width
        .saturating_sub(fixed_columns + 12)
        .clamp(1, max_graph_width.clamp(1, 28))
}

fn graph_spans(
    pipes: &[Pipe],
    selected_sha: Option<&str>,
    prev_sha: Option<&str>,
    width: usize,
    bold: bool,
) -> Vec<Span<'static>> {
    let cells = graph::render_pipe_set(pipes, selected_sha, prev_sha);
    let mut spans = Vec::with_capacity(cells.len() * 2 + 1);
    let mut col = 0usize;
    for (idx, cell) in cells.iter().enumerate() {
        if col >= width {
            break;
        }
        spans.push(Span::styled(
            cell.symbol.to_string(),
            cell_style(cell.symbol_color, bold, cell.symbol_color == SELECTED_COLOR),
        ));
        col += 1;
        if idx + 1 < cells.len() && col < width {
            spans.push(Span::styled(
                cell.connector.to_string(),
                cell_style(
                    cell.connector_color,
                    bold,
                    cell.connector_color == SELECTED_COLOR,
                ),
            ));
            col += 1;
        }
    }
    spans.push(Span::raw(" "));
    spans
}

fn cell_style(color: Color, bold: bool, force_bold: bool) -> Style {
    let mut s = Style::default().fg(color);
    if bold || force_bold {
        s = s.add_modifier(Modifier::BOLD);
    }
    s
}

fn selected_span(span: Span<'static>, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(span.content, selected_style(span.style, true))
    } else {
        span
    }
}

fn hash_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn selected_style(style: Style, selected: bool) -> Style {
    if selected {
        style.bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    } else {
        style
    }
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
