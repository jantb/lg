use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

use crate::{
    app,
    git::{Branch, RemoteBranch},
    state::{AppState, BranchView, SPINNER_FRAMES, clamp_index},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let len = state.branch_list_len();
    let selected_idx = match state.branch_view {
        BranchView::Local => clamp_index(state.branches_idx, len),
        BranchView::Remote => clamp_index(state.remote_branches_idx, len),
    };
    let count = selected_idx.map(|idx| (idx + 1, len));
    let title = match state.branch_view {
        BranchView::Local => "Branches",
        BranchView::Remote => "Remote Branches",
    };
    let block = ui::framed_with_activity(
        3,
        title,
        focused,
        count,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let row_width = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = match state.branch_view {
        BranchView::Local => state
            .branches
            .iter()
            .map(|branch| ListItem::new(local_branch_line(state, branch, row_width)))
            .collect(),
        BranchView::Remote => state
            .visible_remote_branches()
            .map(|branch| ListItem::new(remote_branch_line(state, branch, row_width)))
            .collect(),
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    let mut list_state = ListState::default();
    if focused && let Some(idx) = selected_idx {
        list_state.select(Some(idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    state.clamp();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let len = state.branch_list_len();
            let idx = state.branch_list_idx_mut();
            *idx = idx.saturating_add(1).min(len.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let idx = state.branch_list_idx_mut();
            if *idx > 0 {
                *idx -= 1;
            }
        }
        KeyCode::Enter => {
            if state.checkout_job.is_none() {
                match state.branch_view {
                    BranchView::Local => {
                        if let Some(b) = state.branches.get(state.branches_idx)
                            && !b.is_current
                        {
                            app::checkout_branch_async(state, b.name.clone());
                        }
                    }
                    BranchView::Remote => {
                        let name = state
                            .visible_remote_branches()
                            .nth(state.remote_branches_idx)
                            .map(|branch| branch.name.clone());
                        if let Some(name) = name {
                            app::checkout_remote_branch_async(state, name);
                        }
                    }
                }
            }
        }
        KeyCode::Char('D') => {
            if state.branch_view == BranchView::Remote {
                state.set_status("delete remote branches from local branch view", false);
                return Ok(());
            }
            if let Some(b) = state.branches.get(state.branches_idx) {
                if b.is_current {
                    state.set_status("cannot delete the current branch", true);
                } else {
                    let snapshot = b.clone();
                    state.open_delete_branch_modal(&snapshot);
                }
            }
        }
        KeyCode::Char('r') => {
            state.branch_view = match state.branch_view {
                BranchView::Local => BranchView::Remote,
                BranchView::Remote => BranchView::Local,
            };
            state.clamp();
        }
        _ => {}
    }
    Ok(())
}

fn local_branch_line(state: &AppState, branch: &Branch, row_width: usize) -> Line<'static> {
    if state
        .checkout_job
        .as_ref()
        .is_some_and(|job| job.branch == branch.name)
    {
        let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
        let mut spans = vec![
            Span::styled(
                format!("{spinner} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                visible_local_branch_name(branch, 2, row_width),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        append_local_branch_status(&mut spans, branch);
        Line::from(spans)
    } else if branch.is_current {
        let mut spans = vec![Span::styled(
            format!("* {}", visible_local_branch_name(branch, 2, row_width)),
            current_branch_style(),
        )];
        append_local_branch_status(&mut spans, branch);
        Line::from(spans)
    } else {
        let mut spans = vec![Span::styled(
            format!("  {}", visible_local_branch_name(branch, 2, row_width)),
            Style::default(),
        )];
        append_local_branch_status(&mut spans, branch);
        Line::from(spans)
    }
}

fn remote_branch_line(state: &AppState, branch: &RemoteBranch, row_width: usize) -> Line<'static> {
    if state
        .checkout_job
        .as_ref()
        .is_some_and(|job| job.branch == branch.name)
    {
        let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
        let mut spans = vec![
            Span::styled(
                format!("{spinner} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                visible_remote_branch_name(branch, 2, row_width),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        append_branch_age(&mut spans, branch.last_commit_unix);
        Line::from(spans)
    } else {
        let mut spans = vec![Span::styled(
            format!("  {}", visible_remote_branch_name(branch, 2, row_width)),
            Style::default(),
        )];
        append_branch_age(&mut spans, branch.last_commit_unix);
        Line::from(spans)
    }
}

fn visible_local_branch_name(branch: &Branch, prefix_width: usize, row_width: usize) -> String {
    let status_width = local_branch_status_width(branch);
    let max_name_width = row_width
        .saturating_sub(prefix_width)
        .saturating_sub(status_width);
    truncate_chars(&branch.name, max_name_width)
}

fn visible_remote_branch_name(
    branch: &RemoteBranch,
    prefix_width: usize,
    row_width: usize,
) -> String {
    let status_width = branch_age_width(branch.last_commit_unix);
    let max_name_width = row_width
        .saturating_sub(prefix_width)
        .saturating_sub(status_width);
    truncate_chars(&branch.name, max_name_width)
}

fn append_local_branch_status(spans: &mut Vec<Span<'static>>, branch: &Branch) {
    if branch.upstream_gone {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "(upstream gone)",
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ));
    } else if branch.upstream.is_some() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "\u{2713}",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));
    }
    append_branch_age(spans, branch.last_commit_unix);
}

fn local_branch_status_width(branch: &Branch) -> usize {
    let remote_width = if branch.upstream_gone {
        " (upstream gone)".chars().count()
    } else if branch.upstream.is_some() {
        " \u{2713}".chars().count()
    } else {
        0
    };
    remote_width + branch_age_width(branch.last_commit_unix)
}

fn append_branch_age(spans: &mut Vec<Span<'static>>, last_commit_unix: Option<i64>) {
    if let Some(age) = branch_age_label(last_commit_unix) {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(age, Style::default().fg(Color::DarkGray)));
    }
}

fn branch_age_width(last_commit_unix: Option<i64>) -> usize {
    branch_age_label(last_commit_unix)
        .map(|age| 1 + age.chars().count())
        .unwrap_or(0)
}

fn branch_age_label(last_commit_unix: Option<i64>) -> Option<String> {
    let then = last_commit_unix?;
    let seconds = chrono::Utc::now().timestamp().saturating_sub(then).max(0);
    Some(if seconds < 60 {
        "now".to_string()
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h", seconds / (60 * 60))
    } else if seconds < 60 * 60 * 24 * 30 {
        format!("{}d", seconds / (60 * 60 * 24))
    } else if seconds < 60 * 60 * 24 * 365 {
        format!("{}mo", seconds / (60 * 60 * 24 * 30))
    } else {
        format!("{}y", seconds / (60 * 60 * 24 * 365))
    })
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

fn current_branch_style() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}
