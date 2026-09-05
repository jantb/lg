use ratatui::{
    layout::Alignment,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders},
};

use super::palette;
use crate::{config::BORDER_COLOR, state::SPINNER_FRAMES};

/// A block with the default border color and the given title.
pub fn bordered(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(title)
}

/// Framed block for numbered panels.
/// `n` = panel number shown in title, `focused` controls border colour,
/// `count` = optional `(current, total)` shown bottom-right.
pub fn framed<'a>(
    n: u8,
    title: &'a str,
    focused: bool,
    count: Option<(usize, usize)>,
) -> Block<'a> {
    framed_with_activity(n, title, focused, count, 0, false)
}

pub fn framed_with_activity<'a>(
    n: u8,
    title: &'a str,
    focused: bool,
    count: Option<(usize, usize)>,
    tick: usize,
    active: bool,
) -> Block<'a> {
    // A running job pulses the whole frame, not only the marker in the title:
    // the border is the biggest thing a panel has, so it is what the eye reads
    // as "busy" from across the screen. Idle, the frame holds one accent shade.
    let (border_color, title_style) = if focused {
        let border = if active {
            palette::pulse(tick)
        } else {
            palette::ACCENT
        };
        (
            Style::default().fg(border).add_modifier(Modifier::BOLD),
            Style::default()
                .fg(palette::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            Style::default().fg(palette::FRAME_IDLE),
            Style::default().fg(palette::TEXT_IDLE),
        )
    };

    let title_text = if focused {
        // Focus is already carried by the border colour, so the marker only
        // animates while there is work to report.
        let pulse = if active {
            SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
        } else {
            "\u{25cf}"
        };
        format!("{pulse} [{n}] {title}")
    } else {
        format!("[{n}] {title}")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_color)
        .title(Span::styled(title_text, title_style));

    if let Some((cur, total)) = count {
        let count_text = format!("{cur} of {total}");
        block.title_bottom(
            Line::from(Span::styled(
                count_text,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ))
            .alignment(Alignment::Right),
        )
    } else {
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;

    /// The block's top border, which is where the title and its marker sit.
    fn title_row(tick: usize, active: bool) -> String {
        let area = Rect::new(0, 0, 24, 3);
        let mut buf = Buffer::empty(area);
        framed_with_activity(1, "Status", true, None, tick, active).render(area, &mut buf);
        (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect()
    }

    #[test]
    fn the_focus_marker_holds_still_while_nothing_is_running() {
        let first = title_row(0, false);
        assert!(first.contains("[1] Status"), "{first}");
        for tick in 1..8 {
            assert_eq!(
                first,
                title_row(tick, false),
                "an idle panel must not blink"
            );
        }
    }

    #[test]
    fn the_marker_animates_only_while_work_is_running() {
        assert_ne!(
            title_row(0, true),
            title_row(1, true),
            "a running job still shows progress"
        );
    }

    /// The colour of the top-left corner, which is border and nothing else.
    fn corner_color(tick: usize, focused: bool, active: bool) -> Option<Color> {
        let area = Rect::new(0, 0, 24, 3);
        let mut buf = Buffer::empty(area);
        framed_with_activity(1, "Status", focused, None, tick, active).render(area, &mut buf);
        buf[(0, 0)].style().fg
    }

    #[test]
    fn a_busy_focused_frame_pulses_and_an_idle_one_holds() {
        let idle = corner_color(0, true, false);
        for tick in 1..16 {
            assert_eq!(idle, corner_color(tick, true, false), "idle frame moved");
        }
        assert!(
            (1..16).any(|tick| corner_color(tick, true, true) != corner_color(0, true, true)),
            "a frame with work in it must visibly pulse"
        );
    }

    #[test]
    fn focus_is_told_apart_from_idle_by_colour() {
        assert_ne!(corner_color(0, true, false), corner_color(0, false, false));
        // Work in an unfocused panel does not make it move.
        assert_eq!(corner_color(0, false, true), corner_color(3, false, true));
    }

    #[test]
    fn an_unfocused_panel_has_no_marker() {
        let area = Rect::new(0, 0, 24, 3);
        let mut buf = Buffer::empty(area);
        framed_with_activity(1, "Status", false, None, 0, false).render(area, &mut buf);
        let row: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(row.contains("[1] Status"), "{row}");
        assert!(!row.contains('\u{25cf}'), "{row}");
    }
}
