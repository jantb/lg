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
    state::{AppState, Modal, PendingAction},
    ui,
};

pub fn render(_state: &AppState, area: Rect, frame: &mut Frame) {
    let w = area.width.clamp(48, 72).min(area.width);
    let h = area.height.clamp(7, 9).min(area.height);
    let modal = ui::centered(area, w, h);

    frame.render_widget(Clear, modal);
    let text = vec![
        Line::from(Span::styled(
            "No staged changes",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Stage everything and continue to commit?"),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Green)),
            Span::raw(" stage all  "),
            Span::styled("n/Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel"),
        ]),
    ];

    frame.render_widget(Paragraph::new(text).block(ui::bordered("Commit")), modal);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            state.modal = Modal::None;
            state.pending_action = Some(PendingAction::StageAllAndCommit);
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.modal = Modal::None;
        }
        _ => {}
    }
    Ok(())
}
