use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{app, git::ConflictChoice, state::AppState, ui};

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

    let detail = if let Some(patch) = &state.pending_llm_patch {
        format!("LLM patch preview. Press p to apply.\n\n{patch}")
    } else if let Some(path) = state.conflicts.get(state.conflict_idx) {
        match crate::git::conflict_hunks(path) {
            Ok(hunks) if !hunks.is_empty() => {
                let idx = state.conflict_hunk_idx.min(hunks.len() - 1);
                let h = &hunks[idx];
                let mut text = format!("{}  hunk {} of {}\n\n", path, idx + 1, hunks.len());
                text.push_str("<<<<<<< ours\n");
                text.push_str(&h.ours);
                if let Some(base) = &h.base {
                    text.push_str("||||||| base\n");
                    text.push_str(base);
                }
                text.push_str("======= theirs\n");
                text.push_str(&h.theirs);
                text.push_str(">>>>>>> theirs\n");
                text
            }
            Ok(_) => {
                if state.conflict_log.trim().is_empty() {
                    format!("{path}\n\nNo conflict markers remain. Press s to stage this file.")
                } else {
                    state.conflict_log.clone()
                }
            }
            Err(e) => format!("failed to read conflict hunks: {e}"),
        }
    } else if state.conflict_log.trim().is_empty() {
        "No conflicted file selected.".to_string()
    } else {
        state.conflict_log.clone()
    };
    frame.render_widget(
        Paragraph::new(detail)
            .block(ui::bordered("Merge Editor / Preview"))
            .wrap(Wrap { trim: false }),
        body[1],
    );

    let controls = vec![
        Line::from(vec![
            Span::styled("l", Style::default().fg(Color::Cyan)),
            Span::raw(" LLM  "),
            Span::styled("p", Style::default().fg(Color::Cyan)),
            Span::raw(" apply patch  "),
            Span::styled("o/t/b", Style::default().fg(Color::Yellow)),
            Span::raw(" ours/theirs/both  "),
            Span::styled("[/]", Style::default().fg(Color::Yellow)),
            Span::raw(" hunk  "),
            Span::styled("s", Style::default().fg(Color::Green)),
            Span::raw(" stage  "),
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
                state.conflict_hunk_idx = 0;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.conflict_idx = state.conflict_idx.saturating_sub(1);
            state.conflict_hunk_idx = 0;
        }
        KeyCode::Char(']') => {
            state.conflict_hunk_idx = state.conflict_hunk_idx.saturating_add(1);
        }
        KeyCode::Char('[') => {
            state.conflict_hunk_idx = state.conflict_hunk_idx.saturating_sub(1);
        }
        KeyCode::Char('o') | KeyCode::Char('O') => resolve_selected(state, ConflictChoice::Ours),
        KeyCode::Char('t') | KeyCode::Char('T') => resolve_selected(state, ConflictChoice::Theirs),
        KeyCode::Char('b') | KeyCode::Char('B') => resolve_selected(state, ConflictChoice::Both),
        KeyCode::Char('s') | KeyCode::Char('S') => stage_selected(state),
        KeyCode::Char('p') | KeyCode::Char('P') => app::apply_pending_llm_patch(state),
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

fn resolve_selected(state: &mut AppState, choice: ConflictChoice) {
    let Some(path) = state.conflicts.get(state.conflict_idx).cloned() else {
        return;
    };
    match crate::git::resolve_conflict_hunk(&path, state.conflict_hunk_idx, choice) {
        Ok(()) => {
            state.conflict_log = format!("Resolved hunk {} in {path}", state.conflict_hunk_idx + 1);
            state.conflict_hunk_idx = 0;
        }
        Err(e) => state.conflict_log = e.to_string(),
    }
}

fn stage_selected(state: &mut AppState) {
    let Some(path) = state.conflicts.get(state.conflict_idx).cloned() else {
        return;
    };
    match crate::git::stage_if_resolved(&path) {
        Ok(()) => {
            state.conflict_log = format!("staged {path}");
            state.conflicts.remove(state.conflict_idx);
            state.conflict_idx = state.conflict_idx.saturating_sub(1);
            state.conflict_hunk_idx = 0;
        }
        Err(e) => state.conflict_log = e.to_string(),
    }
}
