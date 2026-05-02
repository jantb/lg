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
const MERGE_PAD: char = '\u{2007}';

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let count = if real_commit_count(&state.commits) == 0 {
        None
    } else {
        Some((
            selected_commit_ordinal(&state.commits, state.commits_idx),
            real_commit_count(&state.commits),
        ))
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

    let hash_width = visible_hash_width(&state.commits);
    let graph_width = visible_graph_width(
        area.width,
        state
            .commits
            .iter()
            .map(graph_display_width)
            .max()
            .unwrap_or(1),
        hash_width,
    );

    let items: Vec<ListItem> = state
        .commits
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let graph_row = c.is_graph_row();
            let selected = focused && idx == state.commits_idx && !graph_row;
            let subject_style = if state.unpushed_shas.contains(&c.sha) {
                Style::default().fg(Color::Red)
            } else if c.is_first_parent || graph_row {
                Style::default()
            } else {
                Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
            };
            let author_style = if c.is_first_parent {
                Style::default()
                    .fg(author_color(&c.author))
                    .add_modifier(Modifier::BOLD)
            } else if graph_row {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(author_color(&c.author))
            };
            let mut spans = vec![
                Span::styled(format!("{:<hash_width$} ", c.sha), hash_style(selected)),
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
    if focused && !state.commits.is_empty() && !state.commits[state.commits_idx].is_graph_row() {
        list_state.select(Some(state.commits_idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(next) = next_commit_idx(&state.commits, state.commits_idx) {
                state.commits_idx = next;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(prev) = previous_commit_idx(&state.commits, state.commits_idx) {
                state.commits_idx = prev;
            }
        }
        KeyCode::Enter => {
            state.focus = Pane::Main;
        }
        _ => {}
    }
    Ok(())
}

fn real_commit_count(commits: &[crate::git::Commit]) -> usize {
    commits
        .iter()
        .filter(|commit| !commit.is_graph_row())
        .count()
}

fn selected_commit_ordinal(commits: &[crate::git::Commit], idx: usize) -> usize {
    commits
        .iter()
        .take(idx.saturating_add(1))
        .filter(|commit| !commit.is_graph_row())
        .count()
        .max(1)
}

fn next_commit_idx(commits: &[crate::git::Commit], idx: usize) -> Option<usize> {
    commits
        .iter()
        .enumerate()
        .skip(idx.saturating_add(1))
        .find_map(|(idx, commit)| (!commit.is_graph_row()).then_some(idx))
}

fn previous_commit_idx(commits: &[crate::git::Commit], idx: usize) -> Option<usize> {
    commits
        .iter()
        .enumerate()
        .take(idx)
        .rev()
        .find_map(|(idx, commit)| (!commit.is_graph_row()).then_some(idx))
}

fn visible_hash_width(commits: &[crate::git::Commit]) -> usize {
    commits
        .iter()
        .filter(|commit| !commit.is_graph_row())
        .map(|commit| commit.sha.chars().count())
        .max()
        .unwrap_or(8)
        .max(8)
}

fn visible_graph_width(area_width: u16, max_graph_width: usize, hash_width: usize) -> usize {
    let content_width = area_width.saturating_sub(2) as usize;
    let fixed_columns = hash_width + 1 + 2 + 1;
    content_width
        .saturating_sub(fixed_columns + 12)
        .clamp(1, max_graph_width.clamp(1, 14))
}

fn graph_spans(commit: &crate::git::Commit, width: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let graph = if commit.graph.trim().is_empty() {
        "*"
    } else {
        commit.graph.as_str()
    };
    let (cells, merge_cols) = graph_cells(commit, graph);

    for (visible_col, ch) in cells.into_iter().enumerate() {
        if visible_col >= width {
            break;
        }
        let is_merge_connector = merge_cols.contains(&visible_col);
        let symbol = graph_symbol(ch);
        let color = if is_merge_connector {
            Color::Yellow
        } else if ch == '*' {
            commit_marker_color(commit)
        } else {
            graph_column_color(visible_col)
        };
        let style = if is_merge_connector || commit.is_first_parent || ch == '*' {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        spans.push(Span::styled(symbol.to_string(), style));
    }

    spans.push(Span::raw(" "));
    spans
}

fn graph_cells(commit: &crate::git::Commit, graph: &str) -> (Vec<char>, Vec<usize>) {
    let mut cells: Vec<char> = graph.chars().collect();
    let Some(star_idx) = cells.iter().position(|ch| *ch == '*') else {
        return (cells, Vec::new());
    };
    if commit.parent_count <= 1 {
        normalize_join_row(&mut cells, star_idx);
        return (cells, Vec::new());
    }

    let end_idx = merge_connector_end(&cells, star_idx);
    if cells.len() <= end_idx {
        cells.resize(end_idx + 1, ' ');
    }
    cells[star_idx] = MERGE_MARKER;
    for idx in star_idx + 1..end_idx {
        cells[idx] = if idx == star_idx + 1 { MERGE_PAD } else { '-' };
    }
    cells[end_idx] = '\u{256e}';
    (cells, (star_idx..=end_idx).collect())
}

fn normalize_join_row(cells: &mut [char], star_idx: usize) {
    let Some(last_join_idx) = cells
        .iter()
        .rposition(|ch| matches!(*ch, '/' | '\\' | '-' | '_'))
    else {
        return;
    };
    if last_join_idx <= star_idx {
        return;
    }

    for cell in &mut cells[star_idx + 1..last_join_idx] {
        if *cell == ' ' {
            *cell = '-';
        }
    }
}

fn merge_connector_end(cells: &[char], star_idx: usize) -> usize {
    let folded_target = cells
        .iter()
        .enumerate()
        .skip(star_idx + 1)
        .rev()
        .find_map(|(idx, ch)| (*ch != ' ').then_some(idx));
    folded_target.unwrap_or(star_idx + 3).max(star_idx + 3)
}

fn graph_display_width(commit: &crate::git::Commit) -> usize {
    let graph_width = if commit.graph.trim().is_empty() {
        1
    } else {
        commit.graph.chars().count()
    };
    if commit.parent_count > 1 {
        let merge_width = commit
            .graph
            .chars()
            .position(|ch| ch == '*')
            .map(|idx| {
                let cells: Vec<char> = commit.graph.chars().collect();
                merge_connector_end(&cells, idx) + 1
            })
            .unwrap_or(4);
        graph_width.max(merge_width)
    } else {
        graph_width
    }
}

fn graph_symbol(ch: char) -> char {
    match ch {
        '*' => '\u{25cb}',
        MERGE_MARKER => MERGE_MARKER,
        MERGE_PAD => ' ',
        '|' => '\u{2502}',
        '/' => '\u{256f}',
        '\\' => '\u{256e}',
        '\u{256e}' => '\u{256e}',
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
