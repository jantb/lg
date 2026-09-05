//! Selecting text with the mouse, one pane at a time.
//!
//! The terminal's own selection runs straight across the split view, so a drag
//! that starts in the diff picks up half of every list beside it. This one is
//! clipped to the pane it started in and reads what is actually drawn there,
//! so it works the same over a diff, a list, or a running session without any
//! of them knowing about it. Letting go copies what was selected.

use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::Modifier;

/// A drag in progress or just finished, in screen cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    /// The cells the selection may cover: the inside of one pane.
    area: Rect,
    /// Where the drag started.
    anchor: Position,
    /// Where the pointer is, or was let go.
    head: Position,
    /// Whether the mouse button is still held.
    pub dragging: bool,
    /// Whether the button has been let go and the text is still to be copied,
    /// which has to wait for the next frame because the text is read off it.
    pub copy_requested: bool,
}

impl TextSelection {
    /// Start a selection at a cell inside `pane`, whose border is not
    /// selectable. `None` when the pane has no inside to select.
    pub fn start(pane: Rect, column: u16, row: u16) -> Option<Self> {
        let area = pane.inner(Margin::new(1, 1));
        if area.is_empty() {
            return None;
        }
        let anchor = clamp(area, column, row);
        Some(Self {
            area,
            anchor,
            head: anchor,
            dragging: true,
            copy_requested: false,
        })
    }

    /// Follow the pointer, staying inside the pane.
    pub fn extend(&mut self, column: u16, row: u16) {
        self.head = clamp(self.area, column, row);
    }

    /// The button was let go. A click that never moved selects nothing and
    /// is over; a drag becomes a copy on the next frame.
    pub fn release(mut self) -> Option<Self> {
        self.dragging = false;
        if self.anchor == self.head {
            return None;
        }
        self.copy_requested = true;
        Some(self)
    }

    /// The first and last selected cells in reading order.
    fn bounds(&self) -> (Position, Position) {
        let key = |p: Position| (p.y, p.x);
        if key(self.anchor) <= key(self.head) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Whether a cell is selected: from the start cell to the end cell in
    /// reading order, full rows in between, all clipped to the pane.
    pub fn contains(&self, column: u16, row: u16) -> bool {
        if !self.area.contains(Position::new(column, row)) {
            return false;
        }
        let (start, end) = self.bounds();
        if row < start.y || row > end.y {
            return false;
        }
        if start.y == end.y {
            return column >= start.x && column <= end.x;
        }
        (row != start.y || column >= start.x) && (row != end.y || column <= end.x)
    }

    /// Mark the selected cells so the reader can see what they have.
    pub fn highlight(&self, buf: &mut Buffer) {
        let (start, end) = self.bounds();
        for row in start.y..=end.y {
            for column in self.area.left()..self.area.right() {
                if self.contains(column, row)
                    && let Some(cell) = buf.cell_mut(Position::new(column, row))
                {
                    cell.modifier |= Modifier::REVERSED;
                }
            }
        }
    }

    /// What the selected cells say, one line per row with the padding a pane
    /// draws to its right edge taken off.
    pub fn text(&self, buf: &Buffer) -> String {
        let (start, end) = self.bounds();
        (start.y..=end.y)
            .map(|row| {
                let line: String = (self.area.left()..self.area.right())
                    .filter(|&column| self.contains(column, row))
                    .filter_map(|column| buf.cell(Position::new(column, row)))
                    .map(|cell| cell.symbol())
                    .collect();
                line.trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn clamp(area: Rect, column: u16, row: u16) -> Position {
    Position::new(
        column.clamp(area.left(), area.right().saturating_sub(1)),
        row.clamp(area.top(), area.bottom().saturating_sub(1)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane_with(lines: &[&str]) -> (Rect, Buffer) {
        let pane = Rect::new(10, 5, 20, lines.len() as u16 + 2);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        for (i, line) in lines.iter().enumerate() {
            buf.set_string(11, 6 + i as u16, line, ratatui::style::Style::default());
            // What lies outside the pane must never be picked up.
            buf.set_string(31, 6 + i as u16, "XXXX", ratatui::style::Style::default());
        }
        (pane, buf)
    }

    #[test]
    fn a_drag_reads_the_text_between_its_ends_and_nothing_beside_it() {
        let (pane, buf) = pane_with(&["first line here", "second line", "third"]);
        let mut sel = TextSelection::start(pane, 17, 6).unwrap();
        sel.extend(14, 8);
        let sel = sel.release().unwrap();

        assert_eq!(sel.text(&buf), "line here\nsecond line\nthir");
        assert!(sel.copy_requested);
    }

    #[test]
    fn dragging_upwards_selects_the_same_text() {
        let (pane, buf) = pane_with(&["first line here", "second line"]);
        let mut sel = TextSelection::start(pane, 16, 7).unwrap();
        sel.extend(17, 6);

        assert_eq!(sel.text(&buf), "line here\nsecond");
    }

    #[test]
    fn a_click_that_never_moved_selects_nothing() {
        let (pane, _) = pane_with(&["first line here"]);
        let sel = TextSelection::start(pane, 12, 6).unwrap();
        assert!(sel.release().is_none());
    }

    #[test]
    fn the_pointer_leaving_the_pane_stops_at_its_edge() {
        let (pane, buf) = pane_with(&["first line here", "second line"]);
        let mut sel = TextSelection::start(pane, 11, 6).unwrap();
        sel.extend(60, 40);

        assert_eq!(sel.text(&buf), "first line here\nsecond line");
    }

    #[test]
    fn the_selection_is_shown_on_the_cells_it_covers() {
        let (pane, mut buf) = pane_with(&["first line here"]);
        let mut sel = TextSelection::start(pane, 11, 6).unwrap();
        sel.extend(13, 6);
        sel.highlight(&mut buf);

        let reversed = |x: u16| {
            buf.cell(Position::new(x, 6))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        };
        assert!(reversed(11) && reversed(13));
        assert!(!reversed(14));
    }
}
