use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    config::BORDER_COLOR,
    panel::keys::{self, SECTIONS},
    state::{AppState, Modal},
    ui::{animate_modal_border, centered, modal_frame_with},
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
    if state.help_return == Modal::GuidedReview {
        return Some("Guided review");
    }
    keys::active_section(state.prev_focus, state.main_keys()).map(|section| section.title)
}

/// The sections the overlay shows, each with the rows of it that do: every
/// row, or with a filter typed, the rows whose key or description has it in —
/// all of a section whose title has.
fn shown(filter: &str) -> Vec<(&'static keys::Section, Vec<&'static keys::Binding>)> {
    let filter = filter.trim().to_lowercase();
    SECTIONS
        .iter()
        .filter_map(|section| {
            let whole = filter.is_empty() || section.title.to_lowercase().contains(&filter);
            let rows: Vec<&keys::Binding> = section
                .bindings
                .iter()
                .filter(|binding| {
                    whole
                        || binding.key.to_lowercase().contains(&filter)
                        || binding.help.to_lowercase().contains(&filter)
                })
                .collect();
            (!rows.is_empty()).then_some((section, rows))
        })
        .collect()
}

/// Body lines `sections` take: a heading and the rows of each, with a blank
/// line between sections.
fn lines_of(sections: &[(&'static keys::Section, Vec<&'static keys::Binding>)]) -> u16 {
    sections
        .iter()
        .enumerate()
        .map(|(i, (_, rows))| {
            let blank = u16::from(i + 1 < sections.len());
            1 + rows.len() as u16 + blank
        })
        .sum()
}

/// Body lines the help text occupies, excluding the modal borders.
fn content_lines() -> u16 {
    lines_of(&shown(""))
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

/// The same for the rows the filter leaves.
fn max_offset_for(state: &AppState, area: Rect) -> u16 {
    lines_of(&shown(&state.help_filter)).saturating_sub(viewport_height(area))
}

pub fn scroll(state: &mut AppState, area: Rect, down: bool, amount: u16) {
    let max = max_offset_for(state, area);
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
    let max = max_offset_for(state, area);
    let offset = state.help_offset.min(max);

    let active = active_title(state);
    let sections = shown(&state.help_filter);
    let mut lines: Vec<Line> = Vec::new();
    if sections.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no key matches the filter",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for (i, (section, rows)) in sections.iter().enumerate() {
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
        for binding in rows {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<KEY_COLUMN$}", binding.key),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(binding.help),
            ]));
        }
        if i + 1 < sections.len() {
            lines.push(Line::from(""));
        }
    }

    // Plain corners, like every other modal: the overlay is one of the same
    // family of boxes, not a shape of its own.
    let block = bordered_owned(format!(
        "Help \u{2014} {}",
        active_title(state)
            .and_then(keys::section)
            .map_or("lg", keys::footer_label)
    ))
    .title_bottom(
        Line::from(Span::styled(
            match (state.help_filtering, state.help_filter.is_empty()) {
                (true, _) => format!(
                    "filter: {}\u{2502} \u{2022} Enter keep \u{2022} Esc clear",
                    state.help_filter.as_str()
                ),
                (false, false) => format!(
                    "filter: {} \u{2022} / change \u{2022} Esc clear",
                    state.help_filter.as_str()
                ),
                (false, true) if max > 0 => format!(
                    "j/k scroll \u{2022} {}%  \u{2022} / filter \u{2022} q/Esc close",
                    scroll_percent(offset, max)
                ),
                (false, true) => "/ filter \u{2022} q/Esc close".to_owned(),
            },
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))
        .alignment(Alignment::Right),
    );

    let inner = modal_frame_with(frame, overlay, block);
    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
    animate_modal_border(state.animation_ms, overlay, &[], frame);
}

/// [`crate::ui::bordered`] for a title the caller built, which outlives no
/// borrow it could lend.
fn bordered_owned(title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(title)
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
    if state.help_filtering {
        match key.code {
            KeyCode::Enter => state.help_filtering = false,
            KeyCode::Esc => {
                state.help_filtering = false;
                state.help_filter.clear();
            }
            _ => {
                if state.help_filter.edit_key(key) == crate::panel::text_input::Edit::Changed {
                    state.help_offset = 0;
                }
            }
        }
        return Ok(());
    }
    match key.code {
        KeyCode::Char('/') => {
            state.help_filtering = true;
            state.help_offset = 0;
        }
        // A kept filter goes first; the next Esc closes.
        KeyCode::Esc if !state.help_filter.is_empty() => {
            state.help_filter.clear();
            state.help_offset = 0;
        }
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
        KeyCode::Char('G') => state.help_offset = max_offset_for(state, area),
        _ => {
            state.modal = std::mem::replace(&mut state.help_return, Modal::None);
            state.help_offset = 0;
            state.help_filter.clear();
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

    /// Typing narrows the rows to the ones that mention it, Enter keeps the
    /// filter while the keys go back to scrolling, and Esc takes it away
    /// before it closes the overlay.
    #[test]
    fn slash_filters_the_rows_and_esc_clears_before_closing() {
        let area = Rect::new(0, 0, 100, 40);
        let mut state = AppState::new();
        state.modal = Modal::Help;
        let press = |state: &mut AppState, code| {
            handle_key(state, KeyEvent::new(code, KeyModifiers::NONE), area).unwrap()
        };

        press(&mut state, KeyCode::Char('/'));
        for c in "cherry".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);

        let sections = shown(&state.help_filter);
        assert!(!sections.is_empty());
        assert!(sections.iter().all(|(_, rows)| {
            rows.iter()
                .all(|row| row.help.to_lowercase().contains("cherry"))
        }));
        assert_eq!(state.modal, Modal::Help);

        press(&mut state, KeyCode::Esc);
        assert!(state.help_filter.is_empty());
        assert_eq!(state.modal, Modal::Help, "the first Esc only clears");
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.modal, Modal::None);
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
