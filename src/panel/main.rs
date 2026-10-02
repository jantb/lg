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

mod hunks;
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
    let block = ui::framed_with_activity(0, "Session", focused, None, state.animation_ms, false);
    let lines = vec![
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("  No session yet."),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("  Pick a checkout on the left and press s to start claude"),
        ratatui::text::Line::from("  in it, t for a terminal, or n to make a worktree first."),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("  w goes back to the git view; sessions keep running."),
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
        state.animation_ms,
        session.attention,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    crate::term::render_screen(session.screen(), inner, frame.buffer_mut());

    if focused
        && state.session_capture
        && let Some((row, col)) = session.cursor_position()
        && row < inner.height
        && col < inner.width
    {
        frame.set_cursor_position((inner.x + col, inner.y + row));
    }
}

fn render_main_content(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if matches!(state.diff_source, DiffSource::Review) && state.review.assisted.is_some() {
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
    // Where the hunk keys would act, while they can: the pane has the keyboard.
    let title = match focused.then(|| hunks::title_note(state)).flatten() {
        Some(note) => format!("{title} \u{b7} {note}"),
        None => title.to_string(),
    };
    let block = ui::framed_with_activity(
        0,
        &title,
        focused,
        None,
        state.animation_ms,
        state.activity_label().is_some(),
    );

    let viewport_width = state.diff_viewport_width.max(area.width.saturating_sub(2));
    let max_offset = max_scroll_offset(state);
    let offset = state.diff_offset.min(max_offset);

    let para = if matches!(state.diff_source, DiffSource::Branch(_)) {
        // The log is wrapped by the paragraph, so its rows are whole lines; the
        // ones above the window still count towards where it starts.
        let mut lines = with_diff_rows(state, viewport_width, <[_]>::to_vec);
        if focused
            && let Some(line) = hunks::cursor_log_line(state)
            && let Some(row) = lines.get_mut(line)
        {
            mark_cursor(row);
        }
        Paragraph::new(lines).scroll((offset, 0))
    } else {
        // Diff rows come pre-wrapped to the pane, one row per screen line, so
        // the window on screen is a slice of them.
        let mut rows = visible_diff_rows(
            state,
            viewport_width,
            offset as usize,
            area.height.saturating_sub(2) as usize,
        );
        if focused
            && hunks::hunks_shown(state)
            && let Some(row) = hunks::cursor_row(state, viewport_width)
            && let Some(row) = row
                .checked_sub(offset as usize)
                .and_then(|at| rows.get_mut(at))
        {
            mark_cursor(row);
        }
        Paragraph::new(rows)
    };
    let para = para.block(block).wrap(Wrap { trim: false });

    frame.render_widget(para, area);
}

/// Draw the hunk or commit cursor on its row.
fn mark_cursor(row: &mut ratatui::text::Line<'static>) {
    row.style = row
        .style
        .patch(crate::ui::palette::selection())
        .add_modifier(ratatui::style::Modifier::BOLD);
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
        KeyCode::Char('x') => crate::panel::environments::request_close_session(state, id, true),
        KeyCode::Backspace => state.show_diff(),
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn review_chat_layout(state: &AppState, area: Rect) -> std::rc::Rc<[Rect]> {
    let min_review_height = 6.min(area.height);
    let desired_chat_height = state
        .review
        .chat_height
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

    if matches!(state.diff_source, DiffSource::Review) && state.review.assisted.is_some() {
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
            hunks::follow_scroll(state);
        }
        KeyCode::Char('G') => {
            state.diff_offset = max_offset;
            hunks::follow_scroll(state);
        }
        KeyCode::Char(']') if hunks::hunks_shown(state) || hunks::log_shown(state) => {
            hunks::step(state, true);
        }
        KeyCode::Char('[') if hunks::hunks_shown(state) || hunks::log_shown(state) => {
            hunks::step(state, false);
        }
        KeyCode::Char(' ') if hunks::hunks_shown(state) => hunks::toggle_stage(state),
        KeyCode::Char('d') if hunks::hunks_shown(state) => hunks::discard(state),
        KeyCode::Char('C') if hunks::log_shown(state) => hunks::cherry_pick(state),
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
    if !matches!(state.diff_source, DiffSource::Review) {
        hunks::follow_scroll(state);
    }
}

/// Bring the hunk cursor and the scroll position back together after the
/// pane's text was read again.
pub fn settle_cursor(state: &mut AppState) {
    if hunks::hunks_shown(state) || hunks::log_shown(state) {
        hunks::settle(state);
    }
}

pub fn select_mouse_row(state: &mut AppState, area: Rect, row: u16) {
    if matches!(state.diff_source, DiffSource::Review) && state.review.assisted.is_some() {
        review::select_mouse_row(state, area, row);
    }
}

/// The highlighted diff rows from `offset` for `height` rows.
fn visible_diff_rows(
    state: &AppState,
    width: u16,
    offset: usize,
    height: usize,
) -> Vec<ratatui::text::Line<'static>> {
    with_diff_rows(state, width, |rows| {
        let start = offset.min(rows.len());
        let end = start.saturating_add(height).min(rows.len());
        rows[start..end].to_vec()
    })
}

/// Hand `f` the main pane's highlighted rows at `width`: a diff wrapped to the
/// pane one row per screen line, or the branch log a line per row. The whole
/// text is highlighted only when it, the width or the view has changed since
/// it last was — the rows on screen and the count that bounds scrolling both
/// come from the one highlighting pass.
fn with_diff_rows<R>(
    state: &AppState,
    width: u16,
    f: impl FnOnce(&[ratatui::text::Line<'static>]) -> R,
) -> R {
    let log_view = matches!(state.diff_source, DiffSource::Branch(_));
    let side_by_side = side_by_side_diff_enabled(state);
    let key = (
        crate::state::DiffRowCountKey {
            text_version: state.diff_text_version,
            // The log is not wrapped here, so its rows do not depend on width.
            viewport_width: if log_view { 0 } else { width },
            view_mode: if side_by_side {
                DiffViewMode::SideBySide
            } else {
                DiffViewMode::Unified
            },
            log_view,
        },
        state.diff_text.len(),
        state.diff_text.as_ptr() as usize,
    );
    let mut cache = state.diff_render_cache.borrow_mut();
    if cache.as_ref().is_none_or(|(cached, _)| *cached != key) {
        let rows = if log_view {
            log_render_lines(&state.diff_text)
                .into_iter()
                .map(|line| owned_line(ui::highlight_log_line(line)))
                .collect()
        } else if side_by_side {
            ui::highlight_side_by_side_diff_text(&state.diff_text, width)
        } else {
            ui::highlight_diff_text_wrapped(&state.diff_text, width)
        };
        *cache = Some((key, rows));
    }
    f(cache
        .as_ref()
        .map(|(_, rows)| rows.as_slice())
        .unwrap_or_default())
}

fn owned_line(line: ratatui::text::Line<'_>) -> ratatui::text::Line<'static> {
    ratatui::text::Line {
        spans: line
            .spans
            .into_iter()
            .map(|span| ratatui::text::Span::styled(span.content.into_owned(), span.style))
            .collect(),
        style: line.style,
        alignment: line.alignment,
    }
}

pub fn max_scroll_offset(state: &AppState) -> u16 {
    if matches!(state.diff_source, DiffSource::Review) && state.review.assisted.is_some() {
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
    let key = crate::state::DiffRowCountKey {
        text_version: state.diff_text_version,
        viewport_width: state.diff_viewport_width,
        view_mode: state.diff_view_mode,
        log_view: matches!(state.diff_source, DiffSource::Branch(_)),
    };
    if let Some((cached_key, count)) = state.diff_row_count_cache.get()
        && cached_key == key
    {
        return count;
    }
    let count = count_rendered_lines(state);
    state.diff_row_count_cache.set(Some((key, count)));
    count
}

fn count_rendered_lines(state: &AppState) -> usize {
    if matches!(state.diff_source, DiffSource::Branch(_)) {
        return wrapped_line_count(
            log_render_lines(&state.diff_text),
            state.diff_viewport_width,
        );
    }
    // The rows the pane draws, counted: highlighting them once serves both.
    with_diff_rows(state, state.diff_viewport_width, <[_]>::len)
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
    let path = match crate::git::patch::diff_git_paths(line) {
        Some((_, path)) => path,
        None => line
            .strip_prefix("+++ b/")
            .or_else(|| line.strip_prefix("--- a/"))?
            .to_string(),
    };
    let path = path.trim();
    (path != "/dev/null" && crate::language::is_source_path(path)).then(|| path.to_string())
}
