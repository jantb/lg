use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};

use crate::{
    state::{AppState, Modal, PendingAction, TreeKind, clamp_index},
    ui,
};

use super::scroll;

/// Map a porcelain status char to a colored Span.
pub(crate) fn code_span(c: char) -> Span<'static> {
    let style = match c {
        'M' => Style::default().fg(Color::Yellow),
        'A' => Style::default().fg(Color::Green),
        'D' => Style::default().fg(Color::Red),
        'R' | 'C' => Style::default().fg(Color::Magenta),
        'U' => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        '?' => Style::default().fg(Color::Cyan),
        ' ' => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::Gray),
    };
    Span::styled(c.to_string(), style)
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let rows = state.tree_rows();
    let total = rows.len();
    let selected_idx = clamp_index(state.files_idx, total);
    let count = selected_idx.map(|idx| (idx + 1, total));
    let block = ui::framed_with_activity(
        2,
        "Files",
        focused,
        count,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let mut items: Vec<ListItem> = Vec::with_capacity(total);
    for row in &rows {
        let indent = "  ".repeat(row.depth as usize);
        let line = match &row.kind {
            TreeKind::AllChanges => Line::from(Span::styled(
                "(all changes)",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            TreeKind::Folder {
                expanded,
                total,
                staged,
            } => {
                let chev = if *expanded { "\u{25be}" } else { "\u{25b8}" };
                let stats = format!(" [{staged}/{total}]");
                Line::from(vec![
                    Span::raw(indent),
                    Span::styled(chev, Style::default().fg(Color::Cyan)),
                    Span::raw(" "),
                    Span::styled(
                        format!("{}/", row.label),
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(stats, Style::default().fg(Color::DarkGray)),
                ])
            }
            TreeKind::File { entry_idx } => {
                let e = &state.files[*entry_idx];
                let fully_staged = e.x != ' ' && e.x != '?' && e.y == ' ';
                if e.x == '?' && e.y == '?' {
                    let marker = Style::default().fg(Color::DarkGray);
                    let label = Style::default().fg(Color::Gray);
                    Line::from(vec![
                        Span::raw(indent),
                        Span::styled("unstaged ", marker),
                        Span::styled(row.label.clone(), label),
                    ])
                } else if fully_staged {
                    let green = Style::default().fg(Color::Green);
                    Line::from(vec![
                        Span::raw(indent),
                        Span::styled(e.x.to_string(), green),
                        Span::styled(e.y.to_string(), green),
                        Span::raw(" "),
                        Span::styled(row.label.clone(), green),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw(indent),
                        code_span(e.x),
                        code_span(e.y),
                        Span::raw(" "),
                        Span::raw(row.label.clone()),
                    ])
                }
            }
        };
        items.push(ListItem::new(line));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    let offset = visible_scroll_offset(state, area);
    let mut list_state = scroll::list_state(focused.then_some(selected_idx).flatten(), offset);

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    let rows = state.tree_rows();
    let selected_idx = clamp_index(state.files_idx, rows.len());
    state.files_scroll_offset = scroll::selection_scroll_offset(
        selected_idx,
        rows.len(),
        scroll::list_viewport_height(area.height),
        state.files_scroll_offset,
    );
}

fn visible_scroll_offset(state: &AppState, area: Rect) -> usize {
    let rows = state.tree_rows();
    let selected_idx = clamp_index(state.files_idx, rows.len());
    scroll::selection_scroll_offset(
        selected_idx,
        rows.len(),
        scroll::list_viewport_height(area.height),
        state.files_scroll_offset,
    )
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let rows = state.tree_rows();
    let total = rows.len();
    state.files_idx = clamp_index(state.files_idx, total).unwrap_or(0);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.files_idx = state
                .files_idx
                .saturating_add(1)
                .min(total.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.files_idx = state.files_idx.saturating_sub(1);
        }
        KeyCode::Char(' ') | KeyCode::Char('y') => {
            if let Some(row) = rows.get(state.files_idx) {
                match &row.kind {
                    TreeKind::AllChanges => {
                        state.pending_action = Some(PendingAction::StageAll);
                    }
                    TreeKind::Folder { .. } => {
                        let path = row.path.clone();
                        state.pending_action = Some(PendingAction::StagePath(path));
                    }
                    TreeKind::File { entry_idx } => {
                        let path = state.files[*entry_idx].path.clone();
                        state.pending_action = Some(PendingAction::StagePath(path));
                    }
                }
            }
        }
        KeyCode::Char('u') => {
            if let Some(row) = rows.get(state.files_idx) {
                match &row.kind {
                    TreeKind::AllChanges => {
                        state.pending_action = Some(PendingAction::UnstageAll);
                    }
                    TreeKind::Folder { .. } => {
                        let path = row.path.clone();
                        state.pending_action = Some(PendingAction::UnstagePath(path));
                    }
                    TreeKind::File { entry_idx } => {
                        let path = state.files[*entry_idx].path.clone();
                        state.pending_action = Some(PendingAction::UnstagePath(path));
                    }
                }
            }
        }
        KeyCode::Char('A') => {
            state.pending_action = Some(PendingAction::StageAll);
        }
        KeyCode::Char('U') => {
            state.pending_action = Some(PendingAction::UnstageAll);
        }
        KeyCode::Char('i') => {
            if let Some(row) = rows.get(state.files_idx) {
                match &row.kind {
                    TreeKind::Folder { .. } => {
                        state.pending_action = Some(PendingAction::IgnorePath {
                            path: row.path.clone(),
                            is_dir: true,
                        });
                    }
                    TreeKind::File { entry_idx } => {
                        state.pending_action = Some(PendingAction::IgnorePath {
                            path: state.files[*entry_idx].path.clone(),
                            is_dir: false,
                        });
                    }
                    TreeKind::AllChanges => {
                        state.set_status("select a file or folder to ignore", false);
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(row) = rows.get(state.files_idx) {
                match &row.kind {
                    TreeKind::Folder { .. } => {
                        state.pending_action = Some(PendingAction::DeletePath {
                            path: row.path.clone(),
                            is_dir: true,
                        });
                    }
                    TreeKind::File { entry_idx } => {
                        state.pending_action = Some(PendingAction::DeletePath {
                            path: state.files[*entry_idx].path.clone(),
                            is_dir: false,
                        });
                    }
                    TreeKind::AllChanges => {
                        state.set_status("select a file or folder to delete", false);
                    }
                }
            }
        }
        KeyCode::Char('o') => {
            if let Some(row) = rows.get(state.files_idx) {
                match row.kind {
                    TreeKind::AllChanges | TreeKind::Folder { .. } => {
                        state.pending_action = Some(PendingAction::OpenProject);
                    }
                    TreeKind::File { entry_idx } => {
                        let path = state.files[entry_idx].path.clone();
                        state.pending_action = Some(PendingAction::OpenFile(path));
                    }
                }
            }
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            if let Some(row) = rows.get(state.files_idx) {
                if let TreeKind::Folder { expanded, .. } = row.kind {
                    if expanded {
                        state.collapsed_dirs.insert(row.path.clone());
                    } else {
                        state.collapsed_dirs.remove(&row.path);
                    }
                }
            }
        }
        KeyCode::Char('h') | KeyCode::Left => {
            if let Some(row) = rows.get(state.files_idx) {
                if let TreeKind::Folder { expanded: true, .. } = row.kind {
                    state.collapsed_dirs.insert(row.path.clone());
                }
            }
        }
        KeyCode::Char('c') => {
            if state.modal == Modal::None {
                state.open_commit_or_stage_all_prompt();
            }
        }
        KeyCode::Char('p') => {
            if state.pull_available() {
                state.pending_action = Some(PendingAction::Pull);
            } else {
                state.set_status("nothing to pull", false);
            }
        }
        _ => {}
    }
    Ok(())
}
