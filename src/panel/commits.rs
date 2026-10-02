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
    state::{AppState, Pane, PendingAction, clamp_index},
    ui,
};

/// The rows on show, where the selection is among them, and how many there
/// are: every commit, or the ones the `/` filter leaves.
fn shown_rows(state: &AppState) -> (Option<Vec<usize>>, Option<usize>, usize) {
    let selected = selected_commit_index(state);
    match state.filtered_rows(Pane::Commits) {
        Some(rows) => {
            let at = selected.and_then(|idx| rows.iter().position(|row| *row == idx));
            let len = rows.len();
            (Some(rows), at, len)
        }
        None => (None, selected, state.commits.len()),
    }
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let selected_idx = selected_commit_index(state);
    let (rows, selected_row, len) = shown_rows(state);
    let count = selected_row.map(|row| (row + 1, len));
    let title = state
        .commits_ref
        .as_deref()
        .map(|branch| format!("Commits: {branch}"))
        .unwrap_or_else(|| "Commits".to_string());
    let title = format!("{title}{}", state.filter_title(Pane::Commits));
    let block = ui::framed_with_activity(
        4,
        &title,
        focused,
        count,
        state.animation_ms,
        state.activity_label().is_some(),
    );

    let pipe_sets = state.commit_pipe_sets();
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

    // Only the rows on screen are turned into items. A filtered list leaves
    // gaps in the history, so its rows go without the graph lanes, which
    // would join commits that are not next to each other.
    let offset = visible_scroll_offset(state, area);
    let window = scroll::visible_window(len, offset, area.height);
    let filtered = rows.is_some();
    let items: Vec<ListItem> = window
        .clone()
        .filter_map(|row| {
            rows.as_ref()
                .map_or(Some(row), |rows| rows.get(row).copied())
        })
        .filter_map(|idx| state.commits.get(idx).map(|c| (c, idx)))
        .map(|(c, idx)| {
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
            if !filtered {
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
            }
            spans.push(Span::styled(
                c.subject.clone(),
                selected_style(subject_style, is_selected_row),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).block(block);

    let mut list_state =
        scroll::window_list_state(focused.then_some(selected_row).flatten(), &window);

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    state.commits_list.scroll = visible_scroll_offset(state, area);
}

fn visible_scroll_offset(state: &AppState, area: Rect) -> usize {
    let (_, selected_row, len) = shown_rows(state);
    scroll::selection_scroll_offset(
        selected_row,
        len,
        scroll::list_viewport_height(area.height),
        state.commits_list.scroll,
    )
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    state.commits_list.idx = selected_commit_index(state).unwrap_or(0);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down if state.step_filtered(Pane::Commits, true, 1) => {}
        KeyCode::Char('k') | KeyCode::Up if state.step_filtered(Pane::Commits, false, 1) => {}
        KeyCode::Char('j') | KeyCode::Down => {
            state.commits_list.idx = next_selectable_commit(state, state.commits_list.idx)
                .unwrap_or(
                    state
                        .commits_list
                        .idx
                        .min(state.commits.len().saturating_sub(1)),
                );
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.commits_list.idx = prev_selectable_commit(state, state.commits_list.idx)
                .unwrap_or(state.commits_list.idx);
        }
        KeyCode::Enter => {
            state.focus = Pane::Main;
        }
        KeyCode::Char('y') => {
            if let Some(commit) = selected_commit(state) {
                // The list holds abbreviated names; the clipboard gets the
                // whole one, which still names the commit once the short form
                // stops being unique.
                let sha = crate::git::full_sha(&commit.sha).unwrap_or(commit.sha);
                copy_sha(state, &sha);
            }
        }
        KeyCode::Char('t') => {
            if let Some(commit) = selected_commit(state) {
                confirm_revert(state, &commit);
            }
        }
        KeyCode::Char('A') => state.open_amend_modal(),
        _ => return Ok(false),
    }
    Ok(true)
}

fn selected_commit(state: &AppState) -> Option<crate::git::Commit> {
    selected_commit_index(state).and_then(|idx| state.commits.get(idx).cloned())
}

/// Put a commit's name on the clipboard; the status names it short.
pub(crate) fn copy_sha(state: &mut AppState, sha: &str) {
    let short = sha.get(..7).unwrap_or(sha).to_string();
    state.pending_action = Some(PendingAction::CopyToClipboard {
        label: short,
        text: sha.to_string(),
    });
}

/// Ask before adding a commit that undoes `commit`. A merge is refused here:
/// undoing one needs a side chosen, and lg does not guess.
fn confirm_revert(state: &mut AppState, commit: &crate::git::Commit) {
    if commit.parent_count() > 1 {
        state.set_status(
            format!(
                "{} is a merge commit; reverting a merge is not offered",
                commit.sha
            ),
            false,
        );
        return;
    }
    let onto = state.branch.clone().unwrap_or_else(|| "HEAD".to_string());
    state.confirm_reversible_action(
        "Revert commit",
        format!("Revert {} on {onto}?", commit.sha),
        format!(
            "{}\nAdds a new commit that undoes it. A conflict opens the conflict editor; a aborts the revert there.",
            commit.subject
        ),
        PendingAction::RevertCommit {
            sha: commit.sha.clone(),
        },
    );
}

pub(crate) fn selected_commit_index(state: &AppState) -> Option<usize> {
    if state.selection_hidden(Pane::Commits) {
        return None;
    }
    let idx = clamp_index(state.commits_list.idx, state.commits.len())?;
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
        crate::ui::palette::selection().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn selected_style(style: Style, selected: bool) -> Style {
    if selected {
        style.patch(crate::ui::palette::selection())
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
