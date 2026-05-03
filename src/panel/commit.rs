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

#[derive(Debug, Clone)]
struct VisualLine {
    text: String,
    start: usize,
    len: usize,
}

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

    let body_area = editor_body_area(area);
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

fn editor_body_area(area: Rect) -> Rect {
    let w = (area.width * 8 / 10).clamp(60, 120).min(area.width);
    let h = (area.height * 3 / 4).clamp(14, 34).min(area.height);
    let modal = ui::centered(area, w, h);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(modal);
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(chunks[0]);
    let input_area = left_chunks[0];
    Rect {
        x: input_area.x.saturating_add(1),
        y: input_area.y.saturating_add(1),
        width: input_area.width.saturating_sub(2),
        height: input_area.height.saturating_sub(2),
    }
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
    let mut cursor_row = 0;
    let mut cursor_col = 0;
    let lines = visual_lines(message, width);

    let mut matched = false;
    for (idx, line) in lines.iter().enumerate() {
        let end = line.start + line.len;
        if cursor >= line.start && cursor <= end {
            cursor_row = idx;
            cursor_col = cursor.saturating_sub(line.start);
            matched = true;
            if cursor < end || line.len == 0 || idx + 1 == lines.len() {
                break;
            }
        }
    }
    if !matched
        && let Some((idx, line)) = lines
            .iter()
            .enumerate()
            .take_while(|(_, line)| line.start <= cursor)
            .last()
    {
        cursor_row = idx;
        cursor_col = line.len;
    }

    let scroll = cursor_row.saturating_sub(height.saturating_sub(1));
    let visible = lines
        .iter()
        .skip(scroll)
        .take(height)
        .map(|line| line.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let cursor_y = (cursor_row - scroll).min(height - 1) as u16;
    let cursor_x = cursor_col.min(width.saturating_sub(1)) as u16;

    (visible, (cursor_x, cursor_y))
}

fn visual_lines(message: &str, width: usize) -> Vec<VisualLine> {
    let width = width.max(1);
    let logical_lines: Vec<&str> = message.split('\n').collect();
    let mut lines = Vec::new();
    let mut consumed = 0usize;

    for (line_idx, logical_line) in logical_lines.iter().enumerate() {
        let chars: Vec<char> = logical_line.chars().collect();
        if chars.is_empty() {
            lines.push(VisualLine {
                text: String::new(),
                start: consumed,
                len: 0,
            });
        } else {
            push_wrapped_visual_lines(&mut lines, &chars, consumed, width);
        }
        consumed += chars.len();
        if line_idx + 1 < logical_lines.len() {
            consumed += 1;
        }
    }

    if lines.is_empty() {
        lines.push(VisualLine {
            text: String::new(),
            start: 0,
            len: 0,
        });
    }

    lines
}

fn push_wrapped_visual_lines(
    lines: &mut Vec<VisualLine>,
    chars: &[char],
    line_start: usize,
    width: usize,
) {
    let mut offset = 0usize;
    while offset < chars.len() {
        let remaining = chars.len() - offset;
        if remaining <= width {
            lines.push(VisualLine {
                text: chars[offset..].iter().collect(),
                start: line_start + offset,
                len: remaining,
            });
            break;
        }

        let limit = offset + width;
        let wrap_at = (offset + 1..limit)
            .rev()
            .find(|idx| chars[*idx].is_whitespace());

        let (end, next) = match wrap_at {
            Some(idx) if idx > offset => {
                let mut next = idx + 1;
                while next < chars.len() && chars[next].is_whitespace() {
                    next += 1;
                }
                (idx, next)
            }
            _ => (limit, limit),
        };

        lines.push(VisualLine {
            text: chars[offset..end].iter().collect(),
            start: line_start + offset,
            len: end - offset,
        });
        offset = next;
    }
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

fn logical_lines(message: &str) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for line in message.split('\n') {
        let len = line.chars().count();
        lines.push((start, start + len));
        start += len + 1;
    }
    if lines.is_empty() {
        lines.push((0, 0));
    }
    lines
}

fn move_cursor_vertical(state: &mut AppState, up: bool) {
    let len = char_len(&state.commit_message);
    state.commit_cursor = state.commit_cursor.min(len);
    let lines = logical_lines(&state.commit_message);
    let current = lines
        .iter()
        .enumerate()
        .find_map(|(idx, (start, end))| {
            (state.commit_cursor >= *start && state.commit_cursor <= *end).then_some(idx)
        })
        .unwrap_or(lines.len().saturating_sub(1));

    let target = if up {
        current.saturating_sub(1)
    } else {
        current.saturating_add(1).min(lines.len().saturating_sub(1))
    };
    let col = state.commit_cursor.saturating_sub(lines[current].0);
    let (target_start, target_end) = lines[target];
    state.commit_cursor = target_start + col.min(target_end.saturating_sub(target_start));
}

pub fn place_cursor_at(state: &mut AppState, area: Rect, column: u16, row: u16) -> bool {
    if state.generation.is_some() {
        return false;
    }
    let body = editor_body_area(area);
    if column < body.x
        || column >= body.x.saturating_add(body.width)
        || row < body.y
        || row >= body.y.saturating_add(body.height)
    {
        return false;
    }

    let width = body.width.max(1) as usize;
    let lines = visual_lines(&state.commit_message, width);
    let visual_row = row.saturating_sub(body.y) as usize;
    let visual_col = column.saturating_sub(body.x) as usize;
    let line = &lines[visual_row.min(lines.len().saturating_sub(1))];
    state.commit_cursor =
        (line.start + visual_col.min(line.len)).min(char_len(&state.commit_message));
    true
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
        KeyCode::Char('a') if ctrl => {
            if !generating {
                state.commit_cursor = 0;
            }
        }
        KeyCode::Char('e') if ctrl => {
            if !generating {
                state.commit_cursor = char_len(&state.commit_message);
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
        KeyCode::Up => {
            if !generating {
                move_cursor_vertical(state, true);
            }
        }
        KeyCode::Down => {
            if !generating {
                move_cursor_vertical(state, false);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_lines_wrap_at_words_when_possible() {
        let lines = visual_lines("complete the saga immediately", 24);

        assert_eq!(lines[0].text, "complete the saga");
        assert_eq!(lines[1].text, "immediately");
    }

    #[test]
    fn visual_lines_hard_wrap_oversized_words() {
        let lines = visual_lines("immediately", 5);

        assert_eq!(lines[0].text, "immed");
        assert_eq!(lines[1].text, "iatel");
        assert_eq!(lines[2].text, "y");
    }
}
