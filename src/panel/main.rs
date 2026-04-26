use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::{config::DIFF_PAGE, state::AppState, ui};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let block = ui::framed_with_activity(
        0,
        "Diff",
        focused,
        None,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let lines: Vec<ratatui::text::Line> = state
        .diff_text
        .split('\n')
        .map(ui::highlight_diff_line)
        .collect();

    let max_offset = state
        .diff_line_count
        .saturating_sub(state.diff_viewport_height);
    let offset = state.diff_offset.min(max_offset);

    let para = Paragraph::new(lines).block(block).scroll((offset, 0));

    frame.render_widget(para, area);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let max_offset = state
        .diff_line_count
        .saturating_sub(state.diff_viewport_height);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.diff_offset = state.diff_offset.saturating_add(1).min(max_offset);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.diff_offset = state.diff_offset.saturating_sub(1);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.diff_offset = state.diff_offset.saturating_add(DIFF_PAGE).min(max_offset);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.diff_offset = state.diff_offset.saturating_sub(DIFF_PAGE);
        }
        KeyCode::Char('g') => {
            state.diff_offset = 0;
        }
        KeyCode::Char('G') => {
            state.diff_offset = max_offset;
        }
        _ => {}
    }
    Ok(())
}
