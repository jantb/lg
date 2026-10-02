use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    state::{AppState, AuthorField, Modal, PendingAction},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = 72.min(area.width);
    let h = 10.min(area.height);
    let modal = ui::centered(area, w, h);
    if modal.width < 24 || modal.height < 8 {
        let inner = ui::modal_frame(frame, modal, "Author");
        frame.render_widget(
            Paragraph::new("Terminal too small for author settings"),
            inner,
        );
        if state.decorative_animations {
            ui::animate_modal_border(state.animation_ms, modal, &[], frame);
        }
        return;
    }

    let status = if state.author.has_subtree_rule {
        "subtree rule"
    } else if state.author.has_local_override {
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
            state.author.path.as_str(),
            state.author.field == AuthorField::Path,
        ),
        field_line(
            "Name",
            state.author.name.as_str(),
            state.author.field == AuthorField::Name,
        ),
        field_line(
            "Email",
            state.author.email.as_str(),
            state.author.field == AuthorField::Email,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" field    "),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" save local    "),
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

    let inner = ui::modal_frame(frame, modal, "Author Settings");
    frame.render_widget(Paragraph::new(lines), inner);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &[], frame);
    }
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
    let (row, value_len) = match state.author.field {
        AuthorField::Path => (3, state.author.path.before_cursor().chars().count()),
        AuthorField::Name => (4, state.author.name.before_cursor().chars().count()),
        AuthorField::Email => (5, state.author.email.before_cursor().chars().count()),
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
            state.author.field = match state.author.field {
                AuthorField::Path => AuthorField::Name,
                AuthorField::Name => AuthorField::Email,
                AuthorField::Email => AuthorField::Path,
            };
        }
        KeyCode::Up => {
            state.author.field = match state.author.field {
                AuthorField::Path => AuthorField::Email,
                AuthorField::Name => AuthorField::Path,
                AuthorField::Email => AuthorField::Name,
            };
        }
        KeyCode::Enter => {
            state.pending_action = Some(PendingAction::SaveAuthor {
                name: state.author.name.text.clone(),
                email: state.author.email.text.clone(),
            });
        }
        KeyCode::Char('l') if ctrl => {
            state.pending_action = Some(PendingAction::SaveAuthor {
                name: state.author.name.text.clone(),
                email: state.author.email.text.clone(),
            });
        }
        KeyCode::Char('u') if ctrl => {
            state.pending_action = Some(PendingAction::ClearSubtreeAuthor {
                path: state.author.path.text.clone(),
            });
        }
        KeyCode::Char('x') if ctrl => {
            state.pending_action = Some(PendingAction::ClearAuthor);
        }
        _ if !ctrl => {
            state.author.focused_mut().handle_key(key);
        }
        _ => {}
    }
    Ok(())
}
