use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    git::ReleaseTargetStatus,
    state::{AppState, SPINNER_FRAMES},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    if !state.flow_available() {
        frame.render_widget(Paragraph::new(""), area);
        return;
    }

    let block = ui::bordered("Deployment Status");
    let mut lines = Vec::new();

    match state.branch.as_deref() {
        Some(branch) => lines.push(Line::from(vec![
            Span::styled("branch ", Style::default().fg(Color::DarkGray)),
            Span::styled(branch.to_string(), Style::default().fg(Color::Green)),
        ])),
        None => lines.push(Line::from(Span::styled(
            "detached HEAD",
            Style::default().fg(Color::Red),
        ))),
    }

    lines.push(env_line(
        "dev",
        state.current_branch_releases.develop.as_ref(),
        Color::Cyan,
        state.animation_tick,
        release_status_loading(state),
    ));
    lines.push(env_line(
        "test",
        state.current_branch_releases.test.as_ref(),
        Color::Yellow,
        state.animation_tick,
        release_status_loading(state),
    ));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn env_line(
    label: &'static str,
    status: Option<&ReleaseTargetStatus>,
    color: Color,
    tick: usize,
    loading: bool,
) -> Line<'static> {
    let marker = match status {
        Some(s) if s.missing_commits == 0 => "[x]",
        Some(_) => "[~]",
        None if loading => "[~]",
        None => "[ ]",
    };
    let mut spans = vec![
        Span::styled(marker, Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    match status {
        Some(s) => {
            spans.push(Span::styled(
                s.released_at.clone(),
                Style::default().fg(Color::Gray),
            ));
            if s.missing_commits > 0 {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("+{} pending", s.missing_commits),
                    Style::default().fg(Color::Red),
                ));
            }
        }
        None if loading => {
            let pulse = SPINNER_FRAMES[tick % SPINNER_FRAMES.len()];
            spans.push(Span::styled(
                format!("{pulse} checking"),
                Style::default().fg(Color::Gray),
            ));
        }
        None => {
            let pulse = if tick % 2 == 0 {
                SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
            } else {
                "-"
            };
            spans.push(Span::styled(
                format!("{pulse} not merged"),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    Line::from(spans)
}

fn release_status_loading(state: &AppState) -> bool {
    state
        .release_status_job
        .as_ref()
        .is_some_and(|job| Some(job.branch.as_str()) == state.branch.as_deref())
}
