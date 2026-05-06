use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::{
    app,
    state::{AppState, clamp_index},
    ui,
};

use super::scroll;

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
            "Merge conflict detected",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("Resolve the conflict outside lg, then press v to validate and continue."),
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
    let selected_idx = clamp_index(state.conflict_idx, state.conflicts.len());
    let offset = scroll::selection_scroll_offset(
        selected_idx,
        state.conflicts.len(),
        scroll::list_viewport_height(body[0].height),
        state.conflict_scroll_offset,
    );
    let mut list_state = scroll::list_state(selected_idx, offset);
    frame.render_stateful_widget(list, body[0], &mut list_state);

    let detail = if let Some(path) = state.conflicts.get(state.conflict_idx) {
        let mut text = format!(
            "{path}\n\nlg will not edit conflict contents. Resolve the file in your editor or with git, then press v."
        );
        if !state.conflict_log.trim().is_empty() {
            text.push_str("\n\nLast message:\n");
            text.push_str(&state.conflict_log);
        }
        text
    } else if state.conflict_log.trim().is_empty() {
        "No conflicted file selected.\n\nIf you already completed the merge, press v to let lg detect that and finish the flow.".to_string()
    } else {
        state.conflict_log.clone()
    };
    frame.render_widget(
        Paragraph::new(detail)
            .block(ui::bordered("Next Step"))
            .wrap(Wrap { trim: false }),
        body[1],
    );

    let controls = vec![Line::from(vec![
        Span::styled("v", Style::default().fg(Color::Green)),
        Span::raw(" validate resolved/staged/merged state  "),
        Span::styled("a", Style::default().fg(Color::Red)),
        Span::raw(" abort  "),
        Span::styled("Esc", Style::default().fg(Color::Gray)),
        Span::raw(" close"),
    ])];
    frame.render_widget(
        Paragraph::new(controls).block(Block::default().borders(Borders::ALL)),
        chunks[2],
    );
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    let files_area = files_area(area);
    state.conflict_scroll_offset = scroll::selection_scroll_offset(
        clamp_index(state.conflict_idx, state.conflicts.len()),
        state.conflicts.len(),
        scroll::list_viewport_height(files_area.height),
        state.conflict_scroll_offset,
    );
}

fn files_area(area: Rect) -> Rect {
    let w = (area.width * 8 / 10).clamp(72, 140).min(area.width);
    let h = (area.height * 4 / 5).clamp(18, 44).min(area.height);
    let modal = ui::centered(area, w, h);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(7),
            Constraint::Length(5),
        ])
        .split(modal);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[1]);
    body[0]
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    state.conflict_idx = clamp_index(state.conflict_idx, state.conflicts.len()).unwrap_or(0);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.conflict_idx = state
                .conflict_idx
                .saturating_add(1)
                .min(state.conflicts.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.conflict_idx = state.conflict_idx.saturating_sub(1);
        }
        KeyCode::Char('v') | KeyCode::Char('V') => app::validate_conflict_resolution(state),
        KeyCode::Char('a') | KeyCode::Char('A') => app::abort_conflict_operation(state),
        KeyCode::Esc => {
            state.modal = crate::state::Modal::None;
        }
        _ => {}
    }
    Ok(())
}
