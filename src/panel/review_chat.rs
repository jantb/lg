use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use crate::{
    panel::markdown,
    state::{AppState, Modal, PendingAction, ReviewChatRole, SPINNER_FRAMES},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = (area.width * 9 / 10).clamp(70, 150).min(area.width);
    let h = (area.height * 4 / 5).clamp(16, 42).min(area.height);
    let modal = ui::centered(area, w, h);

    frame.render_widget(Clear, modal);
    render_contents(state, modal, frame);
}

pub fn render_docked(state: &AppState, area: Rect, frame: &mut Frame) {
    render_contents(state, area, frame);
}

fn render_contents(state: &AppState, area: Rect, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    let running = state.review_chat_job.is_some();
    let title = if let Some(job) = &state.review_chat_job {
        let spinner = SPINNER_FRAMES[job.spinner % SPINNER_FRAMES.len()];
        format!("Review chat  {spinner} asking Ollama")
    } else {
        "Review chat".to_string()
    };
    let lines = conversation_lines(state, chunks[0].width.saturating_sub(2));
    let max_offset = lines
        .len()
        .saturating_sub(chunks[0].height.saturating_sub(2) as usize);
    let offset = state
        .review_chat_scroll
        .min(max_offset.min(u16::MAX as usize) as u16);
    let conversation = Paragraph::new(lines)
        .block(ui::bordered(&title))
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));
    frame.render_widget(conversation, chunks[0]);

    let input_title = if running {
        "Prompt  (waiting for Ollama)"
    } else {
        "Prompt  (Enter=send  Esc=close)"
    };
    let input = Paragraph::new(state.review_chat_input.as_str()).block(ui::bordered(input_title));
    frame.render_widget(input, chunks[1]);
    if !running {
        let body = input_body_area(chunks[1]);
        frame.set_cursor_position(Position::new(
            body.x.saturating_add(
                state
                    .review_chat_cursor
                    .min(state.review_chat_input.chars().count())
                    .min(body.width.saturating_sub(1) as usize) as u16,
            ),
            body.y,
        ));
    }

    let hints = if running {
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" close  "),
            Span::styled("Ollama", Style::default().fg(Color::LightCyan)),
            Span::raw(" keeps streaming"),
        ])
    } else {
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" send  "),
            Span::styled("Ctrl+A/E", Style::default().fg(Color::Yellow)),
            Span::raw(" jump  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" close"),
        ])
    };
    frame.render_widget(Paragraph::new(hints), chunks[2]);
}

fn conversation_lines(state: &AppState, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if state.review_chat_messages.is_empty() && state.review_chat_job.is_none() {
        lines.push(Line::from(Span::styled(
            "Ask about the full review: risks, missing tests, compatibility, or follow-up checks.",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    for message in &state.review_chat_messages {
        push_message_lines(&mut lines, message.role, &message.content, width);
    }
    if let Some(job) = &state.review_chat_job {
        let text = if job.output.trim().is_empty() {
            "thinking..."
        } else {
            job.output.trim()
        };
        push_message_lines(&mut lines, ReviewChatRole::Assistant, text, width);
    }
    lines
}

fn push_message_lines(
    lines: &mut Vec<Line<'static>>,
    role: ReviewChatRole,
    content: &str,
    width: u16,
) {
    let (label, color) = match role {
        ReviewChatRole::User => ("you", Color::Yellow),
        ReviewChatRole::Assistant => ("ollama", Color::LightCyan),
    };
    lines.push(Line::from(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
    lines.extend(markdown::render(
        content,
        "  ",
        width.saturating_sub(2).max(1),
    ));
    lines.push(Line::raw(""));
}

fn input_body_area(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
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
    state.review_chat_cursor = state
        .review_chat_cursor
        .min(char_len(&state.review_chat_input));
    let idx = byte_index_for_char(&state.review_chat_input, state.review_chat_cursor);
    state.review_chat_input.insert(idx, c);
    state.review_chat_cursor += 1;
}

fn remove_before_cursor(state: &mut AppState) {
    state.review_chat_cursor = state
        .review_chat_cursor
        .min(char_len(&state.review_chat_input));
    if state.review_chat_cursor == 0 {
        return;
    }
    let start = byte_index_for_char(&state.review_chat_input, state.review_chat_cursor - 1);
    let end = byte_index_for_char(&state.review_chat_input, state.review_chat_cursor);
    state.review_chat_input.replace_range(start..end, "");
    state.review_chat_cursor -= 1;
}

fn remove_at_cursor(state: &mut AppState) {
    state.review_chat_cursor = state
        .review_chat_cursor
        .min(char_len(&state.review_chat_input));
    if state.review_chat_cursor >= char_len(&state.review_chat_input) {
        return;
    }
    let start = byte_index_for_char(&state.review_chat_input, state.review_chat_cursor);
    let end = byte_index_for_char(&state.review_chat_input, state.review_chat_cursor + 1);
    state.review_chat_input.replace_range(start..end, "");
}

pub(crate) fn scroll(state: &mut AppState, scroll_down: bool, amount: u16) {
    state.review_chat_scroll = if scroll_down {
        state.review_chat_scroll.saturating_add(amount)
    } else {
        state.review_chat_scroll.saturating_sub(amount)
    };
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let running = state.review_chat_job.is_some();
    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
        }
        KeyCode::Enter if !running => {
            let prompt = state.review_chat_input.trim().to_string();
            if prompt.is_empty() {
                return Ok(());
            }
            state.review_chat_input.clear();
            state.review_chat_cursor = 0;
            state
                .review_chat_messages
                .push(crate::state::ReviewChatMessage {
                    role: ReviewChatRole::User,
                    content: prompt.clone(),
                });
            state.pending_action = Some(PendingAction::ReviewChat(prompt));
        }
        KeyCode::Char('a') if ctrl && !running => {
            state.review_chat_cursor = 0;
        }
        KeyCode::Char('e') if ctrl && !running => {
            state.review_chat_cursor = char_len(&state.review_chat_input);
        }
        KeyCode::Backspace if !running => {
            remove_before_cursor(state);
        }
        KeyCode::Delete if !running => {
            remove_at_cursor(state);
        }
        KeyCode::Left if !running => {
            state.review_chat_cursor = state.review_chat_cursor.saturating_sub(1);
        }
        KeyCode::Right if !running => {
            state.review_chat_cursor = state
                .review_chat_cursor
                .saturating_add(1)
                .min(char_len(&state.review_chat_input));
        }
        KeyCode::Up => {
            scroll(state, false, 1);
        }
        KeyCode::Down => {
            scroll(state, true, 1);
        }
        KeyCode::Char(c) if !ctrl && !running => {
            insert_at_cursor(state, c);
        }
        _ => {}
    }
    Ok(())
}
