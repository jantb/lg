use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::{
    config::BORDER_COLOR,
    panel::keys::{self, SECTIONS},
    state::{AppState, Modal},
    ui::centered,
};

/// The overlay is a fixed width and never wraps, so anything wider than it is
/// silently cut off mid-word.
const OVERLAY_WIDTH: u16 = 64;
/// Key column, wide enough for the longest binding plus a separating space.
const KEY_COLUMN: usize = 16;
/// What a description has left: the overlay minus its borders and the indented
/// key column in front of it. Only the table check needs it spelled out.
#[cfg(test)]
const DESC_WIDTH: usize = OVERLAY_WIDTH as usize - 2 - 2 - KEY_COLUMN;

/// The section the overlay opens at and highlights: the one whose keys the
/// pane behind it is listening for.
fn active_title(state: &AppState) -> Option<&'static str> {
    keys::active_section(state.prev_focus, state.main_keys()).map(|section| section.title)
}

/// Body lines the help text occupies, excluding the modal borders.
fn content_lines() -> u16 {
    SECTIONS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let blank = if i + 1 < SECTIONS.len() { 1u16 } else { 0u16 };
            1 + s.bindings.len() as u16 + blank
        })
        .sum()
}

/// Body height of the help overlay for `area`, i.e. how many lines are visible at once.
fn viewport_height(area: Rect) -> u16 {
    let overlay_height = content_lines()
        .saturating_add(2)
        .min(area.height.saturating_sub(2))
        .max(3.min(area.height));
    overlay_height.saturating_sub(2)
}

/// Largest scroll offset that still shows content in the last row.
pub fn max_offset(area: Rect) -> u16 {
    content_lines().saturating_sub(viewport_height(area))
}

pub fn scroll(state: &mut AppState, area: Rect, down: bool, amount: u16) {
    let max = max_offset(area);
    state.help_offset = if down {
        state.help_offset.saturating_add(amount).min(max)
    } else {
        state.help_offset.saturating_sub(amount)
    };
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let height = content_lines()
        .saturating_add(2)
        .min(area.height.saturating_sub(2))
        .max(3.min(area.height));
    let overlay = centered(area, OVERLAY_WIDTH, height);
    let offset = state.help_offset.min(max_offset(area));

    frame.render_widget(Clear, overlay);

    let active = active_title(state);
    let mut lines: Vec<Line> = Vec::new();
    for (i, section) in SECTIONS.iter().enumerate() {
        let is_active = Some(section.title) == active;
        let heading_style = if is_active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let prefix = if is_active { "\u{25b6} " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!("{prefix}{}", section.title),
            heading_style,
        )));
        for binding in section.bindings {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<KEY_COLUMN$}", binding.key),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(binding.help),
            ]));
        }
        if i + 1 < SECTIONS.len() {
            lines.push(Line::from(""));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(format!(
            "Help \u{2014} {}",
            active_title(state)
                .and_then(keys::section)
                .map_or("lg", keys::footer_label)
        ))
        .title_bottom(
            Line::from(Span::styled(
                if max_offset(area) > 0 {
                    format!(
                        "j/k scroll \u{2022} {}%  \u{2022} q/Esc close",
                        scroll_percent(offset, max_offset(area))
                    )
                } else {
                    "q/Esc close".to_owned()
                },
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ))
            .alignment(Alignment::Right),
        );

    let para = Paragraph::new(lines).block(block).scroll((offset, 0));
    frame.render_widget(para, overlay);
}

fn scroll_percent(offset: u16, max: u16) -> u16 {
    if max == 0 {
        100
    } else {
        (u32::from(offset) * 100 / u32::from(max)) as u16
    }
}

/// Where to open the overlay so the focused pane's section is the first thing
/// on it. Clamped, so a short table still opens at the top.
pub fn open_offset(state: &AppState, area: Rect) -> u16 {
    active_title(state).map_or(0, |title| keys::section_line(title).min(max_offset(area)))
}

pub fn handle_key(state: &mut AppState, key: KeyEvent, area: Rect) -> Result<()> {
    let page = viewport_height(area).saturating_sub(1).max(1);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => scroll(state, area, true, 1),
        KeyCode::Char('k') | KeyCode::Up => scroll(state, area, false, 1),
        KeyCode::PageDown | KeyCode::Char(' ') => scroll(state, area, true, page),
        KeyCode::PageUp => scroll(state, area, false, page),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, area, true, page / 2)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            scroll(state, area, false, page / 2)
        }
        KeyCode::Char('g') => state.help_offset = 0,
        KeyCode::Char('G') => state.help_offset = max_offset(area),
        _ => {
            state.modal = Modal::None;
            state.help_offset = 0;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset the overlay opens at is counted in `keys`, and the lines it
    /// draws are laid out here. If the two ever disagree, pressing ? lands near
    /// the right section instead of on it.
    #[test]
    fn a_sections_line_is_where_the_overlay_draws_it() {
        let mut expected = 0u16;
        for section in SECTIONS {
            assert_eq!(
                keys::section_line(section.title),
                expected,
                "{} starts somewhere else on screen",
                section.title
            );
            // Heading, bindings, and the blank line before the next section.
            expected += 2 + section.bindings.len() as u16;
        }
    }

    /// The overlay does not wrap. A binding that outgrows it is not shortened,
    /// it is cut off — so the table is checked rather than trusted.
    #[test]
    fn every_binding_fits_the_overlay() {
        for section in SECTIONS {
            for binding in section.bindings {
                let (key, desc) = (binding.key, binding.help);
                assert!(
                    key.chars().count() < KEY_COLUMN,
                    "{key:?} leaves no gap before its description"
                );
                assert!(
                    desc.chars().count() <= DESC_WIDTH,
                    "{key:?} description is {} columns, {DESC_WIDTH} fit: {desc:?}",
                    desc.chars().count()
                );
            }
        }
    }
}
