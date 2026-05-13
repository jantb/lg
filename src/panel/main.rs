use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Paragraph, Wrap},
};

use crate::{
    config::DIFF_PAGE,
    state::{AppState, DiffSource, Modal, PendingAction},
    ui,
};

mod review;
mod source;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if state.modal == Modal::ReviewChat {
        let chunks = review_chat_layout(state, area);
        render_main_content(state, chunks[0], frame, false);
        crate::panel::review_chat::render_docked(state, chunks[1], frame);
        return;
    }

    render_main_content(state, area, frame, focused);
}

fn render_main_content(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
        review::render(state, area, frame, focused);
        return;
    }

    let title = match state.diff_source {
        DiffSource::Review => "Review",
        DiffSource::Branch(_) => "Log",
        _ => "Diff",
    };
    let block = ui::framed_with_activity(
        0,
        title,
        focused,
        None,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let lines: Vec<ratatui::text::Line> = if matches!(state.diff_source, DiffSource::Branch(_)) {
        log_render_lines(&state.diff_text)
            .into_iter()
            .map(ui::highlight_log_line)
            .collect()
    } else {
        ui::highlight_diff_text(&state.diff_text)
    };

    let max_offset = max_scroll_offset(state);
    let offset = state.diff_offset.min(max_offset);

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));

    frame.render_widget(para, area);
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
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
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
        KeyCode::Char('o') => {
            if let Some(path) = selected_diff_open_path(state) {
                state.pending_action = Some(PendingAction::OpenFile(path));
            } else {
                state.set_status("no source file selected", false);
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn scroll(state: &mut AppState, scroll_down: bool, amount: u16) {
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
    state.diff_text.lines().count()
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
        Some("kt" | "kts" | "java" | "md" | "rs")
    )
}
