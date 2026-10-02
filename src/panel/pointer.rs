//! Hit-testing for the modals that take the mouse: which row of a list a
//! click landed on, which of a line of buttons, and which way the wheel went.

use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

pub(crate) fn inside(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

/// `Some(true)` for the wheel turned down, `Some(false)` up, `None` for
/// anything else.
pub(crate) fn wheel(m: &MouseEvent) -> Option<bool> {
    match m.kind {
        MouseEventKind::ScrollDown => Some(true),
        MouseEventKind::ScrollUp => Some(false),
        _ => None,
    }
}

pub(crate) fn left_click(m: &MouseEvent) -> bool {
    matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
}

/// The entry of a list drawn in `list` from entry `first` down that the click
/// at (`column`, `row`) landed on, if it landed on one of its `len` entries.
pub(crate) fn list_row(
    list: Rect,
    first: usize,
    len: usize,
    column: u16,
    row: u16,
) -> Option<usize> {
    if !inside(list, column, row) {
        return None;
    }
    let at = first + usize::from(row - list.y);
    (at < len).then_some(at)
}

/// Which button of a line drawn from column `x` the click at `column` hit.
/// The line is its segments in order, each with the key it stands for; a
/// segment with none is the space between buttons.
pub(crate) fn button_at(
    x: u16,
    segments: &[(&str, Option<KeyCode>)],
    column: u16,
) -> Option<KeyCode> {
    let mut start = x;
    for (text, key) in segments {
        let end = start.saturating_add(text.chars().count() as u16);
        if column >= start && column < end {
            return *key;
        }
        start = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_lands_on_the_button_under_it() {
        let line = [
            ("y", Some(KeyCode::Char('y'))),
            (" confirm", Some(KeyCode::Char('y'))),
            ("  ", None),
            ("n/Esc", Some(KeyCode::Char('n'))),
        ];
        assert_eq!(button_at(10, &line, 10), Some(KeyCode::Char('y')));
        assert_eq!(button_at(10, &line, 18), Some(KeyCode::Char('y')));
        assert_eq!(button_at(10, &line, 19), None);
        assert_eq!(button_at(10, &line, 21), Some(KeyCode::Char('n')));
        assert_eq!(button_at(10, &line, 40), None);
    }

    #[test]
    fn a_click_below_the_last_entry_selects_nothing() {
        let list = Rect::new(0, 5, 20, 10);
        assert_eq!(list_row(list, 0, 3, 2, 6), Some(1));
        assert_eq!(list_row(list, 4, 30, 2, 5), Some(4));
        assert_eq!(list_row(list, 0, 3, 2, 9), None);
        assert_eq!(list_row(list, 0, 3, 2, 4), None);
    }
}
