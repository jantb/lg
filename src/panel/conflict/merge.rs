//! The three-way merge view: the whole file laid out once, every conflict
//! under its own rule of buttons, with ours and theirs aligned beside the
//! result being written.

use ratatui::{
    Frame,
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    state::{MergeAction, MergeEditor, MergePart, Side},
    ui,
};

mod draw;
mod layout;
pub(super) use draw::render;
use layout::*;

/// Actions on the file as a whole; they sit in the toolbar above the panes.
const FILE_ACTIONS: [(MergeAction, &str, &str); 5] = [
    (
        MergeAction::Save,
        "Save",
        "Save this file after every conflict is settled",
    ),
    (MergeAction::Previous, "‹ Prev", "Previous conflict"),
    (MergeAction::Next, "Next ›", "Next conflict"),
    (
        MergeAction::AllOurs,
        "Accept all Ours",
        "Take our side for every conflict not yet settled",
    ),
    (
        MergeAction::AllTheirs,
        "Accept all Theirs",
        "Take their side for every conflict not yet settled",
    ),
];

/// Actions on one conflict that sit in the rule above it; taking a side
/// and ignoring sit beside the conflict itself, in the panes' gutters.
const HUNK_ACTIONS: [(MergeAction, &str, &str); 4] = [
    (MergeAction::Both, "Both", "Take both sides, ours first"),
    (
        MergeAction::Keep,
        "Keep as is",
        "Settle the conflict with the result as it stands",
    ),
    (MergeAction::Edit, "Edit", "Edit the result"),
    (MergeAction::Base, "Base", "Toggle the common ancestor"),
];

/// The buttons in the gutters at a conflict's first row: arrows take that
/// side into the result, the cross puts the conflict back as it was found.
const GUTTER_ACTIONS: [(MergeAction, &str, &str); 3] = [
    (
        MergeAction::AcceptOurs,
        ">>",
        "Take our side into the result",
    ),
    (
        MergeAction::AcceptTheirs,
        "<<",
        "Take their side into the result",
    ),
    (
        MergeAction::Reset,
        "X",
        "Revert: put the conflict back the way it was found",
    ),
];

fn describe(action: MergeAction) -> &'static str {
    FILE_ACTIONS
        .iter()
        .chain(HUNK_ACTIONS.iter())
        .chain(GUTTER_ACTIONS.iter())
        .find(|entry| entry.0 == action)
        .map_or("", |entry| entry.2)
}

/// Scroll the view by `delta` rows from where it is on screen, and stop
/// following the cursor or the selected conflict.
pub(super) fn scroll_by(editor: &mut MergeEditor, delta: isize) {
    if editor.show_base {
        let lines = editor
            .current()
            .source
            .base
            .as_ref()
            .unwrap_or(&editor.snapshot.base)
            .lines()
            .count();
        editor.base_scroll = editor
            .base_scroll
            .saturating_add_signed(delta)
            .min(lines.saturating_sub(1));
        return;
    }
    let area = editor.viewport.unwrap_or(Rect::new(0, 0, 120, 40));
    let view = view(editor, area, None);
    editor.scroll = view
        .scroll
        .saturating_add_signed(delta)
        .min(view.doc.rows.len().saturating_sub(view.height));
    editor.reveal = false;
    editor.follow_cursor = false;
}

/// Place the result cursor where the user clicked on `row` of the result pane.
fn click_result(editor: &mut MergeEditor, view: &View, row: usize, column: usize) {
    let Some(Row::Hunk { hunk, .. }) = view.doc.rows.get(row) else {
        return;
    };
    let hunk_index = *hunk;
    let hunk_view = &view.doc.hunks[hunk_index];
    // A padding row carries no result line; take the next one in the conflict.
    let line_index = view.doc.rows[row..]
        .iter()
        .take_while(|row| matches!(row, Row::Hunk { hunk, .. } if *hunk == hunk_index))
        .find_map(|row| match row {
            Row::Hunk { lines, .. } => lines[1],
            _ => None,
        })
        .unwrap_or(hunk_view.texts[1].len().saturating_sub(1));
    let column = (column + view.horizontal).saturating_sub(6);
    editor.select(hunk_index);
    let hunk = editor.current_mut();
    let start: usize = hunk
        .result
        .split_inclusive('\n')
        .take(line_index)
        .map(str::len)
        .sum();
    let line = hunk.result[start..]
        .split('\n')
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r');
    let mut width = 0;
    let mut byte = line.len();
    for (index, c) in line.char_indices() {
        let next = Line::from(display(&c.to_string())).width();
        if width + next > column {
            byte = index;
            break;
        }
        width += next;
    }
    hunk.cursor = start + byte;
    editor.editing = true;
    editor.follow_cursor = true;
}

pub(super) fn mouse(
    editor: &mut MergeEditor,
    area: Rect,
    event: &MouseEvent,
) -> Option<(usize, MergeAction)> {
    if area.width < 12 || area.height < 4 {
        return None;
    }
    editor.viewport = Some(area);
    let point = Position::new(event.column, event.row);
    // Hit-test against what is drawn: a preview may change how many rows a
    // conflict takes, and the buttons below it move with them.
    let preview = hover_preview(editor);
    let view = view(
        editor,
        area,
        preview.as_ref().map(|(hunk, text)| (*hunk, text.as_str())),
    );
    let hit = buttons(area, &view)
        .into_iter()
        .find(|button| button.area.contains(point))
        .map(|button| (button.hunk.unwrap_or(editor.selected), button.action));
    match event.kind {
        MouseEventKind::Moved => editor.hovered = hit,
        MouseEventKind::Down(MouseButton::Left) => {
            editor.hovered = None;
            // The user acts on what is on screen; keep the view where it is.
            editor.scroll = view.scroll;
            editor.reveal = false;
            if hit.is_some() {
                return hit;
            }
            if editor.show_base {
                return None;
            }
            let result = inner(panes(area)[1]);
            if !result.contains(point) {
                return None;
            }
            let row = view.scroll + (event.row - result.y) as usize;
            match view.doc.rows.get(row) {
                Some(Row::Rule { hunk, .. }) => editor.select(*hunk),
                Some(Row::Hunk { .. }) => {
                    click_result(editor, &view, row, (event.column - result.x) as usize)
                }
                _ => {}
            }
        }
        MouseEventKind::ScrollDown => {
            editor.hovered = None;
            scroll_by(editor, 3);
        }
        MouseEventKind::ScrollUp => {
            editor.hovered = None;
            scroll_by(editor, -3);
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_retains_every_line_and_matches_common_anchors() {
        let ours = ["same", "left", "tail"];
        let result = ["same", "left", "right", "tail"];
        let theirs = ["same", "right", "tail"];
        let rows = aligned_rows(&ours, &result, &theirs);
        for (pane, count) in [ours.len(), result.len(), theirs.len()]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                rows.iter().filter_map(|row| row[pane]).collect::<Vec<_>>(),
                (0..count).collect::<Vec<_>>()
            );
        }
        assert_eq!(rows.first(), Some(&[Some(0), Some(0), Some(0)]));
        assert_eq!(rows.last(), Some(&[Some(2), Some(3), Some(2)]));
    }

    #[test]
    fn rule_buttons_wrap_instead_of_running_past_a_narrow_rule() {
        let wide = rule_slots(160);
        assert!(wide.iter().all(|slot| slot.0 == 0));
        let narrow = rule_slots(40);
        assert!(rule_height(40) > 1);
        for (line, x, _, label) in narrow {
            assert!(x + button_width(label) <= 40 || (line > 0 && x == 0));
        }
    }
}
