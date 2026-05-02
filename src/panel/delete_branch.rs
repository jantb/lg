use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    state::{AppState, DeleteBranchField, Modal, PendingAction},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = (area.width * 6 / 10).clamp(56, 96).min(area.width);
    let h = 14u16.min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .split(modal);

    let header = vec![Line::from(vec![
        Span::styled("Delete branch ", Style::default().fg(Color::Gray)),
        Span::styled(
            state.delete_branch_target.clone(),
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    frame.render_widget(
        Paragraph::new(header).block(ui::bordered("Confirm")),
        chunks[0],
    );

    let body = vec![
        toggle_line(
            "delete local",
            state.delete_branch_local,
            state.delete_branch_field == DeleteBranchField::Local,
        ),
        toggle_line(
            "delete remote (origin)",
            state.delete_branch_remote,
            state.delete_branch_field == DeleteBranchField::Remote,
        ),
        toggle_line(
            "force (-D, allows unmerged)",
            state.delete_branch_force,
            state.delete_branch_field == DeleteBranchField::Force,
        ),
        Line::from(""),
        Line::from(Span::styled(
            "j/k or Tab to move  Space toggles  Enter confirms  Esc cancels",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )),
    ];
    frame.render_widget(
        Paragraph::new(body)
            .block(ui::bordered("Options"))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    let controls = vec![Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::raw(" delete  "),
        Span::styled("Esc", Style::default().fg(Color::Gray)),
        Span::raw(" cancel"),
    ])];
    frame.render_widget(
        Paragraph::new(controls).block(Block::default().borders(Borders::ALL)),
        chunks[2],
    );
}

fn toggle_line(label: &str, on: bool, focused: bool) -> Line<'static> {
    let marker = if on { "[x]" } else { "[ ]" };
    let style = if focused {
        Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Line::from(vec![
        Span::styled(if focused { "› " } else { "  " }, style),
        Span::styled(format!("{marker} {label}"), style),
    ])
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
        }
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
            state.delete_branch_field = next_field(state.delete_branch_field);
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
            state.delete_branch_field = prev_field(state.delete_branch_field);
        }
        KeyCode::Char(' ') => toggle_current(state),
        KeyCode::Enter => {
            if !state.delete_branch_local && !state.delete_branch_remote {
                state.set_status("nothing selected to delete", true);
                return Ok(());
            }
            state.pending_action = Some(PendingAction::DeleteBranch {
                name: state.delete_branch_target.clone(),
                delete_local: state.delete_branch_local,
                delete_remote: state.delete_branch_remote,
                force: state.delete_branch_force,
            });
        }
        _ => {}
    }
    Ok(())
}

fn next_field(field: DeleteBranchField) -> DeleteBranchField {
    match field {
        DeleteBranchField::Local => DeleteBranchField::Remote,
        DeleteBranchField::Remote => DeleteBranchField::Force,
        DeleteBranchField::Force => DeleteBranchField::Local,
    }
}

fn prev_field(field: DeleteBranchField) -> DeleteBranchField {
    match field {
        DeleteBranchField::Local => DeleteBranchField::Force,
        DeleteBranchField::Remote => DeleteBranchField::Local,
        DeleteBranchField::Force => DeleteBranchField::Remote,
    }
}

fn toggle_current(state: &mut AppState) {
    match state.delete_branch_field {
        DeleteBranchField::Local => state.delete_branch_local = !state.delete_branch_local,
        DeleteBranchField::Remote => state.delete_branch_remote = !state.delete_branch_remote,
        DeleteBranchField::Force => state.delete_branch_force = !state.delete_branch_force,
    }
}
