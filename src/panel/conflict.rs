use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{app, state::AppState, ui};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = (area.width * 8 / 10).clamp(72, 140).min(area.width);
    let h = (area.height * 4 / 5).clamp(18, 44).min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(7),
            Constraint::Length(5),
        ])
        .split(modal);

    let header = vec![
        Line::from(Span::styled(
            "Merge conflict resolver",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("Resolve files, validate with project tests, then continue the Git operation."),
    ];
    frame.render_widget(
        Paragraph::new(header).block(ui::bordered("Conflict")),
        chunks[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[1]);

    let items: Vec<ListItem> = state
        .conflicts
        .iter()
        .map(|path| ListItem::new(Line::from(path.as_str())))
        .collect();
    let list = List::new(items)
        .block(ui::bordered("Files"))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");
    let mut list_state = ListState::default();
    if !state.conflicts.is_empty() {
        list_state.select(Some(state.conflict_idx.min(state.conflicts.len() - 1)));
    }
    frame.render_stateful_widget(list, body[0], &mut list_state);

    let log = if state.conflict_log.trim().is_empty() {
        "No validation log yet. Press v to run cargo/gradle validation.".to_string()
    } else {
        state.conflict_log.clone()
    };
    frame.render_widget(
        Paragraph::new(log)
            .block(ui::bordered("LLM / Validation Log"))
            .wrap(Wrap { trim: false }),
        body[1],
    );

    let controls = vec![
        Line::from(vec![
            Span::styled("l", Style::default().fg(Color::Cyan)),
            Span::raw(" ask LLM  "),
            Span::styled("v", Style::default().fg(Color::Green)),
            Span::raw(" validate  "),
            Span::styled("c", Style::default().fg(Color::Yellow)),
            Span::raw(" continue  "),
            Span::styled("a", Style::default().fg(Color::Red)),
            Span::raw(" abort  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" close"),
        ]),
        Line::from(
            "Detected commands: Cargo projects use cargo test/clippy; Gradle projects use gradle test.",
        ),
    ];
    frame.render_widget(
        Paragraph::new(controls).block(Block::default().borders(Borders::ALL)),
        chunks[2],
    );
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if state.conflict_idx + 1 < state.conflicts.len() {
                state.conflict_idx += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.conflict_idx = state.conflict_idx.saturating_sub(1);
        }
        KeyCode::Char('l') | KeyCode::Char('L') => app::run_conflict_llm(state),
        KeyCode::Char('v') | KeyCode::Char('V') => app::run_conflict_validation(state),
        KeyCode::Char('c') | KeyCode::Char('C') => app::continue_conflict_operation(state),
        KeyCode::Char('a') | KeyCode::Char('A') => app::abort_conflict_operation(state),
        KeyCode::Esc => {
            state.modal = crate::state::Modal::None;
        }
        _ => {}
    }
    Ok(())
}
