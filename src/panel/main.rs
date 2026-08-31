use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Paragraph, Wrap},
};

use crate::{
    config::DIFF_PAGE,
    state::{AppState, DiffSource, DiffViewMode, Modal, PendingAction},
    ui,
};

mod review;
mod source;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if let Some(id) = state.session_view() {
        render_session(state, id, area, frame, focused);
        return;
    }

    if !state.git_panes_visible() {
        render_no_session(state, area, frame, focused);
        return;
    }

    if state.modal == Modal::ReviewChat {
        let chunks = review_chat_layout(state, area);
        render_main_content(state, chunks[0], frame, false);
        crate::panel::review_chat::render_docked(state, chunks[1], frame);
        return;
    }

    render_main_content(state, area, frame, focused);
}

/// Workspace mode with nothing running yet: say how to start something rather
/// than showing an empty frame.
fn render_no_session(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let block = ui::framed_with_activity(0, "Session", focused, None, state.animation_tick, false);
    let lines = vec![
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("  No session yet."),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("  Pick a checkout on the left and press s to start claude"),
        ratatui::text::Line::from("  in it, t for a terminal, or n to make a worktree first."),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("  F2 goes back to the git view; sessions keep running."),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Draw a running session: the program's own screen, framed like any pane, with
/// the real cursor placed on it while it holds the keyboard.
fn render_session(
    state: &AppState,
    id: crate::session::SessionId,
    area: Rect,
    frame: &mut Frame,
    focused: bool,
) {
    let Some(session) = state.sessions.get(id) else {
        return;
    };
    let title = session.title();
    let block = ui::framed_with_activity(
        0,
        &title,
        focused,
        None,
        state.animation_tick,
        session.attention,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    crate::term::render_screen(session.screen(), inner, frame.buffer_mut());

    if focused && state.session_capture {
        if let Some((row, col)) = session.cursor_position() {
            if row < inner.height && col < inner.width {
                frame.set_cursor_position((inner.x + col, inner.y + row));
            }
        }
    }
}

fn render_main_content(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        review::render(state, area, frame, focused);
        return;
    }

    let title = if matches!(state.diff_source, DiffSource::Review) {
        "Review"
    } else if matches!(state.diff_source, DiffSource::Branch(_)) {
        "Log"
    } else if side_by_side_diff_enabled(state) {
        "Diff: side-by-side"
    } else {
        "Diff"
    };
    let block = ui::framed_with_activity(
        0,
        title,
        focused,
        None,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let viewport_width = state.diff_viewport_width.max(area.width.saturating_sub(2));
    let lines: Vec<ratatui::text::Line> = if matches!(state.diff_source, DiffSource::Branch(_)) {
        log_render_lines(&state.diff_text)
            .into_iter()
            .map(ui::highlight_log_line)
            .collect()
    } else if side_by_side_diff_enabled(state) {
        ui::highlight_side_by_side_diff_text(&state.diff_text, viewport_width)
    } else {
        ui::highlight_diff_text_wrapped(&state.diff_text, viewport_width)
    };

    let max_offset = max_scroll_offset(state);
    let offset = state.diff_offset.min(max_offset);

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));

    frame.render_widget(para, area);
}

/// Keys for a session that is on screen but not holding the keyboard. The set
/// is deliberately tiny: take the keyboard back, close the session, or leave.
fn session_handle_key(
    state: &mut AppState,
    id: crate::session::SessionId,
    key: KeyEvent,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('i') | KeyCode::Enter => {
            if state
                .sessions
                .get(id)
                .is_some_and(|session| session.is_running())
            {
                state.session_capture = true;
            } else {
                state.set_status("this session has ended", true);
            }
        }
        KeyCode::Char('x') => {
            state.sessions.close(id);
            match state.sessions.focused() {
                Some(next) => state.show_session(next),
                None => state.show_diff(),
            }
        }
        KeyCode::Backspace => state.show_diff(),
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn review_chat_layout(state: &AppState, area: Rect) -> std::rc::Rc<[Rect]> {
    let min_review_height = 6.min(area.height);
    let desired_chat_height = state
        .review_chat_height
        .unwrap_or_else(|| (area.height / 3).clamp(8, 18));
    let chat_height = desired_chat_height.min(area.height.saturating_sub(min_review_height));
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(chat_height)])
        .split(area)
}
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    if let Some(id) = state.session_view() {
        return session_handle_key(state, id, key);
    }

    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        return review::handle_key(state, key);
    }

    let max_offset = max_scroll_offset(state);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            scroll(state, true, 1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            scroll(state, false, 1);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, true, DIFF_PAGE);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, false, DIFF_PAGE);
        }
        KeyCode::Char('g') => {
            state.diff_offset = 0;
        }
        KeyCode::Char('G') => {
            state.diff_offset = max_offset;
        }
        KeyCode::Char('v') if diff_view_toggle_available(state) => {
            state.diff_view_mode = match state.diff_view_mode {
                DiffViewMode::Unified => DiffViewMode::SideBySide,
                DiffViewMode::SideBySide => DiffViewMode::Unified,
            };
            state.diff_offset = state.diff_offset.min(max_scroll_offset(state));
            let label = match state.diff_view_mode {
                DiffViewMode::Unified => "unified diff",
                DiffViewMode::SideBySide => "side-by-side diff",
            };
            state.set_status(format!("showing {label}"), false);
        }
        KeyCode::Char('o') => {
            if let Some(path) = selected_diff_open_path(state) {
                state.pending_action = Some(PendingAction::OpenFile(path));
            } else {
                state.set_status("no source file selected", false);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// One wheel notch over the main pane, at a cell of it. A session whose
/// program asked about the mouse handles its own scrolling — a full-screen one
/// has to, since the alternate screen keeps no scrollback for lg to move.
/// Anything else falls back to [`scroll`].
pub fn wheel_at(state: &mut AppState, scroll_down: bool, amount: u16, column: u16, row: u16) {
    if let Some(id) = state.session_view()
        && let Some(session) = state.sessions.get_mut(id)
    {
        let mut sent = false;
        for _ in 0..amount.max(1) {
            sent = session.send_wheel(!scroll_down, column, row);
            if !sent {
                break;
            }
        }
        if sent {
            return;
        }
    }
    scroll(state, scroll_down, amount);
}

pub fn scroll(state: &mut AppState, scroll_down: bool, amount: u16) {
    // A session draws from its own scrollback, not from `diff_offset`.
    if let Some(id) = state.session_view() {
        if let Some(session) = state.sessions.get_mut(id) {
            session.scroll(!scroll_down, amount as usize);
        }
        return;
    }
    let max_offset = max_scroll_offset(state);
    let offset = state.diff_offset.min(max_offset);
    state.diff_offset = if scroll_down {
        offset.saturating_add(amount).min(max_offset)
    } else {
        offset.saturating_sub(amount)
    };
}

pub fn select_mouse_row(state: &mut AppState, area: Rect, row: u16) {
    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        review::select_mouse_row(state, area, row);
    }
}

pub fn max_scroll_offset(state: &AppState) -> u16 {
    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        return scroll_bound(review::render_line_count(state), state.diff_viewport_height);
    }
    scroll_bound(rendered_line_count(state), state.diff_viewport_height)
}

fn scroll_bound(line_count: usize, viewport_height: u16) -> u16 {
    line_count
        .min(u16::MAX as usize)
        .saturating_sub(viewport_height as usize) as u16
}

pub fn rendered_line_count(state: &AppState) -> usize {
    if state.diff_text.is_empty() {
        return state.diff_line_count as usize;
    }
    if matches!(state.diff_source, DiffSource::Branch(_)) {
        return wrapped_line_count(
            log_render_lines(&state.diff_text),
            state.diff_viewport_width,
        );
    }
    if side_by_side_diff_enabled(state) {
        return ui::side_by_side_diff_line_count(&state.diff_text, state.diff_viewport_width);
    }
    ui::diff_text_line_count(&state.diff_text, state.diff_viewport_width)
}

fn side_by_side_diff_enabled(state: &AppState) -> bool {
    state.diff_view_mode == DiffViewMode::SideBySide && diff_view_toggle_available(state)
}

fn diff_view_toggle_available(state: &AppState) -> bool {
    !matches!(
        state.diff_source,
        DiffSource::Branch(_) | DiffSource::Review
    )
}

fn wrapped_line_count<'a>(lines: impl IntoIterator<Item = &'a str>, viewport_width: u16) -> usize {
    let lines = lines.into_iter();
    if viewport_width == 0 {
        return lines.count();
    }
    let width = viewport_width.max(1) as usize;
    lines
        .map(|line| line.chars().count().max(1).div_ceil(width))
        .sum()
}

fn log_render_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() && !text.is_empty() {
        vec![text]
    } else {
        lines
    }
}

fn selected_diff_open_path(state: &AppState) -> Option<String> {
    match &state.diff_source {
        DiffSource::File(path) => Some(path.clone()),
        DiffSource::Review => review::selected_open_path(state),
        DiffSource::All | DiffSource::Folder(_) | DiffSource::Commit(_) => {
            diff_path_at_offset(&state.diff_text, state.diff_offset)
        }
        DiffSource::None | DiffSource::Branch(_) => None,
    }
}

fn diff_path_at_offset(diff_text: &str, offset: u16) -> Option<String> {
    let mut current = None;
    for line in diff_text.lines().take(offset as usize + 1) {
        if let Some(path) = diff_path_from_line(line) {
            current = Some(path);
        }
    }
    current.or_else(|| diff_text.lines().find_map(diff_path_from_line))
}

fn diff_path_from_line(line: &str) -> Option<String> {
    let path = line
        .strip_prefix("diff --git a/")
        .and_then(|rest| rest.split_once(" b/").map(|(_, path)| path))
        .or_else(|| line.strip_prefix("+++ b/"))
        .or_else(|| line.strip_prefix("--- a/"))?
        .trim();
    (path != "/dev/null" && is_supported_source_path(path)).then(|| path.to_string())
}

fn is_supported_source_path(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("kt" | "kts" | "java" | "md" | "rs" | "cs" | "csx")
    )
}
