use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

use crate::{
    app,
    state::{AppState, SPINNER_FRAMES, clamp_index},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let selected_idx = clamp_index(state.branches_idx, state.branches.len());
    let count = selected_idx.map(|idx| (idx + 1, state.branches.len()));
    let block = ui::framed_with_activity(
        3,
        "Branches",
        focused,
        count,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let row_width = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = state
        .branches
        .iter()
        .map(|b| {
            let line = if state
                .checkout_job
                .as_ref()
                .is_some_and(|job| job.branch == b.name)
            {
                let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
                let mut spans = vec![
                    Span::styled(
                        format!("{spinner} "),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        visible_branch_name(b, 2, row_width),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ];
                append_branch_status(&mut spans, b);
                Line::from(spans)
            } else if b.is_current {
                let mut spans = vec![Span::styled(
                    format!("* {}", visible_branch_name(b, 2, row_width)),
                    current_branch_style(),
                )];
                append_branch_status(&mut spans, b);
                Line::from(spans)
            } else {
                let mut spans = vec![Span::styled(
                    format!("  {}", visible_branch_name(b, 2, row_width)),
                    Style::default(),
                )];
                append_branch_status(&mut spans, b);
                Line::from(spans)
            };
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    let mut list_state = ListState::default();
    if focused && let Some(idx) = selected_idx {
        list_state.select(Some(idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    state.branches_idx = clamp_index(state.branches_idx, state.branches.len()).unwrap_or(0);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.branches_idx = state
                .branches_idx
                .saturating_add(1)
                .min(state.branches.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.branches_idx > 0 {
                state.branches_idx -= 1;
            }
        }
        KeyCode::Enter => {
            if let Some(b) = state.branches.get(state.branches_idx) {
                if state.checkout_job.is_none() && !b.is_current {
                    let name = b.name.clone();
                    app::checkout_branch_async(state, name);
                }
            }
        }
        KeyCode::Char('D') => {
            if let Some(b) = state.branches.get(state.branches_idx) {
                if b.is_current {
                    state.set_status("cannot delete the current branch", true);
                } else {
                    let snapshot = b.clone();
                    state.open_delete_branch_modal(&snapshot);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn visible_branch_name(
    branch: &crate::git::Branch,
    prefix_width: usize,
    row_width: usize,
) -> String {
    let status_width = branch_status_width(branch);
    let max_name_width = row_width
        .saturating_sub(prefix_width)
        .saturating_sub(status_width);
    truncate_chars(&branch.name, max_name_width)
}

fn append_branch_status(spans: &mut Vec<Span<'static>>, branch: &crate::git::Branch) {
    if branch.upstream_gone {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "(upstream gone)",
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ));
    } else if branch.upstream.is_some() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "\u{2713}",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));
    }
}

fn branch_status_width(branch: &crate::git::Branch) -> usize {
    if branch.upstream_gone {
        " (upstream gone)".chars().count()
    } else if branch.upstream.is_some() {
        " \u{2713}".chars().count()
    } else {
        0
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() && max_chars > 0 {
        out.pop();
        out.push('\u{2026}');
    }
    out
}

fn current_branch_style() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}
