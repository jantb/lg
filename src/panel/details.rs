//! The whole of the last error, for when the status bar had to cut it short.
//!
//! The status bar has one line, shares it with the key hints, and lets an
//! error go after half a minute. Git's complaint about a refused push or a
//! failing hook is often a dozen lines, and the line that says why is rarely
//! the first.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::state::{AppState, Modal, PendingAction};
use crate::ui;

/// What the popup shows: the status on screen when it is an error or runs to
/// more than one line, otherwise the last error this session.
pub fn details_text(state: &AppState) -> Option<&str> {
    match &state.status {
        Some(status) if status.is_error || status.text.trim().contains('\n') => {
            Some(status.text.as_str())
        }
        _ => state.last_error.as_ref().map(|status| status.text.as_str()),
    }
}

/// Open the popup, saying so when there is nothing to show.
pub fn open(state: &mut AppState) {
    if details_text(state).is_none() {
        state.set_status("no error to show details of", false);
        return;
    }
    state.details_offset = 0;
    state.modal = Modal::StatusDetails;
}

/// Where the popup sits in `area`.
pub fn popup_area(area: Rect) -> Rect {
    let width = area.width.saturating_sub(8).clamp(20.min(area.width), 110);
    let height = area.height.saturating_sub(4).clamp(5.min(area.height), 30);
    ui::centered(area, width, height)
}

/// The rows the text takes at the popup's width once wrapped.
fn content_rows(state: &AppState, area: Rect) -> Vec<Line<'static>> {
    let width = popup_area(area).width.saturating_sub(2).max(1) as usize;
    details_text(state)
        .unwrap_or_default()
        .lines()
        .flat_map(|line| ui::word_wrap(Line::raw(line), width))
        .collect()
}

fn viewport(area: Rect) -> u16 {
    popup_area(area).height.saturating_sub(2).max(1)
}

fn max_offset(state: &AppState, area: Rect) -> u16 {
    let rows = u16::try_from(content_rows(state, area).len()).unwrap_or(u16::MAX);
    rows.saturating_sub(viewport(area))
}

pub fn scroll(state: &mut AppState, area: Rect, down: bool, amount: u16) {
    let max = max_offset(state, area);
    state.details_offset = if down {
        state.details_offset.saturating_add(amount).min(max)
    } else {
        state.details_offset.saturating_sub(amount)
    };
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let popup = popup_area(area);
    let title = match &state.status {
        Some(status) if !status.is_error && details_text(state) == Some(status.text.as_str()) => {
            "Status details"
        }
        _ => "Error details",
    };
    let hint = Line::from(Span::styled(
        "j/k scroll \u{2022} y copy \u{2022} q/Esc close",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    ))
    .alignment(Alignment::Right);
    let block = ui::bordered(title).title_bottom(hint);
    let inner = ui::modal_frame_with(frame, popup, block);
    let offset = state.details_offset.min(max_offset(state, area)) as usize;
    let rows: Vec<Line<'static>> = content_rows(state, area)
        .into_iter()
        .skip(offset)
        .take(inner.height as usize)
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent, area: Rect) -> Result<()> {
    let page = viewport(area).saturating_sub(1).max(1);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => scroll(state, area, true, 1),
        KeyCode::Char('k') | KeyCode::Up => scroll(state, area, false, 1),
        KeyCode::PageDown | KeyCode::Char(' ') => scroll(state, area, true, page),
        KeyCode::PageUp => scroll(state, area, false, page),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, area, true, page / 2)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, area, false, page / 2)
        }
        KeyCode::Char('g') => state.details_offset = 0,
        KeyCode::Char('G') => state.details_offset = max_offset(state, area),
        KeyCode::Char('y') => {
            if let Some(text) = details_text(state) {
                state.pending_action = Some(PendingAction::CopyToClipboard {
                    label: "error details".into(),
                    text: text.to_string(),
                });
            }
        }
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('!') | KeyCode::Enter => {
            state.modal = Modal::None;
            state.details_offset = 0;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_popup_keeps_the_last_error_after_the_status_has_moved_on() {
        let mut state = AppState::new();
        state.set_status("git push origin failed: rejected\nhint: pull first", true);
        state.set_status("fetched", false);

        assert_eq!(
            details_text(&state),
            Some("git push origin failed: rejected\nhint: pull first")
        );
    }

    #[test]
    fn with_no_error_there_is_nothing_to_open() {
        let mut state = AppState::new();
        state.set_status("fetched", false);

        open(&mut state);

        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn scrolling_stops_at_the_last_line() {
        let area = Rect::new(0, 0, 80, 12);
        let mut state = AppState::new();
        let long: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
        state.set_status(long.join("\n"), true);
        open(&mut state);

        for _ in 0..200 {
            scroll(&mut state, area, true, 3);
        }

        assert_eq!(state.details_offset, max_offset(&state, area));
        assert!(state.details_offset > 0);
    }

    #[test]
    fn the_last_line_of_a_word_wrapped_error_can_be_scrolled_into_view() {
        let area = Rect::new(0, 0, 80, 12);
        let mut state = AppState::new();
        let wrapping = format!("{} {} {}", "a".repeat(40), "b".repeat(40), "c".repeat(40));
        let mut lines = vec![wrapping; 10];
        lines.push("the reason it failed".to_string());
        state.set_status(lines.join("\n"), true);
        open(&mut state);

        handle_key(&mut state, KeyEvent::from(KeyCode::Char('G')), area).unwrap();

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
                .unwrap();
        terminal
            .draw(|frame| render(&state, frame.area(), frame))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("the reason it failed"), "{screen}");
    }
}
