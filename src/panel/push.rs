use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::{
    state::{AppState, Modal, PendingAction, SPINNER_FRAMES},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = 60.min(area.width);
    let h = 8.min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);

    let diverged = state.branch_diverged_from_remote();
    let text = if let Some(job) = &state.push_job {
        let spinner = SPINNER_FRAMES[job.spinner % SPINNER_FRAMES.len()];
        vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    spinner,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  Pushing "),
                Span::styled(job.branch.clone(), Style::default().fg(Color::Yellow)),
                Span::raw(" \u{2192} "),
                Span::styled(job.remote.clone(), Style::default().fg(Color::Yellow)),
                Span::raw("\u{2026}"),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  please wait",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )),
        ]
    } else if diverged {
        let branch = state.branch.as_deref().unwrap_or("<unknown>");
        let (ahead, behind) = state.current_branch_ahead_behind().unwrap_or((0, 0));
        vec![
            Line::from(vec![
                Span::styled("Branch: ", Style::default().fg(Color::Yellow)),
                Span::raw(branch),
            ]),
            Line::from(vec![
                Span::styled("Diverged: ", Style::default().fg(Color::Yellow)),
                Span::raw(format!("\u{2191}{ahead} \u{2193}{behind}")),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Enter", Style::default().fg(Color::Green)),
                Span::raw(" merge upstream    "),
                Span::styled("Esc", Style::default().fg(Color::Gray)),
                Span::raw(" cancel"),
            ]),
        ]
    } else {
        let branch = state.branch.as_deref().unwrap_or("<unknown>");
        let remote = state.remote_url.as_deref().unwrap_or("<unknown>");
        vec![
            Line::from(vec![
                Span::styled("Branch: ", Style::default().fg(Color::Yellow)),
                Span::raw(branch),
            ]),
            Line::from(vec![
                Span::styled("Remote: ", Style::default().fg(Color::Yellow)),
                Span::raw(remote),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Enter", Style::default().fg(Color::Green)),
                Span::raw(" push    "),
                Span::styled("Esc", Style::default().fg(Color::Gray)),
                Span::raw(" cancel"),
            ]),
        ]
    };

    let title = if state.push_job.is_some() {
        "Push \u{2014} running"
    } else if diverged {
        "Push \u{2014} branch diverged"
    } else {
        "Push"
    };
    let para = Paragraph::new(text).block(ui::bordered(title));
    frame.render_widget(para, modal);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    // While a push is running, keys are swallowed until the job completes.
    if state.push_job.is_some() {
        return Ok(());
    }
    match key.code {
        KeyCode::Enter => {
            state.pending_action = Some(if state.branch_diverged_from_remote() {
                PendingAction::MergeUpstream
            } else {
                PendingAction::Push
            });
        }
        KeyCode::Esc => {
            state.modal = Modal::None;
        }
        _ => {}
    }
    Ok(())
}
