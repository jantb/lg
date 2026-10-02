use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};

use crate::{
    app,
    config::is_protected_branch_name,
    git::{Branch, RemoteBranch},
    state::{AppState, BranchView, FlowAction, Pane, PendingAction, SPINNER_FRAMES, clamp_index},
    ui,
};

use super::scroll;

/// The rows on show and where the selection is among them: every branch of
/// the view, or the ones the `/` filter leaves.
fn shown_rows(state: &AppState) -> (Option<Vec<usize>>, Option<usize>, usize) {
    let len = state.branch_list_len();
    let idx = match state.branch_view {
        BranchView::Local => state.branches_list.idx,
        BranchView::Remote => state.remote_branches_list.idx,
    };
    match state.filtered_rows(Pane::Branches) {
        Some(rows) => {
            let (selected, len) = crate::state::visible_position(Some(&rows), idx, len);
            (Some(rows), selected, len)
        }
        None => (None, clamp_index(idx, len), len),
    }
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let (rows, selected_idx, len) = shown_rows(state);
    let count = selected_idx.map(|idx| (idx + 1, len));
    let title = match state.branch_view {
        BranchView::Local => "Branches",
        BranchView::Remote => "Remote Branches",
    };
    let title = format!("{title}{}", state.filter_title(Pane::Branches));
    let block = ui::framed_with_activity(
        3,
        &title,
        focused,
        count,
        state.animation_ms,
        state.activity_label().is_some(),
    );

    let row_width = area.width.saturating_sub(4) as usize;
    // Only the rows on screen are turned into items, and the deploy
    // environments they are tagged with are read once for all of them.
    let offset = visible_scroll_offset(state, area);
    let window = scroll::visible_window(len, offset, area.height);
    // Which entry of the view each row on screen shows.
    let entry_at = |row: usize| {
        rows.as_ref()
            .map_or(Some(row), |rows| rows.get(row).copied())
    };
    let items: Vec<ListItem> = match state.branch_view {
        BranchView::Local => {
            let environments = crate::preferences::load().config.branches.environments;
            window
                .clone()
                .filter_map(entry_at)
                .filter_map(|at| state.branches.get(at))
                .map(|branch| {
                    ListItem::new(local_branch_line(state, branch, &environments, row_width))
                })
                .collect()
        }
        BranchView::Remote => {
            let remotes: Vec<&RemoteBranch> = state.visible_remote_branches().collect();
            window
                .clone()
                .filter_map(entry_at)
                .filter_map(|at| remotes.get(at).copied())
                .map(|branch| ListItem::new(remote_branch_line(state, branch, row_width)))
                .collect()
        }
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(crate::ui::palette::selection())
        .highlight_symbol("\u{203a} ");

    let mut list_state =
        scroll::window_list_state(focused.then_some(selected_idx).flatten(), &window);

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    let offset = visible_scroll_offset(state, area);
    *branch_scroll_offset_mut(state) = offset;
}

fn visible_scroll_offset(state: &AppState, area: Rect) -> usize {
    let (_, selected_idx, len) = shown_rows(state);
    scroll::selection_scroll_offset(
        selected_idx,
        len,
        scroll::list_viewport_height(area.height),
        branch_scroll_offset(state),
    )
}

pub(crate) fn branch_scroll_offset(state: &AppState) -> usize {
    match state.branch_view {
        BranchView::Local => state.branches_list.scroll,
        BranchView::Remote => state.remote_branches_list.scroll,
    }
}

fn branch_scroll_offset_mut(state: &mut AppState) -> &mut usize {
    match state.branch_view {
        BranchView::Local => &mut state.branches_list.scroll,
        BranchView::Remote => &mut state.remote_branches_list.scroll,
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    let configured_base = crate::preferences::base_branch();
    state.clamp();
    let shifted_m = shifted_char(key, 'm', 'M');
    let shifted_d = shifted_char(key, 'd', 'D');
    // With nothing the filter leaves, there is no branch to act on; moving
    // and switching views still work.
    if state.selection_hidden(Pane::Branches)
        && !matches!(
            key.code,
            KeyCode::Char('j' | 'k' | 'r') | KeyCode::Up | KeyCode::Down
        )
    {
        state.set_status(
            format!(
                "no branch matches /{} \u{2014} Esc clears the filter",
                state.branches_filter.text.trim()
            ),
            false,
        );
        return Ok(true);
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down if state.step_filtered(Pane::Branches, true, 1) => {}
        KeyCode::Char('k') | KeyCode::Up if state.step_filtered(Pane::Branches, false, 1) => {}
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
                        if let Some(b) = state.branches.get(state.branches_list.idx)
                            && !b.is_current
                        {
                            app::checkout_branch_async(state, b.name.clone());
                        }
                    }
                    BranchView::Remote => {
                        let name = state
                            .visible_remote_branches()
                            .nth(state.remote_branches_list.idx)
                            .map(|branch| branch.name.clone());
                        if let Some(name) = name {
                            app::checkout_remote_branch_async(state, name);
                        }
                    }
                }
            }
        }
        _ if shifted_d => {
            if state.branch_view == BranchView::Remote {
                state.set_status("delete remote branches from local branch view", false);
                return Ok(true);
            }
            if let Some(b) = state.branches.get(state.branches_list.idx) {
                if protected_branch(&b.name) {
                    state.set_status(format!("cannot delete protected branch {}", b.name), true);
                } else {
                    let snapshot = b.clone();
                    state.open_delete_branch_modal(&snapshot);
                }
            }
        }
        KeyCode::Char('d') if !shifted_d => {
            if state.branch_view == BranchView::Remote {
                state.set_status("delete remote branches from local branch view", false);
                return Ok(true);
            }
            let Some(branch) = state.branches.get(state.branches_list.idx) else {
                return Ok(true);
            };
            if protected_branch(&branch.name) {
                state.set_status(
                    format!("cannot delete protected branch {}", branch.name),
                    true,
                );
            } else if branch.upstream.is_some() && !branch.upstream_gone {
                state.set_status("branch has a remote; use D for delete options", false);
            } else {
                let snapshot = branch.clone();
                state.open_delete_branch_modal(&snapshot);
            }
        }
        KeyCode::Char('y') => {
            if let Some(name) = state.selected_branch_ref().map(str::to_owned) {
                match crate::git::full_sha(&name) {
                    Ok(sha) => super::commits::copy_sha(state, &sha),
                    Err(err) => state.set_status(format!("copy failed: {err}"), true),
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
        KeyCode::Char('o') => {
            state.pending_action = Some(PendingAction::OpenProject);
        }
        KeyCode::Char('u') => {
            queue_set_upstream(state);
        }
        KeyCode::Char('m') if !shifted_m => {
            if state.branch_view == BranchView::Remote {
                state.set_status("merge main from local branch view", false);
            } else if state.branch.as_deref() == Some(configured_base.as_str()) {
                if state.pull_available() {
                    state.pending_action = Some(PendingAction::Pull);
                } else {
                    state.set_status("main is not behind origin/main", false);
                }
            } else if !state.merge_main_available() {
                let status = match state.branch.as_deref() {
                    Some(branch) if state.is_deploy_branch(branch) => {
                        "current branch is not behind origin/main"
                    }
                    _ => "checkout a feature branch before merging main",
                };
                state.set_status(status, true);
            } else {
                confirm_merge_main(state, &configured_base);
            }
        }
        _ if shifted_m => {
            if state.branch_view == BranchView::Remote {
                state.set_status("sync local branches from local branch view", false);
            } else {
                confirm_merge_main_all_branches(state, &configured_base);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Merging main in ends in a push, so a single key does not get to start it:
/// it asks first, the way the same action does from the flow menu.
fn confirm_merge_main(state: &mut AppState, configured_base: &str) {
    let remote = crate::preferences::remote();
    let current = state.branch.clone().unwrap_or_default();
    state.confirm_action(
        "Merge Main",
        format!("Merge {remote}/{configured_base} into {current} and push it?"),
        format!(
            "Stashes local changes, updates {configured_base} from {remote}, merges \
             {remote}/{configured_base} into {current}, pushes {current}, then restores \
             the stash."
        ),
        PendingAction::Flow(FlowAction::MergeMain),
    );
}

/// Syncing every branch merges into and pushes branches that are not even
/// checked out, so the prompt names each one and what will happen to it.
fn confirm_merge_main_all_branches(state: &mut AppState, configured_base: &str) {
    let remote = crate::preferences::remote();
    let mut pushed = Vec::new();
    let mut local_only = Vec::new();
    for branch in &state.branches {
        if branch.name == configured_base || crate::git::is_safety_ref(&branch.name) {
            continue;
        }
        match branch.upstream.as_deref() {
            Some(upstream) if !branch.upstream_gone => {
                pushed.push(format!("  {} \u{2192} {upstream}", branch.name));
            }
            _ => local_only.push(format!("  {}", branch.name)),
        }
    }
    let count = pushed.len() + local_only.len();
    let mut detail = Vec::new();
    if count == 0 {
        detail.push(format!(
            "No other local branches; only {configured_base} is updated from {remote}."
        ));
    }
    if !pushed.is_empty() {
        detail.push("Merged, then pushed:".to_string());
        detail.extend(pushed);
    }
    if !local_only.is_empty() {
        detail.push("Merged locally only (no upstream):".to_string());
        detail.extend(local_only);
    }
    let plural = if count == 1 { "branch" } else { "branches" };
    state.confirm_action(
        "Sync All Branches",
        format!("Merge {remote}/{configured_base} into {count} local {plural} and push them?"),
        detail.join("\n"),
        PendingAction::MergeMainAllBranches,
    );
}

fn queue_set_upstream(state: &mut AppState) {
    let configured_remote = crate::preferences::remote();
    if state.branch_view == BranchView::Remote {
        state.set_status("set upstream from local branch view", false);
        return;
    }
    let Some(branch) = state.branches.get(state.branches_list.idx) else {
        return;
    };
    if branch.upstream.is_some() && !branch.upstream_gone {
        state.set_status(
            format!("{} already tracks a remote branch", branch.name),
            false,
        );
        return;
    }

    let matching = state
        .remote_branches
        .iter()
        .filter(|remote| remote.local_name == branch.name)
        .min_by_key(|remote| usize::from(remote.remote != configured_remote.as_str()));

    let Some(remote) = matching else {
        state.set_status(
            format!("no matching remote branch for {}", branch.name),
            true,
        );
        return;
    };

    state.pending_action = Some(PendingAction::SetBranchUpstream {
        branch: branch.name.clone(),
        upstream: remote.name.clone(),
    });
}

fn shifted_char(key: KeyEvent, lower: char, upper: char) -> bool {
    matches!(key.code, KeyCode::Char(c) if c == upper)
        || (matches!(key.code, KeyCode::Char(c) if c == lower)
            && key.modifiers.contains(KeyModifiers::SHIFT))
}

fn protected_branch(name: &str) -> bool {
    is_protected_branch_name(name)
}

fn local_branch_line(
    state: &AppState,
    branch: &Branch,
    environments: &[crate::preferences::Environment],
    row_width: usize,
) -> Line<'static> {
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
        for env in environments.iter().filter(|e| e.branch == branch.name) {
            spans.push(Span::styled(
                format!(" [{}]", env.name),
                Style::default().fg(Color::Magenta),
            ));
        }
        append_local_branch_status(&mut spans, branch);
        Line::from(spans)
    } else if branch.is_current {
        let mut spans = vec![Span::styled(
            format!("* {}", visible_local_branch_name(branch, 2, row_width)),
            current_branch_style(),
        )];
        for env in environments.iter().filter(|e| e.branch == branch.name) {
            spans.push(Span::styled(
                format!(" [{}]", env.name),
                Style::default().fg(Color::Magenta),
            ));
        }
        append_local_branch_status(&mut spans, branch);
        Line::from(spans)
    } else {
        let mut spans = vec![Span::styled(
            format!("  {}", visible_local_branch_name(branch, 2, row_width)),
            Style::default(),
        )];
        for env in environments.iter().filter(|e| e.branch == branch.name) {
            spans.push(Span::styled(
                format!(" [{}]", env.name),
                Style::default().fg(Color::Magenta),
            ));
        }
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
    } else if branch.ahead > 0 || branch.behind > 0 {
        append_tracking_counts(spans, branch);
    } else if branch.upstream.is_some() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "\u{2713}",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "no upstream",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
    }
    append_main_behind_count(spans, branch);
    append_branch_age(spans, branch.last_commit_unix);
}

fn local_branch_status_width(branch: &Branch) -> usize {
    let remote_width = if branch.upstream_gone {
        " (upstream gone)".chars().count()
    } else if branch.ahead > 0 || branch.behind > 0 {
        tracking_counts_width(branch)
    } else if branch.upstream.is_some() {
        " \u{2713}".chars().count()
    } else {
        " no upstream".chars().count()
    };
    remote_width + main_behind_width(branch) + branch_age_width(branch.last_commit_unix)
}

fn append_tracking_counts(spans: &mut Vec<Span<'static>>, branch: &Branch) {
    if branch.ahead > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("\u{2191}{}", branch.ahead),
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if branch.behind > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("\u{2193}{}", branch.behind),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
}

fn tracking_counts_width(branch: &Branch) -> usize {
    let mut width = 0;
    if branch.ahead > 0 {
        width += 1 + 1 + branch.ahead.to_string().chars().count();
    }
    if branch.behind > 0 {
        width += 1 + 1 + branch.behind.to_string().chars().count();
    }
    width
}

fn append_main_behind_count(spans: &mut Vec<Span<'static>>, branch: &Branch) {
    if branch.behind_main > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("main\u{2193}{}", branch.behind_main),
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ));
    }
}

fn main_behind_width(branch: &Branch) -> usize {
    if branch.behind_main > 0 {
        " main\u{2193}".chars().count() + branch.behind_main.to_string().chars().count()
    } else {
        0
    }
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
