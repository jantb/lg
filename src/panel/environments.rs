use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::{
    app,
    git::{Branch, NestedRepo, ReleaseTargetStatus, RemoteBranch},
    state::{AppState, BranchView, SPINNER_FRAMES, clamp_index},
    ui,
};

use super::scroll;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if !state.nested_repositories.is_empty() {
        render_nested_repositories(state, area, frame, focused);
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

fn render_nested_repositories(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if let Some(path) = state.nested_repo_detail_path.as_deref() {
        render_nested_repo_branches(state, path, area, frame, focused);
        return;
    }

    let len = state.nested_repositories.len();
    let selected_idx = clamp_index(state.nested_repositories_idx, len);
    let block = ui::framed_with_activity(
        1,
        "Repositories",
        focused,
        selected_idx.map(|idx| (idx + 1, len)),
        state.animation_tick,
        state.activity_label().is_some(),
    );
    let row_width = area.width.saturating_sub(4) as usize;
    let items = state
        .nested_repositories
        .iter()
        .map(|repo| ListItem::new(nested_repo_line(repo, row_width)))
        .collect::<Vec<_>>();
    let offset = nested_repo_scroll_offset(state, area);
    let mut list_state = scroll::list_state(focused.then_some(selected_idx).flatten(), offset);
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_nested_repo_branches(
    state: &AppState,
    path: &str,
    area: Rect,
    frame: &mut Frame,
    focused: bool,
) {
    let len = state.nested_repo_branch_list_len();
    let selected_idx = match state.nested_repo_branch_view {
        BranchView::Local => clamp_index(state.nested_repo_branches_idx, len),
        BranchView::Remote => clamp_index(state.nested_repo_remote_branches_idx, len),
    };
    let title = match state.nested_repo_branch_view {
        BranchView::Local => format!("{path} branches"),
        BranchView::Remote => format!("{path} remotes"),
    };
    let block = ui::framed_with_activity(
        1,
        &title,
        focused,
        selected_idx.map(|idx| (idx + 1, len)),
        state.animation_tick,
        state.activity_label().is_some(),
    );
    let row_width = area.width.saturating_sub(4) as usize;
    let items = match state.nested_repo_branch_view {
        BranchView::Local => state
            .nested_repo_branches
            .iter()
            .map(|branch| ListItem::new(nested_branch_line(branch, row_width)))
            .collect::<Vec<_>>(),
        BranchView::Remote => state
            .visible_nested_repo_remote_branches()
            .map(|branch| ListItem::new(nested_remote_branch_line(branch, row_width)))
            .collect::<Vec<_>>(),
    };
    let offset = nested_repo_branch_scroll_offset(state, area);
    let mut list_state = scroll::list_state(focused.then_some(selected_idx).flatten(), offset);
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");
    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    if state.nested_repo_detail_path.is_some() {
        let len = state.nested_repo_branch_list_len();
        let selected_idx = match state.nested_repo_branch_view {
            BranchView::Local => clamp_index(state.nested_repo_branches_idx, len),
            BranchView::Remote => clamp_index(state.nested_repo_remote_branches_idx, len),
        };
        let current = match state.nested_repo_branch_view {
            BranchView::Local => state.nested_repo_branches_scroll_offset,
            BranchView::Remote => state.nested_repo_remote_branches_scroll_offset,
        };
        let offset = scroll::selection_scroll_offset(
            selected_idx,
            len,
            scroll::list_viewport_height(area.height),
            current,
        );
        match state.nested_repo_branch_view {
            BranchView::Local => state.nested_repo_branches_scroll_offset = offset,
            BranchView::Remote => state.nested_repo_remote_branches_scroll_offset = offset,
        }
    } else {
        let len = state.nested_repositories.len();
        let selected_idx = clamp_index(state.nested_repositories_idx, len);
        state.nested_repositories_scroll_offset = scroll::selection_scroll_offset(
            selected_idx,
            len,
            scroll::list_viewport_height(area.height),
            state.nested_repositories_scroll_offset,
        );
    }
}

pub fn handle_key(
    state: &mut AppState,
    key: ratatui::crossterm::event::KeyEvent,
) -> anyhow::Result<()> {
    use ratatui::crossterm::event::KeyCode;

    if state.nested_repositories.is_empty() {
        return Ok(());
    }
    state.clamp();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => move_selection(state, true, 1),
        KeyCode::Char('k') | KeyCode::Up => move_selection(state, false, 1),
        KeyCode::Enter => {
            if let Some(path) = state.nested_repo_detail_path.clone() {
                if let Some(branch) = state.selected_nested_repo_branch_ref().map(str::to_owned) {
                    match state.nested_repo_branch_view {
                        BranchView::Local => app::checkout_nested_branch_async(state, path, branch),
                        BranchView::Remote => {
                            app::checkout_nested_remote_branch_async(state, path, branch)
                        }
                    }
                }
            } else if let Some(path) = state
                .nested_repositories
                .get(state.nested_repositories_idx)
                .map(|repo| repo.path.clone())
            {
                open_nested_repo_detail(state, path);
            }
        }
        KeyCode::Char('r') if state.nested_repo_detail_path.is_some() => {
            state.nested_repo_branch_view = match state.nested_repo_branch_view {
                BranchView::Local => BranchView::Remote,
                BranchView::Remote => BranchView::Local,
            };
            state.clamp();
        }
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('h') => {
            close_nested_repo_detail(state);
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn open_nested_repo_detail(state: &mut AppState, path: String) {
    match load_nested_repo_detail(state, &path) {
        Ok(()) => {
            state.nested_repo_detail_path = Some(path.clone());
            state.nested_repo_branch_view = BranchView::Local;
            state.nested_repo_branches_idx = state
                .nested_repo_branches
                .iter()
                .position(|branch| branch.is_current)
                .unwrap_or(0);
            state.nested_repo_remote_branches_idx = 0;
            state.set_status(format!("opened {path} branches"), false);
        }
        Err(err) => state.set_status(format!("load nested branches failed: {err}"), true),
    }
}

pub(crate) fn reload_nested_repo_detail(state: &mut AppState) {
    if let Some(path) = state.nested_repo_detail_path.clone()
        && let Err(err) = load_nested_repo_detail(state, &path)
    {
        state.set_status(format!("load nested branches failed: {err}"), true);
    }
}

fn load_nested_repo_detail(state: &mut AppState, path: &str) -> anyhow::Result<()> {
    state.nested_repo_branches = crate::git::nested_repo_branches(path)?;
    state.nested_repo_remote_branches = crate::git::nested_repo_remote_branches(path)?;
    state.clamp();
    Ok(())
}

pub(crate) fn close_nested_repo_detail(state: &mut AppState) {
    if state.nested_repo_detail_path.take().is_some() {
        state.nested_repo_branches.clear();
        state.nested_repo_remote_branches.clear();
        state.nested_repo_branch_view = BranchView::Local;
        state.nested_repo_branches_idx = 0;
        state.nested_repo_remote_branches_idx = 0;
    }
}

pub(crate) fn nested_repo_scroll_offset(state: &AppState, area: Rect) -> usize {
    scroll::selection_scroll_offset(
        clamp_index(
            state.nested_repositories_idx,
            state.nested_repositories.len(),
        ),
        state.nested_repositories.len(),
        scroll::list_viewport_height(area.height),
        state.nested_repositories_scroll_offset,
    )
}

pub(crate) fn nested_repo_branch_scroll_offset(state: &AppState, area: Rect) -> usize {
    let len = state.nested_repo_branch_list_len();
    let selected_idx = match state.nested_repo_branch_view {
        BranchView::Local => clamp_index(state.nested_repo_branches_idx, len),
        BranchView::Remote => clamp_index(state.nested_repo_remote_branches_idx, len),
    };
    let current = match state.nested_repo_branch_view {
        BranchView::Local => state.nested_repo_branches_scroll_offset,
        BranchView::Remote => state.nested_repo_remote_branches_scroll_offset,
    };
    scroll::selection_scroll_offset(
        selected_idx,
        len,
        scroll::list_viewport_height(area.height),
        current,
    )
}

fn move_selection(state: &mut AppState, down: bool, amount: usize) {
    if state.nested_repo_detail_path.is_some() {
        let len = state.nested_repo_branch_list_len();
        move_index(state.nested_repo_branch_list_idx_mut(), len, down, amount);
    } else {
        let len = state.nested_repositories.len();
        move_index(&mut state.nested_repositories_idx, len, down, amount);
    }
}

fn move_index(idx: &mut usize, len: usize, down: bool, amount: usize) {
    if len == 0 {
        *idx = 0;
    } else if down {
        *idx = idx.saturating_add(amount).min(len - 1);
    } else {
        *idx = idx.saturating_sub(amount);
    }
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

fn nested_branch_line(branch: &Branch, row_width: usize) -> Line<'static> {
    let prefix = if branch.is_current { "* " } else { "  " };
    let mut spans = vec![Span::styled(
        format!(
            "{prefix}{}",
            truncate_chars(&branch.name, row_width.saturating_sub(2))
        ),
        if branch.is_current {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        },
    )];
    if branch.ahead > 0 {
        spans.push(Span::styled(
            format!(" \u{2191}{}", branch.ahead),
            Style::default().fg(Color::Green),
        ));
    }
    if branch.behind > 0 {
        spans.push(Span::styled(
            format!(" \u{2193}{}", branch.behind),
            Style::default().fg(Color::Yellow),
        ));
    }
    Line::from(spans)
}

fn nested_remote_branch_line(branch: &RemoteBranch, row_width: usize) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(
            "  {}",
            truncate_chars(&branch.name, row_width.saturating_sub(2))
        ),
        Style::default().fg(Color::LightMagenta),
    )])
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
