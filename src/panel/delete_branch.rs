use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
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

fn modal_area(area: Rect) -> Rect {
    let w = (area.width * 6 / 10).clamp(56, 96).min(area.width);
    let h = 12u16.min(area.height);
    ui::centered(area, w, h)
}

/// The options in the order they are drawn, one a row.
fn option_rows(state: &AppState) -> Vec<Option<DeleteBranchField>> {
    if state.delete_branch.remote_available {
        vec![
            Some(DeleteBranchField::Local),
            Some(DeleteBranchField::Remote),
            Some(DeleteBranchField::Force),
        ]
    } else {
        // Local is not an option without a remote to choose instead.
        vec![None, Some(DeleteBranchField::Force)]
    }
}

/// A click on an option selects and toggles it, and one on the line of keys
/// under them does what that key does — Enter deletes with the options shown.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> Result<()> {
    if !super::pointer::left_click(m) {
        return Ok(());
    }
    let inner = ui::modal_inner(modal_area(area));
    let (chunks, _) = ui::modal_row_areas(
        inner,
        &[
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ],
    );
    if super::pointer::inside(chunks[1], m.column, m.row) {
        let at = usize::from(m.row - chunks[1].y);
        if let Some(Some(field)) = option_rows(state).get(at) {
            state.delete_branch.field = *field;
            toggle_current(state);
        }
    } else if m.row == chunks[2].y {
        let buttons = [
            ("j/k", None),
            (" move  ", None),
            ("Space", Some(KeyCode::Char(' '))),
            (" toggle  ", Some(KeyCode::Char(' '))),
            ("Enter", Some(KeyCode::Enter)),
            (" delete  ", Some(KeyCode::Enter)),
            ("Esc", Some(KeyCode::Esc)),
            (" cancel", Some(KeyCode::Esc)),
        ];
        if let Some(code) = super::pointer::button_at(chunks[2].x, &buttons, m.column) {
            handle_key(state, KeyEvent::from(code))?;
        }
    }
    Ok(())
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = modal_area(area);
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
            state.delete_branch.target.clone(),
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    frame.render_widget(Paragraph::new(header), chunks[0]);

    let mut body = if state.delete_branch.remote_available {
        vec![toggle_line(
            "delete local",
            state.delete_branch.local,
            state.delete_branch.field == DeleteBranchField::Local,
        )]
    } else {
        vec![Line::from(vec![
            Span::raw("  "),
            Span::styled("delete local branch", Style::default().fg(Color::Gray)),
        ])]
    };
    if state.delete_branch.remote_available {
        body.push(toggle_line(
            "delete remote (origin)",
            state.delete_branch.remote,
            state.delete_branch.field == DeleteBranchField::Remote,
        ));
    }
    body.extend([toggle_line(
        "force local delete (-D, for unmerged branches)",
        state.delete_branch.force,
        state.delete_branch.field == DeleteBranchField::Force,
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
            state.delete_branch.field = next_field(state);
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
            state.delete_branch.field = prev_field(state);
        }
        KeyCode::Char(' ') => toggle_current(state),
        KeyCode::Enter => {
            if !state.delete_branch.local && !state.delete_branch.remote {
                state.set_status("nothing selected to delete", true);
                return Ok(());
            }
            state.pending_action = Some(PendingAction::DeleteBranch {
                name: state.delete_branch.target.clone(),
                delete_local: state.delete_branch.local,
                delete_remote: state.delete_branch.remote,
                force: state.delete_branch.force,
            });
        }
        _ => {}
    }
    Ok(())
}

fn next_field(state: &AppState) -> DeleteBranchField {
    if !state.delete_branch.remote_available {
        return DeleteBranchField::Force;
    }
    match state.delete_branch.field {
        DeleteBranchField::Local if state.delete_branch.remote_available => {
            DeleteBranchField::Remote
        }
        DeleteBranchField::Local => DeleteBranchField::Force,
        DeleteBranchField::Remote => DeleteBranchField::Force,
        DeleteBranchField::Force => DeleteBranchField::Local,
    }
}

fn prev_field(state: &AppState) -> DeleteBranchField {
    if !state.delete_branch.remote_available {
        return DeleteBranchField::Force;
    }
    match state.delete_branch.field {
        DeleteBranchField::Local => DeleteBranchField::Force,
        DeleteBranchField::Remote => DeleteBranchField::Local,
        DeleteBranchField::Force if state.delete_branch.remote_available => {
            DeleteBranchField::Remote
        }
        DeleteBranchField::Force => DeleteBranchField::Local,
    }
}

fn toggle_current(state: &mut AppState) {
    match state.delete_branch.field {
        DeleteBranchField::Local if state.delete_branch.remote_available => {
            state.delete_branch.local = !state.delete_branch.local
        }
        DeleteBranchField::Local => {}
        DeleteBranchField::Remote if state.delete_branch.remote_available => {
            state.delete_branch.remote = !state.delete_branch.remote
        }
        DeleteBranchField::Remote => {}
        DeleteBranchField::Force => state.delete_branch.force = !state.delete_branch.force,
    }
}
