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
    let h = area.height.clamp(9, 11).min(area.height);
    let modal = ui::centered(area, w, h);

    frame.render_widget(Clear, modal);
    let text = vec![
        Line::from(Span::styled(
            prompt.question.clone(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            prompt.detail.clone(),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "This cannot be undone.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Red)),
            Span::raw(" confirm  "),
            Span::styled("n/Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel"),
        ]),
    ];

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
