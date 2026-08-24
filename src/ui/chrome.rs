use ratatui::{
    layout::Alignment,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders},
};

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
    let (border_color, title_style) = if focused {
        (
            if active {
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            },
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::Gray),
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
