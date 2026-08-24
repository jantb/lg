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
    let mut spans = vec![
        Span::styled(
            name,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled(root.to_string(), Style::default().fg(Color::DarkGray)),
    ];
    if let Some(badge) = session_badge(state) {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(badge);
    }
    Line::from(spans)
}

/// Sessions running elsewhere are easy to forget about, so the header says how
/// many there are and how many have said something unread.
fn session_badge(state: &AppState) -> Option<Span<'static>> {
    let count = state.sessions.len();
    if count == 0 {
        return None;
    }
    let plural = if count == 1 { "session" } else { "sessions" };
    let waiting = state.sessions.attention_count();
    let (text, color) = if waiting > 0 {
        (
            format!("{count} {plural} \u{b7} {waiting} waiting"),
            Color::Yellow,
        )
    } else {
        (format!("{count} {plural}"), Color::Green)
    };
    Some(Span::styled(
        text,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
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
