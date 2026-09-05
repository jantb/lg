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
    state::{AppState, PendingAction, clamp_index},
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
            Constraint::Length(5),
            Constraint::Min(7),
            Constraint::Length(4),
        ])
        .split(modal);

    let header = vec![
        Line::from(Span::styled(
            "Git conflict detected",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "Resolve the conflict outside lg, with l or with c ({}), then press v to continue.",
            state.preferred_agent.label()
        )),
        local_pass_line(state),
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
        .map(|path| ListItem::new(conflict_row(state, path)))
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
        let mut text = if state.conflict_resolved.contains(path) {
            format!(
                "{path}\n\nThe local model settled every conflict in this file and wrote it back. Nothing is staged yet \u{2014} open it with o and read the merge before pressing v."
            )
        } else {
            format!(
                "{path}\n\nlg has not touched this file. Resolve it in your editor, with l, or with c ({}), then press v.",
                state.preferred_agent.label()
            )
        };
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

    let controls = vec![
        Line::from(vec![
            Span::styled("j/k", Style::default().fg(Color::LightCyan)),
            Span::raw(" select  "),
            Span::styled("o/Enter", Style::default().fg(Color::LightCyan)),
            Span::raw(" open  "),
            Span::styled("v", Style::default().fg(Color::Green)),
            Span::raw(" validate resolved/staged/merged state"),
        ]),
        Line::from(vec![
            Span::styled("l", Style::default().fg(Color::LightGreen)),
            Span::raw(" try the local model, then claude  "),
            Span::styled("c", Style::default().fg(Color::LightGreen)),
            Span::raw(format!(" {}  ", state.preferred_agent.label())),
            Span::styled("a", Style::default().fg(Color::Red)),
            Span::raw(" abort  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" close  "),
            Span::styled("F", Style::default().fg(Color::Gray)),
            Span::raw(" reopens this later"),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(controls).block(Block::default().borders(Borders::ALL)),
        chunks[2],
    );
}

/// The header's third line: what the local pass is doing, or what it did, or
/// the reminder that committing by hand is fine when it has not run.
fn local_pass_line(state: &AppState) -> Line<'static> {
    if let Some(job) = state.conflict_resolve_job.as_ref() {
        let progress = format!("local model: {}/{} file(s)", job.completed, job.total);
        let detail = job
            .active_path
            .as_deref()
            .map(|path| format!(" \u{2014} {path}"))
            .unwrap_or_default();
        return Line::from(Span::styled(
            format!("{progress}{detail}"),
            Style::default().fg(Color::LightGreen),
        ));
    }
    if state.conflict_resolved.is_empty() {
        return Line::from("Committing the resolution yourself is fine; v continues either way.");
    }
    Line::from(Span::styled(
        format!(
            "{} file(s) resolved by the local model \u{2014} read them before v.",
            state.conflict_resolved.len()
        ),
        Style::default().fg(Color::LightGreen),
    ))
}

/// One row of the file list, marked when the local model has already settled
/// it. The mark is what separates a file waiting to be read from one waiting
/// to be resolved; both are still conflicted as far as git is concerned.
fn conflict_row(state: &AppState, path: &str) -> Line<'static> {
    if state.conflict_resolved.contains(path) {
        Line::from(vec![
            Span::styled("\u{2713} ", Style::default().fg(Color::LightGreen)),
            Span::raw(path.to_string()),
        ])
    } else {
        Line::from(format!("  {path}"))
    }
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
            Constraint::Length(5),
            Constraint::Min(7),
            Constraint::Length(4),
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
    // The local pass is rewriting these files. Reading them is fine; settling,
    // aborting or starting a second resolver on top of it is not.
    if state.conflict_resolve_job.is_some()
        && matches!(
            key.code,
            KeyCode::Char('c' | 'C' | 'v' | 'V' | 'a' | 'A' | 'l' | 'L')
        )
    {
        state.set_status(
            "the local model is still working \u{2014} wait, or press Esc to stop it",
            false,
        );
        return Ok(());
    }
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
        KeyCode::Char('o') | KeyCode::Enter => {
            if let Some(path) = state.conflicts.get(state.conflict_idx) {
                state.pending_action = Some(PendingAction::OpenFile(path.clone()));
            } else {
                state.set_status("no conflicted file selected", false);
            }
        }
        KeyCode::Char('l') | KeyCode::Char('L') => app::spawn_conflict_resolve(state),
        KeyCode::Char('c') => app::start_conflict_session(state, true),
        KeyCode::Char('C') => app::start_conflict_session(state, false),
        KeyCode::Char('v') | KeyCode::Char('V') => app::validate_conflict_resolution(state),
        KeyCode::Char('a') | KeyCode::Char('A') => app::abort_conflict_operation(state),
        KeyCode::Esc => {
            // Esc stops LLM work everywhere else in lg, and a local pass
            // halfway through the files is exactly what someone hitting Esc
            // here means to stop. The modal stays up: the conflict is still
            // unresolved, and closing it would hide that.
            if state.conflict_resolve_job.is_some() {
                if let Some(message) = state.cancel_llm_jobs() {
                    state.set_status(message, false);
                }
            } else {
                state.modal = crate::state::Modal::None;
            }
        }
        _ => {}
    }
    Ok(())
}
