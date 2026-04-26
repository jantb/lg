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
    app,
    state::{AppState, SPINNER_FRAMES},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let count = if state.branches.is_empty() {
        None
    } else {
        Some((state.branches_idx + 1, state.branches.len()))
    };
    let block = ui::framed_with_activity(
        3,
        "Branches",
        focused,
        count,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let items: Vec<ListItem> = state
        .branches
        .iter()
        .map(|b| {
            let line = if state
                .checkout_job
                .as_ref()
                .is_some_and(|job| job.branch == b.name)
            {
                let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
                Line::from(vec![
                    Span::styled(
                        format!("{spinner} "),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        b.name.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else if b.is_current {
                Line::from(Span::styled(
                    format!("* {}", b.name),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::raw(format!("  {}", b.name)))
            };
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    let mut list_state = ListState::default();
    if focused && !state.branches.is_empty() {
        list_state.select(Some(state.branches_idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !state.branches.is_empty() && state.branches_idx + 1 < state.branches.len() {
                state.branches_idx += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.branches_idx > 0 {
                state.branches_idx -= 1;
            }
        }
        KeyCode::Enter => {
            if let Some(b) = state.branches.get(state.branches_idx) {
                if state.checkout_job.is_none() && !b.is_current {
                    let name = b.name.clone();
                    app::checkout_branch_async(state, name);
                }
            }
        }
        _ => {}
    }
    Ok(())
}
