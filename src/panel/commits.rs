use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

use crate::{
    state::{AppState, Pane},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let count = if state.commits.is_empty() {
        None
    } else {
        Some((state.commits_idx + 1, state.commits.len()))
    };
    let block = ui::framed(4, "Commits", focused, count);

    let items: Vec<ListItem> = state
        .commits
        .iter()
        .map(|c| {
            let subject_style = if state.unpushed_shas.contains(&c.sha) {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            let line = Line::from(vec![
                Span::styled(format!("{} ", c.sha), Style::default().fg(Color::DarkGray)),
                Span::styled(c.subject.clone(), subject_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    let mut list_state = ListState::default();
    if focused && !state.commits.is_empty() {
        list_state.select(Some(state.commits_idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !state.commits.is_empty() && state.commits_idx + 1 < state.commits.len() {
                state.commits_idx += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.commits_idx > 0 {
                state.commits_idx -= 1;
            }
        }
        KeyCode::Enter => {
            state.focus = Pane::Main;
        }
        _ => {}
    }
    Ok(())
}
