use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, Paragraph},
};
use std::collections::HashSet;

use crate::{
    git::FileEntry,
    state::{AppState, Modal, PendingAction, SPINNER_FRAMES, TreeKind, build_tree_rows},
    ui,
};

use super::files::code_span;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = (area.width * 8 / 10).clamp(60, 120).min(area.width);
    let h = (area.height * 3 / 4).clamp(14, 34).min(area.height);
    let modal = ui::centered(area, w, h);

    frame.render_widget(Clear, modal);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(modal);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(chunks[0]);

    let (msg_view, msg_cursor, title_text, editable) = match &state.generation {
        Some(g) => {
            let spinner = SPINNER_FRAMES[g.spinner % SPINNER_FRAMES.len()];
            let title = format!("Commit message  {spinner} generating\u{2026}  (Esc=cancel)");
            (g.output.clone(), g.output.chars().count(), title, false)
        }
        None => (
            state.commit_message.clone(),
            state.commit_cursor,
            "Commit message  (Ctrl+S=commit  Shift+P=commit&push  Enter=newline  Ctrl+R=regenerate  Esc=back)"
                .to_owned(),
            true,
        ),
    };

    let input_area = left_chunks[0];
    let body_area = Rect {
        x: input_area.x.saturating_add(1),
        y: input_area.y.saturating_add(1),
        width: input_area.width.saturating_sub(2),
        height: input_area.height.saturating_sub(2),
    };
    let (visible_text, cursor) =
        visible_message_view(&msg_view, msg_cursor, body_area.width, body_area.height);

    let input = Paragraph::new(visible_text).block(ui::bordered(&title_text));
    frame.render_widget(input, left_chunks[0]);
    if editable && body_area.width > 0 && body_area.height > 0 {
        frame.set_cursor_position(Position::new(
            body_area.x.saturating_add(cursor.0),
            body_area.y.saturating_add(cursor.1),
        ));
    }

    let generating = state.generation.is_some();
    let hints = if generating {
        Paragraph::new(Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel generation"),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("Ctrl+S", Style::default().fg(Color::Green)),
            Span::raw(" commit  "),
            Span::styled("Shift+P", Style::default().fg(Color::Green)),
            Span::raw(" commit&push  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" newline  "),
            Span::styled("Ctrl+R", Style::default().fg(Color::Yellow)),
            Span::raw(" regenerate  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" back"),
        ]))
    };
    frame.render_widget(hints, left_chunks[1]);

    let staged_entries: Vec<FileEntry> = state
        .files
        .iter()
        .filter(|e| e.x != ' ' && e.x != '?')
        .cloned()
        .collect();
    let rows = build_tree_rows(&staged_entries, &HashSet::new());
    let items: Vec<ListItem> = rows
        .iter()
        .skip(1) // drop synthetic AllChanges header
        .map(|row| {
            let indent = "  ".repeat(row.depth as usize);
            let line = match &row.kind {
                TreeKind::Folder { .. } => Line::from(vec![
                    Span::raw(indent),
                    Span::styled("\u{25be}", Style::default().fg(Color::Cyan)),
                    Span::raw(" "),
                    Span::styled(
                        format!("{}/", row.label),
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                TreeKind::File { entry_idx } => {
                    let e = &staged_entries[*entry_idx];
                    Line::from(vec![
                        Span::raw(indent),
                        code_span(e.x),
                        Span::raw(" "),
                        Span::raw(row.label.clone()),
                    ])
                }
                TreeKind::AllChanges => unreachable!(),
            };
            ListItem::new(line)
        })
        .collect();

    let sidebar = List::new(items).block(ui::bordered("Staged"));
    frame.render_widget(sidebar, chunks[1]);
}

fn visible_message_view(
    message: &str,
    cursor: usize,
    width: u16,
    height: u16,
) -> (String, (u16, u16)) {
    if width == 0 || height == 0 {
        return (String::new(), (0, 0));
    }

    let width = width as usize;
    let height = height as usize;
    let cursor = cursor.min(message.chars().count());
    let mut lines = Vec::new();
    let mut cursor_row = 0;
    let mut cursor_col = 0;
    let mut consumed = 0usize;
    let logical_lines: Vec<&str> = message.split('\n').collect();

    for (line_idx, logical_line) in logical_lines.iter().enumerate() {
        let chars: Vec<char> = logical_line.chars().collect();
        let line_len = chars.len();
        let line_start_row = lines.len();
        let cursor_in_line = cursor >= consumed && cursor <= consumed + line_len;

        if chars.is_empty() {
            lines.push(String::new());
        } else {
            for chunk in chars.chunks(width) {
                lines.push(chunk.iter().collect::<String>());
            }
        }

        if cursor_in_line {
            let offset = cursor - consumed;
            cursor_row = line_start_row + offset / width;
            cursor_col = offset % width;
            if cursor_row >= lines.len() {
                lines.push(String::new());
            }
        }

        consumed += line_len;
        if line_idx + 1 < logical_lines.len() {
            consumed += 1;
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    let scroll = cursor_row.saturating_sub(height.saturating_sub(1));
    let visible = lines
        .iter()
        .skip(scroll)
        .take(height)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let cursor_y = (cursor_row - scroll).min(height - 1) as u16;
    let cursor_x = cursor_col.min(width.saturating_sub(1)) as u16;

    (visible, (cursor_x, cursor_y))
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn byte_index_for_char(s: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    s.char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(s.len())
}

fn insert_at_cursor(state: &mut AppState, c: char) {
    state.commit_cursor = state.commit_cursor.min(char_len(&state.commit_message));
    let idx = byte_index_for_char(&state.commit_message, state.commit_cursor);
    state.commit_message.insert(idx, c);
    state.commit_cursor += 1;
}

fn remove_before_cursor(state: &mut AppState) {
    state.commit_cursor = state.commit_cursor.min(char_len(&state.commit_message));
    if state.commit_cursor == 0 {
        return;
    }
    let start = byte_index_for_char(&state.commit_message, state.commit_cursor - 1);
    let end = byte_index_for_char(&state.commit_message, state.commit_cursor);
    state.commit_message.replace_range(start..end, "");
    state.commit_cursor -= 1;
}

fn remove_at_cursor(state: &mut AppState) {
    state.commit_cursor = state.commit_cursor.min(char_len(&state.commit_message));
    if state.commit_cursor >= char_len(&state.commit_message) {
        return;
    }
    let start = byte_index_for_char(&state.commit_message, state.commit_cursor);
    let end = byte_index_for_char(&state.commit_message, state.commit_cursor + 1);
    state.commit_message.replace_range(start..end, "");
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let generating = state.generation.is_some();
    match key.code {
        KeyCode::Esc => {
            if generating {
                state.cancel_generation();
                state.set_status("generation cancelled", false);
            } else {
                state.modal = Modal::None;
            }
        }
        KeyCode::Char('s') if ctrl => {
            if !generating && !state.commit_message.trim().is_empty() {
                state.push_after_commit = false;
                state.pending_action = Some(PendingAction::Commit);
            }
        }
        KeyCode::Char('P') if !ctrl => {
            if !generating && !state.commit_message.trim().is_empty() {
                state.push_after_commit = true;
                state.pending_action = Some(PendingAction::Commit);
            }
        }
        KeyCode::Char('r') if ctrl => {
            if !generating {
                state.commit_message.clear();
                state.commit_cursor = 0;
                state.set_status("generating\u{2026}", false);
                state.pending_action = Some(PendingAction::GenerateMessage);
            }
        }
        KeyCode::Enter => {
            if !generating {
                insert_at_cursor(state, '\n');
            }
        }
        KeyCode::Backspace => {
            if !generating {
                remove_before_cursor(state);
            }
        }
        KeyCode::Delete => {
            if !generating {
                remove_at_cursor(state);
            }
        }
        KeyCode::Left => {
            if !generating {
                state.commit_cursor = state.commit_cursor.saturating_sub(1);
            }
        }
        KeyCode::Right => {
            if !generating {
                state.commit_cursor = state
                    .commit_cursor
                    .saturating_add(1)
                    .min(char_len(&state.commit_message));
            }
        }
        KeyCode::Home => {
            if !generating {
                state.commit_cursor = 0;
            }
        }
        KeyCode::End => {
            if !generating {
                state.commit_cursor = char_len(&state.commit_message);
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if !generating {
                insert_at_cursor(state, c);
            }
        }
        _ => {}
    }
    Ok(())
}
