use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    state::{AppState, Modal},
    ui,
};

/// The prompt's lines and the box they are drawn in. The buttons are the last
/// line.
fn layout(state: &AppState, area: Rect) -> Option<(Rect, Vec<Line<'static>>)> {
    let prompt = state.confirm.as_ref()?;

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
    for paragraph in prompt.detail.lines() {
        for line in super::wrap_words(paragraph, inner) {
            text.push(Line::from(Span::styled(
                line,
                Style::default().fg(Color::Yellow),
            )));
        }
    }
    if !prompt.reversible {
        text.push(Line::from(""));
        text.push(Line::from(Span::styled(
            "This cannot be undone.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    text.push(Line::from(""));
    text.push(Line::from(
        BUTTONS
            .iter()
            .map(|(label, key)| match key {
                Some(KeyCode::Char('y')) if label.len() == 1 => {
                    Span::styled(*label, Style::default().fg(Color::Red))
                }
                Some(KeyCode::Char('n')) if label.starts_with('n') => {
                    Span::styled(*label, Style::default().fg(Color::Gray))
                }
                _ => Span::raw(*label),
            })
            .collect::<Vec<_>>(),
    ));

    let h = (text.len() as u16 + 2).max(9).min(area.height);
    Some((ui::centered(area, w, h), text))
}

/// The button line, as the segments a click is matched against.
const BUTTONS: &[(&str, Option<KeyCode>)] = &[
    ("y", Some(KeyCode::Char('y'))),
    (" confirm", Some(KeyCode::Char('y'))),
    ("  ", None),
    ("n/Esc", Some(KeyCode::Char('n'))),
    (" cancel", Some(KeyCode::Char('n'))),
];

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let Some((modal, text)) = layout(state, area) else {
        return;
    };
    let Some(prompt) = &state.confirm else {
        return;
    };
    let inner = ui::modal_frame(frame, modal, &prompt.title);
    frame.render_widget(Paragraph::new(text), inner);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &[], frame);
    }
}

/// A click on `y confirm` confirms and one on `n/Esc cancel` cancels, as the
/// keys do.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> Result<()> {
    if !super::pointer::left_click(m) {
        return Ok(());
    }
    let Some((modal, text)) = layout(state, area) else {
        return Ok(());
    };
    let inner = ui::modal_inner(modal);
    let buttons_row = inner.y + text.len().saturating_sub(1) as u16;
    if m.row != buttons_row {
        return Ok(());
    }
    if let Some(code) = super::pointer::button_at(inner.x, BUTTONS, m.column) {
        handle_key(state, KeyEvent::from(code))?;
    }
    Ok(())
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            state.modal = Modal::None;
            if let Some(prompt) = state.confirm.take() {
                match prompt.action {
                    // Nothing to wait for: the session is lg's own.
                    crate::state::PendingAction::CloseSession { id, follow } => {
                        crate::panel::environments::close_session(state, id, follow)
                    }
                    action => state.pending_action = Some(action),
                }
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
