//! The mouse in a guided review: the step list, the panes beside it, the
//! chat, and the badges and buttons around them.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::state::AppState;

use super::chat::chat_session;
use super::found::toggle_ticks;
use super::load::switch_scope;
use super::navigate::{go_to, go_to_finding, move_cursor, scroll_final, switch_walk};
use super::progress::reset_review;
use super::{Mode, draw, keys};

/// The wheel over the step list scrolls it, over the diff moves its cursor
/// (or on the final step scrolls the whole changeset), and over the
/// commentary scrolls it (or the notes, in the summary). Only a click on a
/// step in the list goes to it, which is what has the model read it. The
/// divider beside the list drags to resize it, the scope badge switches
/// scope, and the reset button under the list resets the review on a second
/// click. None of it marks a step reviewed: that stays with the keys that
/// say so.
pub fn handle_mouse(state: &mut AppState, area: ratatui::layout::Rect, m: &MouseEvent) {
    use crate::panel::pointer;
    let Some((list, side)) = state
        .guided
        .as_deref()
        .and_then(|guided| draw::pane_areas(guided, area, state.guided_list_width))
    else {
        return;
    };
    if resize_list(state, list, m) {
        return;
    }
    if pointer::left_click(m) && button_click(state, area, list, m) {
        return;
    }
    if chat_mouse(state, side, m) {
        return;
    }
    if select_text(state, side, m) {
        return;
    }
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.typing() && !matches!(guided.mode, Mode::Summary { .. }) {
        return;
    }
    let browsing = matches!(guided.mode, Mode::Browse);
    let steps = draw::steps_area(list);
    match pointer::wheel(m) {
        Some(down) if pointer::inside(list, m.column, m.row) && browsing => {
            let (rows, first) = draw::row_targets(guided, steps.height);
            let last = rows.len().saturating_sub(usize::from(steps.height));
            let first = if down {
                (first + 3).min(last)
            } else {
                first.saturating_sub(3)
            };
            if let Some(guided) = state.guided.as_mut() {
                guided.list_scroll = Some(first);
            }
        }
        Some(down) if pointer::inside(side, m.column, m.row) => {
            let mode = state.diff_view_mode;
            let over_diff = browsing
                && draw::diff_area(guided, side, mode)
                    .is_some_and(|diff| pointer::inside(diff, m.column, m.row));
            match (over_diff, guided.on_final()) {
                (true, true) => scroll_final(state, if down { 3 } else { -3 }),
                // The file's diff scrolls with its cursor, as j and k move it.
                (true, false) => {
                    for _ in 0..3 {
                        move_cursor(state, down);
                    }
                }
                (false, _) => keys::scroll_side(state, if down { 3 } else { -3 }),
            }
        }
        Some(_) => {}
        None if pointer::left_click(m) && browsing && pointer::inside(side, m.column, m.row) => {
            // Ticking waits for the release: a drag selects text instead.
            let area = draw::selectable_areas(guided, side, state.diff_view_mode)
                .into_iter()
                .find(|area| pointer::inside(*area, m.column, m.row));
            state.selection =
                area.and_then(|area| crate::ui::TextSelection::within(area, m.column, m.row));
        }
        None if pointer::left_click(m) && browsing => {
            let (rows, first) = draw::row_targets(guided, steps.height);
            let target = pointer::list_row(steps, first, rows.len(), m.column, m.row)
                .and_then(|at| rows[at].clone());
            // The list stays where it was scrolled, under the mouse.
            let scrolled = Some(first);
            match target {
                Some(draw::RowTarget::Step(step)) => go_to(state, step, false),
                Some(draw::RowTarget::Finding(open)) => go_to_finding(state, &open),
                Some(draw::RowTarget::Concern(_)) | None => {}
            }
            if let Some(guided) = state.guided.as_mut() {
                guided.list_scroll = scrolled;
            }
        }
        None => {}
    }
}

/// The scope badge and the reset button. Any other click disarms the reset.
/// Returns whether the click went to one of them.
fn button_click(
    state: &mut AppState,
    area: ratatui::layout::Rect,
    list: ratatui::layout::Rect,
    m: &MouseEvent,
) -> bool {
    use crate::panel::pointer;
    let Some(guided) = state.guided.as_deref_mut() else {
        return false;
    };
    if guided.mode != Mode::Browse {
        return false;
    }
    if pointer::inside(draw::reset_button_area(list), m.column, m.row) {
        reset_review(state);
        return true;
    }
    guided.reset_armed = false;
    let on = |badge: Option<ratatui::layout::Rect>| {
        badge.is_some_and(|badge| pointer::inside(badge, m.column, m.row))
    };
    if on(draw::scope_badge_area(guided, area)) {
        switch_scope(state);
        return true;
    }
    if on(draw::walk_badge_area(guided, area)) {
        switch_walk(state);
        return true;
    }
    false
}

/// A press and release that never moved is a click, which ticks the finding under it.
fn select_text(state: &mut AppState, side: ratatui::layout::Rect, m: &MouseEvent) -> bool {
    match m.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            match state
                .selection
                .as_mut()
                .filter(|selection| selection.dragging)
            {
                Some(selection) => {
                    selection.extend(m.column, m.row);
                    true
                }
                None => false,
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let Some(pressed) = state.selection.take() else {
                return false;
            };
            state.selection = pressed.release();
            let clicked = state.selection.is_none();
            let finding = state
                .guided
                .as_deref()
                .filter(|guided| clicked && guided.mode == Mode::Browse)
                .and_then(|_| draw::finding_at(state, side, m.column, m.row));
            if let Some(finding) = finding {
                toggle_ticks(state, vec![finding]);
                return true;
            }
            let target = state
                .guided
                .as_deref()
                .filter(|guided| clicked && guided.mode == Mode::Browse)
                .and_then(|_| draw::overview_target_at(state, side, m.column, m.row));
            match target {
                Some(draw::OverviewTarget::Finding(open)) => go_to_finding(state, &open),
                Some(draw::OverviewTarget::File(step)) => go_to(state, step, false),
                None => {}
            }
            true
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            state.selection = None;
            false
        }
        _ => false,
    }
}

/// A click in the chat gives it the keyboard and starts a selection kept to
/// its screen; a click elsewhere while the chat has the keyboard takes it back.
fn chat_mouse(state: &mut AppState, side: ratatui::layout::Rect, m: &MouseEvent) -> bool {
    use crate::panel::pointer;
    let Some(id) = chat_session(state) else {
        return false;
    };
    let mode = state.diff_view_mode;
    let screen = state
        .guided
        .as_deref()
        .and_then(|guided| draw::chat_screen_area(guided, side, mode));
    let over = screen.filter(|area| pointer::inside(*area, m.column, m.row));
    let Some(guided) = state.guided.as_mut() else {
        return false;
    };
    match (pointer::wheel(m), over) {
        (Some(down), Some(area)) => {
            if let Some(session) = state.sessions.get_mut(id)
                && !session.send_wheel(!down, m.column - area.x, m.row - area.y)
            {
                session.scroll(!down, 3);
            }
            true
        }
        (None, Some(area)) if pointer::left_click(m) => {
            guided.mode = Mode::Chat;
            // A drag from here selects the chat's own text, and no further.
            state.selection = crate::ui::TextSelection::within(area, m.column, m.row);
            true
        }
        (None, None) if pointer::left_click(m) && guided.mode == Mode::Chat => {
            guided.mode = Mode::Browse;
            false
        }
        _ => false,
    }
}

/// The divider right of the step list, dragged, sets how wide the list is.
/// Returns whether the event went to the drag.
fn resize_list(state: &mut AppState, list: ratatui::layout::Rect, m: &MouseEvent) -> bool {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let divider = list.x.saturating_add(list.width);
            let rows = list.y..list.y.saturating_add(list.height);
            state.guided_list_drag_active = m.column == divider && rows.contains(&m.row);
            state.guided_list_drag_active
        }
        MouseEventKind::Drag(MouseButton::Left) if state.guided_list_drag_active => {
            state.guided_list_width = Some(m.column.saturating_sub(list.x));
            true
        }
        MouseEventKind::Up(MouseButton::Left) if state.guided_list_drag_active => {
            state.guided_list_drag_active = false;
            true
        }
        _ => false,
    }
}
