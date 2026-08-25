//! Drawing the workspace rows and the deployment status block.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::{
    config::BRANCH_MAIN,
    git::{Branch, NestedRepo, ReleaseEnv, ReleaseTargetStatus, RemoteBranch, Worktree},
    state::{AppState, SPINNER_FRAMES, clamp_index},
    ui,
};

use super::scroll;

use super::tree::{
    NestedRepoTreeRow, nested_repo_selected, nested_repo_tree_rows, root_repo_selected,
    worktree_selected,
};
use super::{
    ACTIVE_REPOSITORY_BG, DEPLOYMENT_STATUS_BASE_HEIGHT, MIN_REPOSITORY_TREE_WITH_DEPLOYMENT,
};

/// Height the deployment box needs in this checkout: only the deploy branches
/// that exist get a row.
pub(super) fn deployment_status_height(state: &AppState) -> u16 {
    DEPLOYMENT_STATUS_BASE_HEIGHT + u16::try_from(release_envs(state).len()).unwrap_or(0)
}

/// The deploy environments this checkout has, in promotion order.
fn release_envs(state: &AppState) -> Vec<(ReleaseEnv, String)> {
    [ReleaseEnv::Dev, ReleaseEnv::Test]
        .into_iter()
        .filter_map(|env| {
            state
                .release_branch(env)
                .map(|branch| (env, branch.to_string()))
        })
        .collect()
}

pub(super) fn render_deployment_status(state: &AppState, area: Rect, frame: &mut Frame) {
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
    for (env, branch) in release_envs(state) {
        let (status, color) = match env {
            ReleaseEnv::Dev => (state.current_branch_releases.develop.as_ref(), Color::Cyan),
            ReleaseEnv::Test => (state.current_branch_releases.test.as_ref(), Color::Yellow),
        };
        lines.push(env_line(
            &branch,
            status,
            color,
            state.animation_tick,
            release_status_loading(state),
        ));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub(super) fn render_nested_repositories(
    state: &AppState,
    area: Rect,
    frame: &mut Frame,
    focused: bool,
) {
    let deployment_height = deployment_status_height(state);
    let show_deployment = state.flow_available()
        && area.height >= deployment_height + MIN_REPOSITORY_TREE_WITH_DEPLOYMENT;
    let (tree_area, deployment_area) = if show_deployment {
        let chunks = Layout::vertical([
            Constraint::Min(MIN_REPOSITORY_TREE_WITH_DEPLOYMENT),
            Constraint::Length(deployment_height),
        ])
        .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };
    let rows = nested_repo_tree_rows(state);
    let len = rows.len();
    let selected_idx = clamp_index(state.nested_repo_tree_idx, len);
    let title = repositories_title(state);
    let block = ui::framed_with_activity(
        1,
        &title,
        focused,
        selected_idx.map(|idx| (idx + 1, len)),
        state.animation_tick,
        state.activity_label().is_some(),
    );
    let row_width = tree_area.width.saturating_sub(4) as usize;
    let items = rows
        .iter()
        .map(|row| match row {
            NestedRepoTreeRow::Root => {
                repository_list_item(root_repo_line(state, row_width), root_repo_selected(state))
            }
            NestedRepoTreeRow::Session { id } => match state.sessions.get(*id) {
                Some(session) => {
                    let shown = state.session_view() == Some(*id);
                    repository_list_item(session_line(session, row_width, shown), shown)
                }
                None => ListItem::new(Line::from("")),
            },
            NestedRepoTreeRow::Worktree { wt_idx } => {
                let worktree = &state.worktrees[*wt_idx];
                let active = worktree_selected(state, worktree);
                repository_list_item(worktree_line(worktree, row_width, active), active)
            }
            NestedRepoTreeRow::Repo { repo_idx } => {
                let repo = &state.nested_repositories[*repo_idx];
                let expanded = state
                    .nested_repositories
                    .get(*repo_idx)
                    .is_some_and(|repo| {
                        state.nested_repo_detail_path.as_deref() == Some(&repo.path)
                    });
                let active = nested_repo_selected(state, &repo.path);
                repository_list_item(nested_repo_line(repo, row_width, expanded, active), active)
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

pub(crate) fn nested_repo_scroll_offset(state: &AppState, area: Rect) -> usize {
    let len = nested_repo_tree_rows(state).len();
    scroll::selection_scroll_offset(
        clamp_index(state.nested_repo_tree_idx, len),
        len,
        scroll::list_viewport_height(area.height),
        state.nested_repositories_scroll_offset,
    )
}

fn repository_list_item(line: Line<'static>, selected: bool) -> ListItem<'static> {
    let item = ListItem::new(line);
    if selected {
        item.style(Style::default().bg(ACTIVE_REPOSITORY_BG))
    } else {
        item
    }
}

/// Names the checkout every other panel is showing, so the active repository
/// is readable from the panel frame and not only from the highlighted row.
fn repositories_title(state: &AppState) -> String {
    let kind = if state.git_panes_visible() {
        "Repositories"
    } else {
        "Checkouts"
    };
    match active_repo_name(state) {
        Some(name) => format!("{kind} \u{2022} {name}"),
        None => kind.to_string(),
    }
}

fn active_repo_name(state: &AppState) -> Option<String> {
    let root = state.repo_root.as_deref()?;
    std::path::Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn nested_repo_line(
    repo: &NestedRepo,
    row_width: usize,
    expanded: bool,
    active: bool,
) -> Line<'static> {
    let branch = repo
        .branch
        .clone()
        .or_else(|| {
            repo.detached_at
                .as_ref()
                .map(|sha| format!("detached@{sha}"))
        })
        .unwrap_or_else(|| "unknown".to_string());
    let marker_width = 4 + if repo.has_changes { 2 } else { 0 };
    let branch_width = branch.chars().count().saturating_add(1);
    let max_path_width = row_width
        .saturating_sub(marker_width)
        .saturating_sub(branch_width);

    let mut spans = Vec::new();
    spans.push(Span::styled(
        if expanded { "\u{25be} " } else { "\u{25b8} " },
        Style::default().fg(Color::LightMagenta),
    ));
    spans.push(Span::styled(
        if active { "* " } else { "  " },
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
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
        if active {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        },
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

/// The colour of a running session's dot: green when it is ready for a command,
/// yellow while it works, red when it is blocked on a question.
fn activity_color(activity: crate::session::SessionActivity) -> Color {
    match activity {
        crate::session::SessionActivity::Idle => Color::Green,
        crate::session::SessionActivity::Working => Color::Yellow,
        crate::session::SessionActivity::NeedsInput => Color::Red,
    }
}

/// What to add after "claude" for an activity, or nothing when it is simply
/// ready — the row is narrow, and "ready" is the state that needs no words.
fn activity_word(activity: crate::session::SessionActivity) -> Option<&'static str> {
    match activity {
        crate::session::SessionActivity::Idle => None,
        crate::session::SessionActivity::Working => Some("working"),
        crate::session::SessionActivity::NeedsInput => Some("needs input"),
    }
}

/// A session under its checkout: whether it is running, what it is doing, and
/// whether it has said something since it was last looked at.
fn session_line(session: &crate::session::Session, row_width: usize, shown: bool) -> Line<'static> {
    let (glyph, glyph_color) = match &session.status {
        crate::session::SessionStatus::Ended(_) => ("\u{25cb} ", Color::DarkGray),
        crate::session::SessionStatus::Running => ("\u{25cf} ", activity_color(session.activity())),
    };
    // The dot says what it is doing; the word repeats it for anyone the colour
    // alone does not reach, and the caret still marks output nobody has read.
    let state_text = match &session.status {
        crate::session::SessionStatus::Ended(notice) => format!("claude {notice}"),
        crate::session::SessionStatus::Running => {
            let mut text = "claude".to_string();
            if let Some(word) = activity_word(session.activity()) {
                text.push_str(" \u{b7} ");
                text.push_str(word);
            }
            if session.attention {
                text.push_str(" \u{25b4}");
            }
            text
        }
    };
    let mut spans = vec![
        Span::styled("    ", Style::default()),
        Span::styled(glyph, Style::default().fg(glyph_color)),
    ];
    spans.push(Span::styled(
        truncate_chars(&state_text, row_width.saturating_sub(6)),
        if shown {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        },
    ));
    if session.sandboxed && row_width > state_text.chars().count() + 16 {
        spans.push(Span::styled(
            " sandboxed",
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

/// One checkout of the active repository: what is checked out there, which
/// directory it is, and whether it can be entered at all.
fn worktree_line(worktree: &Worktree, row_width: usize, active: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled("  \u{2387} ", Style::default().fg(Color::LightMagenta)),
        Span::styled(
            if active { "* " } else { "  " },
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if worktree.has_changes {
        spans.push(Span::styled(
            "! ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let note = worktree_note(worktree);
    let landing = landing_marker(worktree);
    let used = 4
        + if worktree.has_changes { 2 } else { 0 }
        + landing
            .as_ref()
            .map_or(0, |(text, _)| text.chars().count() + 1);
    let note_width = note.as_ref().map_or(0, |note| note.chars().count() + 1);
    let label = worktree.label();
    let label_width = row_width
        .saturating_sub(used)
        .saturating_sub(note_width)
        .saturating_sub(worktree.dir_name().chars().count() + 1);

    spans.push(Span::styled(
        truncate_chars(&label, label_width.max(4)),
        Style::default()
            .fg(if worktree.branch.is_some() {
                Color::Green
            } else {
                Color::LightMagenta
            })
            .add_modifier(Modifier::BOLD),
    ));
    if let Some((text, color)) = landing {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(text, Style::default().fg(color)));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        truncate_chars(&worktree.dir_name(), row_width.saturating_sub(used + 1)),
        Style::default().fg(Color::DarkGray),
    ));
    if let Some(note) = note {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(note, Style::default().fg(Color::Red)));
    }
    Line::from(spans)
}

/// What landing this worktree would move, so `m` is legible before it is
/// pressed: how many commits main is missing, or that it is already merged and
/// only the cleanup is left.
fn landing_marker(worktree: &Worktree) -> Option<(String, Color)> {
    match worktree.unmerged? {
        0 => Some(("merged".to_string(), Color::DarkGray)),
        n => Some((format!("\u{2191}{n}"), Color::Cyan)),
    }
}

/// Why a worktree cannot simply be entered, when that is the case.
fn worktree_note(worktree: &Worktree) -> Option<String> {
    if worktree.is_missing() {
        return Some("missing".to_string());
    }
    if worktree.locked.is_some() {
        return Some("locked".to_string());
    }
    None
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
    label: &str,
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
            label.to_string(),
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
