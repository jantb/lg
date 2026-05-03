use std::path::Path;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::state::AppState;

pub(super) fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let right_width = if area.width >= 60 {
        34
    } else if area.width >= 36 {
        20
    } else {
        0
    };
    let chunks =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).split(area);

    frame.render_widget(
        Paragraph::new(project_line(state)).alignment(Alignment::Left),
        chunks[0],
    );

    if right_width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                branch_text(state),
                Style::default().fg(Color::Gray),
            ))
            .alignment(Alignment::Right),
            chunks[1],
        );
    }
}

fn project_line(state: &AppState) -> Line<'static> {
    let Some(root) = state.repo_root.as_deref() else {
        return Line::from(Span::styled(
            "unknown project",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    };
    let name = project_name(root);
    Line::from(vec![
        Span::styled(
            name,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled(root.to_string(), Style::default().fg(Color::DarkGray)),
    ])
}

fn branch_text(state: &AppState) -> String {
    let branch = state.branch.as_deref().unwrap_or("no branch");
    match state.ahead_behind {
        Some((ahead, behind)) if ahead > 0 && behind > 0 => {
            format!("{branch}  ahead {ahead} behind {behind}")
        }
        Some((ahead, _)) if ahead > 0 => format!("{branch}  ahead {ahead}"),
        Some((_, behind)) if behind > 0 => format!("{branch}  behind {behind}"),
        _ => branch.to_string(),
    }
}

fn project_name(root: &str) -> String {
    Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(root)
        .to_string()
}
