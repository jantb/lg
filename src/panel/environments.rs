use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    git::{NestedRepo, ReleaseTargetStatus},
    state::{AppState, SPINNER_FRAMES},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    if !state.nested_repositories.is_empty() {
        render_nested_repositories(state, area, frame);
        return;
    }

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
        "main",
        state.current_branch_releases.main.as_ref(),
        Color::Magenta,
        state.animation_tick,
        release_status_loading(state),
    ));
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

fn render_nested_repositories(state: &AppState, area: Rect, frame: &mut Frame) {
    let block = ui::bordered("Repositories");
    let inner = block.inner(area);
    let viewport_height = inner.height as usize;
    let row_width = inner.width as usize;
    let mut lines = Vec::new();
    for (shown, repo) in state
        .nested_repositories
        .iter()
        .take(viewport_height)
        .enumerate()
    {
        if shown + 1 == viewport_height && state.nested_repositories.len() > viewport_height {
            let remaining = state.nested_repositories.len().saturating_sub(shown);
            lines.push(Line::from(Span::styled(
                format!("{remaining} more..."),
                Style::default().fg(Color::DarkGray),
            )));
            break;
        }
        lines.push(nested_repo_line(repo, row_width));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn nested_repo_line(repo: &NestedRepo, row_width: usize) -> Line<'static> {
    let branch = repo
        .branch
        .clone()
        .or_else(|| {
            repo.detached_at
                .as_ref()
                .map(|sha| format!("detached@{sha}"))
        })
        .unwrap_or_else(|| "unknown".to_string());
    let marker_width = if repo.has_changes { 2 } else { 0 };
    let branch_width = branch.chars().count().saturating_add(1);
    let max_path_width = row_width
        .saturating_sub(marker_width)
        .saturating_sub(branch_width);

    let mut spans = Vec::new();
    if repo.has_changes {
        spans.push(Span::styled(
            "! ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        truncate_chars(&repo.path, max_path_width),
        Style::default().fg(Color::Gray),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        truncate_chars(&branch, row_width.saturating_sub(marker_width + 1)),
        Style::default()
            .fg(if repo.branch.is_some() {
                Color::Green
            } else {
                Color::LightMagenta
            })
            .add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
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

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() && max_chars > 0 {
        out.pop();
        out.push('\u{2026}');
    }
    out
}

fn release_status_loading(state: &AppState) -> bool {
    state
        .release_status_job
        .as_ref()
        .is_some_and(|job| Some(job.branch.as_str()) == state.branch.as_deref())
}
