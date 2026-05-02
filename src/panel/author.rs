use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::{
    state::{AppState, AuthorField, Modal, PendingAction},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = 72.min(area.width);
    let h = 10.min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);

    let status = if state.author_has_subtree_rule {
        "subtree rule"
    } else if state.author_has_local_override {
        "repo-local override"
    } else {
        "using inherited/default author"
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("Mode:  ", Style::default().fg(Color::Yellow)),
            Span::styled(status, Style::default().fg(Color::Gray)),
        ]),
        Line::from(""),
        field_line(
            "Folder",
            &state.author_path_input,
            state.author_field == AuthorField::Path,
        ),
        field_line(
            "Name",
            &state.author_name_input,
            state.author_field == AuthorField::Name,
        ),
        field_line(
            "Email",
            &state.author_email_input,
            state.author_field == AuthorField::Email,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" field    "),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" save subtree    "),
            Span::styled("Ctrl+L", Style::default().fg(Color::Green)),
            Span::raw(" save local    "),
            Span::styled("Ctrl+U", Style::default().fg(Color::Red)),
            Span::raw(" clear subtree    "),
            Span::styled("Ctrl+X", Style::default().fg(Color::Red)),
            Span::raw(" clear local    "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel"),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(ui::bordered("Author Settings")),
        modal,
    );
    if let Some((x, y)) = active_field_cursor(state, modal) {
        frame.set_cursor_position(Position::new(x, y));
    }
}

fn field_line(label: &'static str, value: &str, selected: bool) -> Line<'static> {
    let label_style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let value_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Line::from(vec![
        Span::styled(format!("{label:<6}"), label_style),
        Span::styled(value.to_string(), value_style),
    ])
}

fn active_field_cursor(state: &AppState, modal: Rect) -> Option<(u16, u16)> {
    if modal.width <= 2 || modal.height <= 2 {
        return None;
    }
    let (row, value_len) = match state.author_field {
        AuthorField::Path => (3, state.author_path_input.chars().count()),
        AuthorField::Name => (4, state.author_name_input.chars().count()),
        AuthorField::Email => (5, state.author_email_input.chars().count()),
    };
    let content_width = modal.width.saturating_sub(2);
    let label_width = 6u16;
    let cursor_x = label_width.saturating_add(
        value_len.min(content_width.saturating_sub(label_width + 1) as usize) as u16,
    );
    Some((modal.x + 1 + cursor_x, modal.y + row))
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
        }
        KeyCode::Tab | KeyCode::Down => {
            state.author_field = match state.author_field {
                AuthorField::Path => AuthorField::Name,
                AuthorField::Name => AuthorField::Email,
                AuthorField::Email => AuthorField::Path,
            };
        }
        KeyCode::Up => {
            state.author_field = match state.author_field {
                AuthorField::Path => AuthorField::Email,
                AuthorField::Name => AuthorField::Path,
                AuthorField::Email => AuthorField::Name,
            };
        }
        KeyCode::Enter => {
            state.pending_action = Some(PendingAction::SaveSubtreeAuthor {
                path: state.author_path_input.clone(),
                name: state.author_name_input.clone(),
                email: state.author_email_input.clone(),
            });
        }
        KeyCode::Char('l') if ctrl => {
            state.pending_action = Some(PendingAction::SaveAuthor {
                name: state.author_name_input.clone(),
                email: state.author_email_input.clone(),
            });
        }
        KeyCode::Char('u') if ctrl => {
            state.pending_action = Some(PendingAction::ClearSubtreeAuthor {
                path: state.author_path_input.clone(),
            });
        }
        KeyCode::Char('x') if ctrl => {
            state.pending_action = Some(PendingAction::ClearAuthor);
        }
        KeyCode::Backspace if !ctrl => match state.author_field {
            AuthorField::Path => {
                state.author_path_input.pop();
            }
            AuthorField::Name => {
                state.author_name_input.pop();
            }
            AuthorField::Email => {
                state.author_email_input.pop();
            }
        },
        KeyCode::Char(c) if !ctrl => match state.author_field {
            AuthorField::Path => state.author_path_input.push(c),
            AuthorField::Name => state.author_name_input.push(c),
            AuthorField::Email => state.author_email_input.push(c),
        },
        _ => {}
    }
    Ok(())
}
