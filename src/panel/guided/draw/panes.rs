//! How the side is shared on a file's step: the diff above, the model's read
//! below, and the chat beside the read while one runs.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::git::guided::{GuidedFile, TickedFinding};
use crate::panel::guided::{Guided, Mode};
use crate::state::{AppState, DiffViewMode};
use crate::ui;

use super::LABEL;
use super::commentary::draw_commentary;
use super::diff::diff_lines;
use super::final_review::{draw_final, final_areas, final_commentary_lines, final_diff_lines};
use super::overview::draw_overview;

/// The step on screen: its file's diff above, the model's read of it below.
pub(super) fn draw_step(
    state: &AppState,
    guided: &Guided,
    area: Rect,
    frame: &mut Frame,
    dividers: &mut Vec<Rect>,
) {
    let Some(step) = guided.step() else {
        if guided.on_final() {
            draw_final(state, guided, area, frame, dividers);
            return;
        }
        match super::chat_session(state) {
            Some(id) => {
                let (columns, gaps) = split_chat(frame, guided, area);
                dividers.extend(gaps);
                draw_overview(state, guided, columns[0], frame);
                draw_chat(state, guided, id, columns[1], frame);
            }
            None => draw_overview(state, guided, area, frame),
        }
        return;
    };
    let mode = state.diff_view_mode;
    let (parts, gaps) = step_areas(guided, step, area, mode);
    ui::draw_dividers(frame, &gaps);
    dividers.extend(gaps);
    draw_diff(guided, step, mode, parts[0], frame);
    match super::chat_session(state) {
        Some(id) => {
            let (columns, gaps) = split_chat(frame, guided, parts[1]);
            dividers.extend(gaps);
            draw_commentary(state, guided, step, columns[0], frame);
            draw_chat(state, guided, id, columns[1], frame);
        }
        None => draw_commentary(state, guided, step, parts[1], frame),
    }
}

/// The chat is left out: it takes the mouse for itself.
pub(in crate::panel::guided) fn selectable_areas(
    guided: &Guided,
    side: Rect,
    mode: DiffViewMode,
) -> Vec<Rect> {
    let Some(parts) = diff_and_below(guided, side, mode) else {
        return vec![commentary_area(guided, side)];
    };
    vec![parts[0], commentary_area(guided, parts[1])]
}

/// The narrowest the chat and the read beside it are drawn.
const MIN_CHAT_WIDTH: u16 = 24;
const MIN_READ_WIDTH: u16 = 20;

/// How wide the chat is when it shares `room` columns with the read: as
/// asked for, else three fifths, leaving the read its own minimum.
pub(in crate::panel::guided) fn chat_width(asked: Option<u16>, room: u16) -> u16 {
    let most = room.saturating_sub(MIN_READ_WIDTH + 1);
    asked
        .unwrap_or(room * 3 / 5)
        .min(most)
        .max(MIN_CHAT_WIDTH.min(most))
}

fn chat_split(guided: &Guided, room: u16) -> [Constraint; 2] {
    [
        Constraint::Min(0),
        Constraint::Length(chat_width(guided.chat_width, room)),
    ]
}

/// The read left, the chat right, and the divider between them drawn.
pub(super) fn split_chat(frame: &mut Frame, guided: &Guided, area: Rect) -> (Vec<Rect>, Vec<Rect>) {
    guided.chat_room.set(area.width);
    ui::modal_columns(frame, area, &chat_split(guided, area.width))
}

/// The diff and what sits below it, on a file's step or the final review.
fn diff_and_below(guided: &Guided, side: Rect, mode: DiffViewMode) -> Option<Vec<Rect>> {
    if guided.on_final() {
        return Some(final_areas(guided, side, mode).0);
    }
    let step = guided.step()?;
    Some(step_areas(guided, step, side, mode).0)
}

/// The model's read below the diff, or the overview, left of the chat
/// while one runs.
pub(super) fn commentary_area(guided: &Guided, below: Rect) -> Rect {
    if guided.chat.is_some() {
        ui::modal_column_areas(below, &chat_split(guided, below.width)).0[0]
    } else {
        below
    }
}

pub(in crate::panel::guided) fn chat_screen_area(
    guided: &Guided,
    side: Rect,
    mode: DiffViewMode,
) -> Option<Rect> {
    guided.chat?;
    // The overview has nothing above it: the chat shares the whole side.
    let below = diff_and_below(guided, side, mode).map_or(side, |parts| parts[1]);
    let (columns, _) = ui::modal_column_areas(below, &chat_split(guided, below.width));
    let chat = columns[1];
    Some(Rect {
        y: chat.y + 1,
        height: chat.height.saturating_sub(1),
        ..chat
    })
}

pub(super) fn draw_chat(
    state: &AppState,
    guided: &Guided,
    id: crate::session::SessionId,
    area: Rect,
    frame: &mut Frame,
) {
    let Some(session) = state.sessions.get(id) else {
        return;
    };
    let focused = guided.mode == Mode::Chat;
    let (title, hint) = if focused {
        (
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            "  Ctrl-] back to the review",
        )
    } else {
        (Style::default().fg(Color::Gray), "  C or click to type")
    };
    let file = guided.chat_file.as_deref().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("Claude \u{b7} {file}"), title),
            Span::styled(hint, LABEL),
        ])),
        Rect { height: 1, ..area },
    );
    let screen = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    crate::term::render_screen(session.screen(), screen, frame.buffer_mut());
    if focused
        && let Some((row, col)) = session.cursor_position()
        && row < screen.height
        && col < screen.width
    {
        frame.set_cursor_position((screen.x + col, screen.y + row));
    }
}

/// Where the diff is drawn: the file's, or on the final step the whole changeset.
pub(in crate::panel::guided) fn diff_area(
    guided: &Guided,
    side: Rect,
    mode: DiffViewMode,
) -> Option<Rect> {
    Some(diff_and_below(guided, side, mode)?[0])
}

/// The findings sit in the diff, so it gets most of the room.
pub(in crate::panel::guided) fn step_areas(
    guided: &Guided,
    step: &GuidedFile,
    area: Rect,
    mode: DiffViewMode,
) -> (Vec<Rect>, Vec<Rect>) {
    let rows = diff_lines(guided, step, mode, area.width).lines.len();
    split_for_diff(area, rows, diff_room(guided, area))
}

/// A running chat needs room to be read, so the diff gives up more of it.
pub(super) fn diff_room(guided: &Guided, area: Rect) -> u16 {
    if guided.chat.is_some() {
        area.height / 2
    } else {
        area.height * 2 / 3
    }
}

pub(super) fn split_for_diff(area: Rect, rows: usize, most: u16) -> (Vec<Rect>, Vec<Rect>) {
    let diff_height = u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .min(most)
        .max(area.height.min(6));
    ui::modal_row_areas(area, &[Constraint::Length(diff_height), Constraint::Min(3)])
}

fn draw_diff(
    guided: &Guided,
    step: &GuidedFile,
    mode: DiffViewMode,
    area: Rect,
    frame: &mut Frame,
) {
    let diff = diff_lines(guided, step, mode, area.width);
    let scroll = diff.scroll(area.height);
    frame.render_widget(Paragraph::new(diff.lines).scroll((scroll as u16, 0)), area);
}

pub(in crate::panel::guided) fn finding_at(
    state: &AppState,
    side: Rect,
    column: u16,
    row: u16,
) -> Option<TickedFinding> {
    let guided = state.guided.as_deref()?;
    let mode = state.diff_view_mode;
    use crate::panel::pointer::inside;
    let ticked_at = |findings: Vec<(std::ops::Range<usize>, TickedFinding)>, at: usize| {
        findings
            .into_iter()
            .find(|(rows, _)| rows.contains(&at))
            .map(|(_, finding)| finding)
    };
    if guided.on_final() {
        let (parts, _) = final_areas(guided, side, mode);
        let (diff, commentary) = (parts[0], commentary_area(guided, parts[1]));
        if inside(diff, column, row) {
            let lines = final_diff_lines(guided, mode, diff.width);
            let scroll = guided.final_scroll.get().min(guided.final_max.get());
            return ticked_at(lines.findings, usize::from(scroll + row - diff.y));
        }
        if inside(commentary, column, row) {
            let lines = final_commentary_lines(state, guided, commentary.width);
            let scroll = guided.side_scroll.min(guided.side_max.get());
            return ticked_at(lines.findings, usize::from(scroll + row - commentary.y));
        }
        return None;
    }
    let step = guided.step()?;
    let (parts, _) = step_areas(guided, step, side, mode);
    let area = parts[0];
    if !inside(area, column, row) {
        return None;
    }
    let diff = diff_lines(guided, step, mode, area.width);
    let at = diff.scroll(area.height) + usize::from(row - area.y);
    ticked_at(diff.findings, at)
}
