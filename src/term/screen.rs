//! Draw a parsed terminal screen into a ratatui area.
//!
//! Written against the buffer rather than as a widget so the session pane can
//! sit in lg's layout next to the git panes, with no dependency on a terminal
//! widget crate tracking a particular ratatui version.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

/// Paint `screen` into `area`. Cells beyond the screen's own size are cleared,
/// so a shrinking program never leaves its old output behind.
pub fn render_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer) {
    let (screen_rows, screen_cols) = screen.size();
    for row in 0..area.height {
        for col in 0..area.width {
            let x = area.x + col;
            let y = area.y + row;
            if x >= buf.area.right() || y >= buf.area.bottom() {
                continue;
            }
            let target = &mut buf[(x, y)];
            if row >= screen_rows || col >= screen_cols {
                target.reset();
                continue;
            }
            let Some(cell) = screen.cell(row, col) else {
                target.reset();
                continue;
            };
            // The right half of a double-width character is drawn by the left
            // half; leaving it empty is how ratatui expects wide cells.
            if cell.is_wide_continuation() {
                target.reset();
                target.set_symbol("");
                continue;
            }
            target.reset();
            if cell.has_contents() {
                target.set_symbol(cell.contents());
            } else {
                target.set_symbol(" ");
            }
            target.set_style(cell_style(cell));
        }
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default()
        .fg(color(cell.fgcolor()))
        .bg(color(cell.bgcolor()));
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.dim() {
        style = style.add_modifier(Modifier::DIM);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

fn color(color: vt100::Color) -> Color {
    match color {
        // Reset, so the terminal's own default shows through rather than a
        // guess at what the user's black or white is.
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen_with(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(bytes);
        parser
    }

    fn text_of(buf: &Buffer) -> String {
        let mut text = String::new();
        for y in buf.area.top()..buf.area.bottom() {
            for x in buf.area.left()..buf.area.right() {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn plain_text_lands_in_the_area() {
        let parser = screen_with(2, 8, b"hi\r\nthere");
        let area = Rect::new(0, 0, 8, 2);
        let mut buf = Buffer::empty(area);
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(text_of(&buf), "hi      \nthere   \n");
    }

    #[test]
    fn drawing_is_offset_into_the_area_it_is_given() {
        let parser = screen_with(1, 2, b"ab");
        let area = Rect::new(2, 1, 2, 1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 3));
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(buf[(2, 1)].symbol(), "a");
        assert_eq!(buf[(3, 1)].symbol(), "b");
        assert_eq!(buf[(0, 0)].symbol(), " ", "outside the area is untouched");
    }

    #[test]
    fn colors_and_attributes_carry_over() {
        // Bold, red on blue.
        let parser = screen_with(1, 4, b"\x1b[1;31;44mx\x1b[m");
        let area = Rect::new(0, 0, 4, 1);
        let mut buf = Buffer::empty(area);
        render_screen(parser.screen(), area, &mut buf);
        let cell = &buf[(0, 0)];
        assert_eq!(cell.fg, Color::Indexed(1));
        assert_eq!(cell.bg, Color::Indexed(4));
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn true_color_is_kept_exactly() {
        let parser = screen_with(1, 2, b"\x1b[38;2;10;20;30mx");
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn the_default_color_resets_rather_than_guessing() {
        let parser = screen_with(1, 2, b"x");
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(buf[(0, 0)].fg, Color::Reset);
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
    }

    #[test]
    fn a_wide_character_leaves_its_second_cell_empty() {
        let parser = screen_with(1, 4, "字x".as_bytes());
        let area = Rect::new(0, 0, 4, 1);
        let mut buf = Buffer::empty(area);
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "字");
        assert_eq!(buf[(1, 0)].symbol(), "");
        assert_eq!(buf[(2, 0)].symbol(), "x");
    }

    #[test]
    fn an_area_larger_than_the_screen_is_blanked() {
        let parser = screen_with(1, 2, b"ab");
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        buf[(3, 1)].set_symbol("stale");
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(text_of(&buf), "ab  \n    \n");
    }

    #[test]
    fn an_area_outside_the_buffer_is_ignored() {
        let parser = screen_with(4, 8, b"abcdefgh");
        let area = Rect::new(6, 0, 8, 4);
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 1));
        render_screen(parser.screen(), area, &mut buf);
        assert_eq!(buf[(6, 0)].symbol(), "a");
        assert_eq!(buf[(7, 0)].symbol(), "b");
    }
}
