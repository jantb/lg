use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::{
    app,
    config::{BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST},
    git::{Branch, NestedRepo, ReleaseTargetStatus, RemoteBranch},
    state::{AppState, BranchView, SPINNER_FRAMES, clamp_index},
    ui,
};

use super::scroll;

const DEPLOYMENT_STATUS_HEIGHT: u16 = 6;
const MIN_REPOSITORY_TREE_WITH_DEPLOYMENT: u16 = 6;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if !state.nested_repositories.is_empty() || !active_repo_is_workspace(state) {
        render_nested_repositories(state, area, frame, focused);
        return;
    }

    if !state.flow_available() {
        frame.render_widget(Paragraph::new(""), area);
        return;
    }

    render_deployment_status(state, area, frame);
}

fn render_deployment_status(state: &AppState, area: Rect, frame: &mut Frame) {
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
        BRANCH_MAIN,
        state.current_branch_releases.main.as_ref(),
        Color::Magenta,
        state.animation_tick,
        release_status_loading(state),
    ));
    lines.push(env_line(
        BRANCH_DEV,
        state.current_branch_releases.develop.as_ref(),
        Color::Cyan,
        state.animation_tick,
        release_status_loading(state),
    ));
    lines.push(env_line(
        BRANCH_TEST,
        state.current_branch_releases.test.as_ref(),
        Color::Yellow,
        state.animation_tick,
        release_status_loading(state),
    ));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_nested_repositories(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let show_deployment = state.flow_available()
        && area.height >= DEPLOYMENT_STATUS_HEIGHT + MIN_REPOSITORY_TREE_WITH_DEPLOYMENT;
    let (tree_area, deployment_area) = if show_deployment {
        let chunks = Layout::vertical([
            Constraint::Min(MIN_REPOSITORY_TREE_WITH_DEPLOYMENT),
            Constraint::Length(DEPLOYMENT_STATUS_HEIGHT),
        ])
        .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };
    let rows = nested_repo_tree_rows(state);
    let len = rows.len();
    let selected_idx = clamp_index(state.nested_repo_tree_idx, len);
    let block = ui::framed_with_activity(
        1,
        "Repositories",
        focused,
        selected_idx.map(|idx| (idx + 1, len)),
        state.animation_tick,
        state.activity_label().is_some(),
    );
    let row_width = tree_area.width.saturating_sub(4) as usize;
    let items = rows
        .iter()
        .map(|row| match row {
            NestedRepoTreeRow::Root => ListItem::new(root_repo_line(state, row_width)),
            NestedRepoTreeRow::Repo { repo_idx } => {
                let expanded = state
                    .nested_repositories
                    .get(*repo_idx)
                    .is_some_and(|repo| {
                        state.nested_repo_detail_path.as_deref() == Some(&repo.path)
                    });
                ListItem::new(nested_repo_line(
                    &state.nested_repositories[*repo_idx],
                    row_width,
                    expanded,
                ))
            }
            NestedRepoTreeRow::Branch { branch_idx, .. } => ListItem::new(nested_branch_line(
                &state.nested_repo_branches[*branch_idx],
                row_width,
            )),
            NestedRepoTreeRow::Remote { branch_idx, .. } => {
                ListItem::new(nested_remote_branch_line(
                    state
                        .visible_nested_repo_remote_branches()
                        .nth(*branch_idx)
                        .expect("visible remote row index"),
                    row_width,
                ))
            }
        })
        .collect::<Vec<_>>();
    let offset = nested_repo_scroll_offset(state, tree_area);
    let mut list_state = scroll::list_state(focused.then_some(selected_idx).flatten(), offset);
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");
    frame.render_stateful_widget(list, tree_area, &mut list_state);
    if let Some(area) = deployment_area {
        render_deployment_status(state, area, frame);
    }
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    let len = nested_repo_tree_rows(state).len();
    let selected_idx = clamp_index(state.nested_repo_tree_idx, len);
    state.nested_repositories_scroll_offset = scroll::selection_scroll_offset(
        selected_idx,
        len,
        scroll::list_viewport_height(area.height),
        state.nested_repositories_scroll_offset,
    );
}

pub fn handle_key(
    state: &mut AppState,
    key: ratatui::crossterm::event::KeyEvent,
) -> anyhow::Result<()> {
    use ratatui::crossterm::event::KeyCode;

    if state.nested_repositories.is_empty()
        && state.workspace_root.is_none()
        && state.repo_root.is_none()
    {
        return Ok(());
    }
    state.clamp();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => move_selection(state, true, 1),
        KeyCode::Char('k') | KeyCode::Up => move_selection(state, false, 1),
        KeyCode::Enter => match selected_tree_row(state) {
            Some(NestedRepoTreeRow::Root) => {
                state.pending_action =
                    Some(crate::state::PendingAction::SwitchRepository { path: None });
            }
            Some(NestedRepoTreeRow::Repo { repo_idx }) => {
                if let Some(path) = state
                    .nested_repositories
                    .get(repo_idx)
                    .map(|repo| repo.path.clone())
                {
                    state.pending_action = Some(crate::state::PendingAction::SwitchRepository {
                        path: Some(path.clone()),
                    });
                    if state.nested_repo_detail_path.as_deref() == Some(path.as_str()) {
                        state.nested_repo_tree_idx =
                            tree_idx_for_repo_path(state, &path).unwrap_or(0);
                    } else {
                        open_nested_repo_detail(state, path);
                    }
                }
            }
            Some(NestedRepoTreeRow::Branch {
                repo_idx,
                branch_idx,
            }) => {
                if let (Some(repo), Some(branch)) = (
                    state.nested_repositories.get(repo_idx),
                    state.nested_repo_branches.get(branch_idx),
                ) {
                    app::checkout_nested_branch_async(
                        state,
                        repo.path.clone(),
                        branch.name.clone(),
                    );
                }
            }
            Some(NestedRepoTreeRow::Remote {
                repo_idx,
                branch_idx,
            }) => {
                if let Some(repo) = state.nested_repositories.get(repo_idx) {
                    let branch = state
                        .visible_nested_repo_remote_branches()
                        .nth(branch_idx)
                        .map(|branch| branch.name.clone());
                    if let Some(branch) = branch {
                        app::checkout_nested_remote_branch_async(state, repo.path.clone(), branch);
                    }
                }
            }
            None => {}
        },
        KeyCode::Char('r') if state.nested_repo_detail_path.is_some() => {
            state.nested_repo_branch_view = match state.nested_repo_branch_view {
                BranchView::Local => BranchView::Remote,
                BranchView::Remote => BranchView::Local,
            };
            if let Some(path) = state.nested_repo_detail_path.clone() {
                let _ = load_nested_repo_detail(state, &path);
            }
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
            state.nested_repo_tree_idx = tree_idx_for_repo_path(state, &path).unwrap_or(0);
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
    if let Some(root) = state.workspace_root.as_deref() {
        let root = std::path::Path::new(root);
        state.nested_repo_branches = crate::git::nested_repo_branches_at(root, path)?;
        state.nested_repo_remote_branches = crate::git::nested_repo_remote_branches_at(root, path)?;
    } else {
        state.nested_repo_branches = crate::git::nested_repo_branches(path)?;
        state.nested_repo_remote_branches = crate::git::nested_repo_remote_branches(path)?;
    }
    state.clamp();
    Ok(())
}

pub(crate) fn close_nested_repo_detail(state: &mut AppState) {
    if let Some(path) = state.nested_repo_detail_path.take() {
        state.nested_repo_branches.clear();
        state.nested_repo_remote_branches.clear();
        state.nested_repo_branch_view = BranchView::Local;
        state.nested_repo_branches_idx = 0;
        state.nested_repo_remote_branches_idx = 0;
        state.nested_repo_tree_idx = tree_idx_for_repo_path(state, &path).unwrap_or(0);
    }
}

pub(crate) fn nested_repo_scroll_offset(state: &AppState, area: Rect) -> usize {
    let len = nested_repo_tree_rows(state).len();
    scroll::selection_scroll_offset(
        clamp_index(state.nested_repo_tree_idx, len),
        len,
        scroll::list_viewport_height(area.height),
        state.nested_repositories_scroll_offset,
    )
}

pub(crate) fn nested_repo_tree_len(state: &AppState) -> usize {
    nested_repo_tree_rows(state).len()
}

pub(crate) fn select_nested_repo_tree_row(state: &mut AppState, idx: usize) {
    state.nested_repo_tree_idx = idx;
    match selected_tree_row(state) {
        Some(
            NestedRepoTreeRow::Repo { repo_idx }
            | NestedRepoTreeRow::Branch { repo_idx, .. }
            | NestedRepoTreeRow::Remote { repo_idx, .. },
        ) => state.nested_repositories_idx = repo_idx,
        Some(NestedRepoTreeRow::Root) | None => {}
    }
}

fn move_selection(state: &mut AppState, down: bool, amount: usize) {
    let len = nested_repo_tree_rows(state).len();
    move_index(&mut state.nested_repo_tree_idx, len, down, amount);
    match selected_tree_row(state) {
        Some(
            NestedRepoTreeRow::Repo { repo_idx }
            | NestedRepoTreeRow::Branch { repo_idx, .. }
            | NestedRepoTreeRow::Remote { repo_idx, .. },
        ) => state.nested_repositories_idx = repo_idx,
        Some(NestedRepoTreeRow::Root) | None => {}
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NestedRepoTreeRow {
    Root,
    Repo { repo_idx: usize },
    Branch { repo_idx: usize, branch_idx: usize },
    Remote { repo_idx: usize, branch_idx: usize },
}

fn nested_repo_tree_rows(state: &AppState) -> Vec<NestedRepoTreeRow> {
    let mut rows = Vec::new();
    rows.push(NestedRepoTreeRow::Root);
    for (repo_idx, repo) in state.nested_repositories.iter().enumerate() {
        rows.push(NestedRepoTreeRow::Repo { repo_idx });
        if state.nested_repo_detail_path.as_deref() == Some(repo.path.as_str()) {
            match state.nested_repo_branch_view {
                BranchView::Local => {
                    rows.extend(state.nested_repo_branches.iter().enumerate().map(
                        |(branch_idx, _)| NestedRepoTreeRow::Branch {
                            repo_idx,
                            branch_idx,
                        },
                    ))
                }
                BranchView::Remote => {
                    rows.extend(state.visible_nested_repo_remote_branches().enumerate().map(
                        |(branch_idx, _)| NestedRepoTreeRow::Remote {
                            repo_idx,
                            branch_idx,
                        },
                    ))
                }
            }
        }
    }
    rows
}

fn selected_tree_row(state: &AppState) -> Option<NestedRepoTreeRow> {
    nested_repo_tree_rows(state)
        .get(state.nested_repo_tree_idx)
        .copied()
}

fn tree_idx_for_repo_path(state: &AppState, path: &str) -> Option<usize> {
    nested_repo_tree_rows(state)
        .iter()
        .position(|row| matches!(row, NestedRepoTreeRow::Repo { repo_idx } if state.nested_repositories.get(*repo_idx).is_some_and(|repo| repo.path == path)))
}

fn root_repo_selected(state: &AppState) -> bool {
    match (state.workspace_root.as_deref(), state.repo_root.as_deref()) {
        (Some(workspace), Some(repo)) => {
            std::path::Path::new(workspace) == std::path::Path::new(repo)
        }
        _ => true,
    }
}

fn active_repo_is_workspace(state: &AppState) -> bool {
    match (state.workspace_root.as_deref(), state.repo_root.as_deref()) {
        (Some(workspace), Some(repo)) => {
            std::path::Path::new(workspace) == std::path::Path::new(repo)
        }
        _ => true,
    }
}

fn nested_repo_line(repo: &NestedRepo, row_width: usize, expanded: bool) -> Line<'static> {
    let branch = repo
        .branch
        .clone()
        .or_else(|| {
            repo.detached_at
                .as_ref()
                .map(|sha| format!("detached@{sha}"))
        })
        .unwrap_or_else(|| "unknown".to_string());
    let marker_width = 2 + if repo.has_changes { 2 } else { 0 };
    let branch_width = branch.chars().count().saturating_add(1);
    let max_path_width = row_width
        .saturating_sub(marker_width)
        .saturating_sub(branch_width);

    let mut spans = Vec::new();
    spans.push(Span::styled(
        if expanded { "\u{25be} " } else { "\u{25b8} " },
        Style::default().fg(Color::LightMagenta),
    ));
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

fn root_repo_line(state: &AppState, row_width: usize) -> Line<'static> {
    let label = state
        .workspace_root
        .as_deref()
        .and_then(|root| std::path::Path::new(root).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let marker = if root_repo_selected(state) {
        "* "
    } else {
        "  "
    };
    Line::from(vec![
        Span::styled("\u{25b8} ", Style::default().fg(Color::LightMagenta)),
        Span::styled(
            format!(
                "{marker}{}",
                truncate_chars(label, row_width.saturating_sub(4))
            ),
            Style::default()
                .fg(if root_repo_selected(state) {
                    Color::Green
                } else {
                    Color::Gray
                })
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn nested_branch_line(branch: &Branch, row_width: usize) -> Line<'static> {
    let prefix = if branch.is_current { "  * " } else { "    " };
    let mut spans = vec![Span::styled(
        format!(
            "{prefix}{}",
            truncate_chars(&branch.name, row_width.saturating_sub(4))
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
            "    {}",
            truncate_chars(&branch.name, row_width.saturating_sub(4))
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
            let released_at = if s.released_at.is_empty() {
                "not merged".to_string()
            } else {
                s.released_at.clone()
            };
            spans.push(Span::styled(released_at, Style::default().fg(Color::Gray)));
            if s.missing_commits > 0 {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("+{}", s.missing_commits),
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
