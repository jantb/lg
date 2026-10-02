use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    state::{AppState, Modal, PendingAction, SPINNER_FRAMES},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = modal_area(area);

    let diverged = state.branch_diverged_from_remote();
    let text = if let Some(job) = &state.push_job {
        let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
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
            if force_push_offered(branch) {
                Line::from(vec![
                    Span::styled("f", Style::default().fg(Color::Red)),
                    Span::raw(format!(
                        " force-push with lease, overwriting {behind} remote commit{}",
                        if behind == 1 { "" } else { "s" }
                    )),
                ])
            } else {
                Line::from(Span::styled(
                    "protected branch: no force push",
                    Style::default().fg(Color::DarkGray),
                ))
            },
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
    let inner = ui::modal_frame(frame, modal, title);
    frame.render_widget(Paragraph::new(text), inner);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &[], frame);
    }
}

/// Where the push modal sits in `area`.
fn modal_area(area: Rect) -> Rect {
    ui::centered(area, 64.min(area.width), 9.min(area.height))
}

/// A click on one of the options the modal draws does what its key does:
/// `Enter push` or `Enter merge upstream`, `Esc cancel`, and the force-push
/// line when it is offered, which asks first as `f` does.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> Result<()> {
    if !super::pointer::left_click(m) || state.push_job.is_some() {
        return Ok(());
    }
    let inner = ui::modal_inner(modal_area(area));
    let diverged = state.branch_diverged_from_remote();
    let enter_label = if diverged { " merge upstream" } else { " push" };
    let buttons = [
        ("Enter", Some(KeyCode::Enter)),
        (enter_label, Some(KeyCode::Enter)),
        ("    ", None),
        ("Esc", Some(KeyCode::Esc)),
        (" cancel", Some(KeyCode::Esc)),
    ];
    let code = if m.row == inner.y + 3 {
        super::pointer::button_at(inner.x, &buttons, m.column)
    } else if diverged
        && m.row == inner.y + 4
        && state.branch.as_deref().is_some_and(force_push_offered)
        && super::pointer::inside(inner, m.column, m.row)
    {
        Some(KeyCode::Char('f'))
    } else {
        None
    };
    if let Some(code) = code {
        handle_key(state, KeyEvent::from(code))?;
    }
    Ok(())
}

/// Whether the push modal offers to force-push `branch`. lg never rewrites a
/// protected or deploy branch on the remote.
fn force_push_offered(branch: &str) -> bool {
    !crate::config::is_protected_branch_name(branch)
        && !crate::config::is_deploy_branch_name(branch)
}

/// Ask before pushing over the remote branch, naming it and what is lost
/// there. The plan carries the remote commit the prompt was shown against, so
/// the push is refused if anyone pushed in between.
fn confirm_force_push(state: &mut AppState) {
    let branch = state.branch.clone().unwrap_or_default();
    if !force_push_offered(&branch) {
        state.set_status(
            format!("{branch} is a protected branch; lg never force-pushes it"),
            true,
        );
        return;
    }
    let plan = match crate::git::force_push_plan() {
        Ok(plan) => plan,
        Err(err) => {
            state.set_status(format!("force push unavailable: {err}"), true);
            return;
        }
    };
    let remote_ref = plan.remote_ref();
    let count = plan.overwritten.len();
    let commits = if count == 1 { "commit" } else { "commits" };
    let mut detail = format!(
        "{count} {commits} on {remote_ref} that {} does not have would be overwritten:",
        plan.branch
    );
    for line in plan.overwritten.iter().take(5) {
        detail.push_str("\n  ");
        detail.push_str(line);
    }
    if count > 5 {
        detail.push_str(&format!("\n  \u{2026} and {} more", count - 5));
    }
    detail.push_str(&format!(
        "\nUses --force-with-lease: if {remote_ref} has moved since the last fetch, the push is refused."
    ));
    state.confirm_action(
        "Force push",
        format!("Force-push {} over {remote_ref}?", plan.branch),
        detail,
        PendingAction::ForcePushWithLease(Box::new(plan)),
    );
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
        KeyCode::Char('f') if state.branch_diverged_from_remote() => confirm_force_push(state),
        KeyCode::Esc => {
            state.modal = Modal::None;
        }
        _ => {}
    }
    Ok(())
}
