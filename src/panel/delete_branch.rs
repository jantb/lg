use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    state::{AppState, DeleteBranchField, Modal, PendingAction},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = (area.width * 6 / 10).clamp(56, 96).min(area.width);
    let h = 12u16.min(area.height);
    let modal = ui::centered(area, w, h);
    let inner = ui::modal_frame(frame, modal, "Confirm");
    let (chunks, dividers) = ui::modal_rows(
        frame,
        inner,
        &[
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ],
    );

    let header = vec![Line::from(vec![
        Span::styled("Delete branch ", Style::default().fg(Color::Gray)),
        Span::styled(
            state.delete_branch_target.clone(),
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    frame.render_widget(Paragraph::new(header), chunks[0]);

    let mut body = if state.delete_branch_remote_available {
        vec![toggle_line(
            "delete local",
            state.delete_branch_local,
            state.delete_branch_field == DeleteBranchField::Local,
        )]
    } else {
        vec![Line::from(vec![
            Span::raw("  "),
            Span::styled("delete local branch", Style::default().fg(Color::Gray)),
        ])]
    };
    if state.delete_branch_remote_available {
        body.push(toggle_line(
            "delete remote (origin)",
            state.delete_branch_remote,
            state.delete_branch_field == DeleteBranchField::Remote,
        ));
    }
    body.extend([toggle_line(
        "force local delete (-D, for unmerged branches)",
        state.delete_branch_force,
        state.delete_branch_field == DeleteBranchField::Force,
    )]);
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), chunks[1]);
    ui::section_title(frame, chunks[1], "Options");

    let controls = vec![Line::from(vec![
        Span::styled("j/k", Style::default().fg(Color::LightCyan)),
        Span::raw(" move  "),
        Span::styled("Space", Style::default().fg(Color::Yellow)),
        Span::raw(" toggle  "),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::raw(" delete  "),
        Span::styled("Esc", Style::default().fg(Color::Gray)),
        Span::raw(" cancel"),
    ])];
    frame.render_widget(Paragraph::new(controls), chunks[2]);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
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
            state.delete_branch_field = next_field(state);
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
            state.delete_branch_field = prev_field(state);
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

fn next_field(state: &AppState) -> DeleteBranchField {
    if !state.delete_branch_remote_available {
        return DeleteBranchField::Force;
    }
    match state.delete_branch_field {
        DeleteBranchField::Local if state.delete_branch_remote_available => {
            DeleteBranchField::Remote
        }
        DeleteBranchField::Local => DeleteBranchField::Force,
        DeleteBranchField::Remote => DeleteBranchField::Force,
        DeleteBranchField::Force => DeleteBranchField::Local,
    }
}

fn prev_field(state: &AppState) -> DeleteBranchField {
    if !state.delete_branch_remote_available {
        return DeleteBranchField::Force;
    }
    match state.delete_branch_field {
        DeleteBranchField::Local => DeleteBranchField::Force,
        DeleteBranchField::Remote => DeleteBranchField::Local,
        DeleteBranchField::Force if state.delete_branch_remote_available => {
            DeleteBranchField::Remote
        }
        DeleteBranchField::Force => DeleteBranchField::Local,
    }
}

fn toggle_current(state: &mut AppState) {
    match state.delete_branch_field {
        DeleteBranchField::Local if state.delete_branch_remote_available => {
            state.delete_branch_local = !state.delete_branch_local
        }
        DeleteBranchField::Local => {}
        DeleteBranchField::Remote if state.delete_branch_remote_available => {
            state.delete_branch_remote = !state.delete_branch_remote
        }
        DeleteBranchField::Remote => {}
        DeleteBranchField::Force => state.delete_branch_force = !state.delete_branch_force,
    }
}
