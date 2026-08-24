use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::{
    state::{AppState, Modal},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let Some(prompt) = &state.confirm else {
        return;
    };

    let w = area.width.clamp(48, 72).min(area.width);
    let inner = w.saturating_sub(2) as usize;

    let mut text: Vec<Line> = Vec::new();
    for line in super::wrap_words(&prompt.question, inner) {
        text.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }
    text.push(Line::from(""));
    // The detail is what the action will actually do, and for a multi-step one
    // the last step is usually the one worth reading. It wraps, and keeps the
    // line breaks it was written with, rather than being cut off mid-path.
    for paragraph in prompt.detail.lines() {
        for line in super::wrap_words(paragraph, inner) {
            text.push(Line::from(Span::styled(
                line,
                Style::default().fg(Color::Yellow),
            )));
        }
    }
    text.push(Line::from(""));
    text.push(Line::from(Span::styled(
        "This cannot be undone.",
        Style::default().fg(Color::DarkGray),
    )));
    text.push(Line::from(""));
    text.push(Line::from(vec![
        Span::styled("y", Style::default().fg(Color::Red)),
        Span::raw(" confirm  "),
        Span::styled("n/Esc", Style::default().fg(Color::Gray)),
        Span::raw(" cancel"),
    ]));

    // Grown to fit the wrapped text, so a long detail is read rather than
    // guessed at.
    let h = (text.len() as u16 + 2).max(9).min(area.height);
    let modal = ui::centered(area, w, h);

    frame.render_widget(Clear, modal);
    frame.render_widget(
        Paragraph::new(text).block(ui::bordered(&prompt.title)),
        modal,
    );
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            state.modal = Modal::None;
            if let Some(prompt) = state.confirm.take() {
                state.pending_action = Some(prompt.action);
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.modal = Modal::None;
            state.confirm = None;
        }
        _ => {}
    }
    Ok(())
}
