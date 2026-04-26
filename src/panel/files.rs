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
    state::{AppState, Modal, TreeKind},
    ui,
};

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
    let count = if total == 0 {
        None
    } else {
        Some((state.files_idx + 1, total))
    };
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
                if fully_staged {
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

    let mut list_state = ListState::default();
    if focused {
        list_state.select(Some(state.files_idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let rows = state.tree_rows();
    let total = rows.len();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if state.files_idx + 1 < total {
                state.files_idx += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.files_idx = state.files_idx.saturating_sub(1);
        }
        KeyCode::Char(' ') | KeyCode::Char('y') => {
            if let Some(row) = rows.get(state.files_idx) {
                match &row.kind {
                    TreeKind::AllChanges => match crate::git::stage_all() {
                        Ok(()) => state.set_status("staged all", false),
                        Err(e) => state.set_status(e.to_string(), true),
                    },
                    TreeKind::Folder { .. } => {
                        let path = row.path.clone();
                        match crate::git::stage(&path) {
                            Ok(()) => state.set_status(format!("staged {path}/"), false),
                            Err(e) => state.set_status(e.to_string(), true),
                        }
                    }
                    TreeKind::File { entry_idx } => {
                        let path = state.files[*entry_idx].path.clone();
                        match crate::git::stage(&path) {
                            Ok(()) => state.set_status(format!("staged {path}"), false),
                            Err(e) => state.set_status(e.to_string(), true),
                        }
                    }
                }
            }
        }
        KeyCode::Char('u') => {
            if let Some(row) = rows.get(state.files_idx) {
                match &row.kind {
                    TreeKind::AllChanges => match crate::git::unstage_all() {
                        Ok(()) => state.set_status("unstaged all", false),
                        Err(e) => state.set_status(e.to_string(), true),
                    },
                    TreeKind::Folder { .. } => {
                        let path = row.path.clone();
                        match crate::git::unstage(&path) {
                            Ok(()) => state.set_status(format!("unstaged {path}/"), false),
                            Err(e) => state.set_status(e.to_string(), true),
                        }
                    }
                    TreeKind::File { entry_idx } => {
                        let path = state.files[*entry_idx].path.clone();
                        match crate::git::unstage(&path) {
                            Ok(()) => state.set_status(format!("unstaged {path}"), false),
                            Err(e) => state.set_status(e.to_string(), true),
                        }
                    }
                }
            }
        }
        KeyCode::Char('A') => match crate::git::stage_all() {
            Ok(()) => state.set_status("staged all", false),
            Err(e) => state.set_status(e.to_string(), true),
        },
        KeyCode::Char('U') => match crate::git::unstage_all() {
            Ok(()) => state.set_status("unstaged all", false),
            Err(e) => state.set_status(e.to_string(), true),
        },
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
                state.open_commit_modal();
            }
        }
        KeyCode::Char('p') => {
            state.modal = Modal::Push;
        }
        _ => {}
    }
    Ok(())
}
