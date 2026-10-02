//! The stash: every entry with where it came from, applying, popping or
//! dropping one, and stashing the working changes under a name.
//!
//! lg stashes on its own on the way into a pull or a branch flow, and puts the
//! stash back on the way out. A flow that stopped on a conflict, or a pull that
//! failed, can leave one behind; those are marked here, since they are the
//! ones a reader is most likely to be looking for.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::{
    panel::text_input::TextInput,
    state::{AppState, Modal, PendingAction},
    ui,
};

/// Open the stash list, read afresh.
pub fn open(state: &mut AppState) {
    state.stash.naming = None;
    state.stash.reload();
    state.modal = Modal::Stash;
}

/// Open the stash list straight onto naming a new entry.
pub fn open_new(state: &mut AppState) {
    open(state);
    start_naming(state);
}

fn start_naming(state: &mut AppState) {
    if state.files.is_empty() {
        state.set_status("nothing to stash: the working tree is clean", false);
        return;
    }
    state.stash.naming = Some(TextInput::default());
}

fn modal_area(state: &AppState, area: Rect) -> Rect {
    let rows = state.stash.entries.len().max(1) as u16;
    let height = (rows + 6).min(area.height).max(7.min(area.height));
    ui::centered(area, 90.min(area.width), height)
}

/// A click on an entry selects it; the wheel moves through them. Applying,
/// popping and dropping stay on their keys, so a stray click changes
/// nothing in the working tree.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> Result<()> {
    if state.stash.naming.is_some() {
        return Ok(());
    }
    if let Some(down) = super::pointer::wheel(m) {
        return handle_key(
            state,
            KeyEvent::from(if down { KeyCode::Down } else { KeyCode::Up }),
        );
    }
    if !super::pointer::left_click(m) {
        return Ok(());
    }
    let inner = ui::modal_inner(modal_area(state, area));
    let (chunks, _) = ui::modal_row_areas(
        inner,
        &[
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ],
    );
    let len = state.stash.entries.len();
    let selected = state.stash.idx.min(len.saturating_sub(1));
    let first = selected.saturating_sub((chunks[0].height.max(1) as usize).saturating_sub(1));
    if let Some(at) = super::pointer::list_row(chunks[0], first, len, m.column, m.row) {
        state.stash.idx = at;
    }
    Ok(())
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let form = &state.stash;
    let modal = modal_area(state, area);
    let title = match form.entries.len() {
        0 => "Stash".to_string(),
        n => format!("Stash \u{b7} {n}"),
    };
    let inner = ui::modal_frame(frame, modal, &title);
    let (chunks, dividers) = ui::modal_rows(
        frame,
        inner,
        &[
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ],
    );

    if let Some(error) = &form.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("could not read the stash: {error}"),
                Style::default().fg(Color::Red),
            )),
            chunks[0],
        );
    } else if form.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "The stash is empty. n stashes the working changes.",
                Style::default().fg(Color::DarkGray),
            )),
            chunks[0],
        );
    } else {
        let items: Vec<ListItem> = form
            .entries
            .iter()
            .map(|entry| {
                let mut spans = vec![
                    Span::styled(
                        format!("stash@{{{}}} ", entry.index),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("{} ", entry.short_sha()),
                        Style::default().fg(Color::Yellow),
                    ),
                ];
                if entry.auto {
                    spans.push(Span::styled(
                        "lg auto-stash ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                spans.push(Span::raw(entry.subject.clone()));
                spans.push(Span::styled(
                    format!("  {}", entry.age),
                    Style::default().fg(Color::DarkGray),
                ));
                ListItem::new(Line::from(spans))
            })
            .collect();
        let list = List::new(items)
            .highlight_style(crate::ui::palette::selection())
            .highlight_symbol("\u{203a} ");
        let selected = form.idx.min(form.entries.len().saturating_sub(1));
        let visible = chunks[0].height.max(1) as usize;
        let offset = selected.saturating_sub(visible.saturating_sub(1));
        let mut list_state = super::scroll::list_state(Some(selected), offset);
        frame.render_stateful_widget(list, chunks[0], &mut list_state);
    }

    match &form.naming {
        Some(input) => {
            let label = "Message: ";
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(label, Style::default().fg(Color::Yellow)),
                    Span::raw(input.as_str().to_string()),
                ])),
                chunks[1],
            );
            let column = (label.chars().count() + input.before_cursor().chars().count()) as u16;
            if column < chunks[1].width {
                frame.set_cursor_position(Position::new(chunks[1].x + column, chunks[1].y));
            }
        }
        None => {
            let note = form
                .selected()
                .filter(|entry| entry.auto)
                .map(|_| {
                    "lg stashed this before a pull or flow that did not finish; apply or pop it to get the work back"
                })
                .unwrap_or("");
            frame.render_widget(
                Paragraph::new(Span::styled(note, Style::default().fg(Color::DarkGray))),
                chunks[1],
            );
        }
    }

    let hints: Vec<Span> = if form.naming.is_some() {
        vec![
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" stash (message optional)  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel"),
        ]
    } else {
        vec![
            Span::styled("space", Style::default().fg(Color::Green)),
            Span::raw(" apply  "),
            Span::styled("g", Style::default().fg(Color::Green)),
            Span::raw(" pop  "),
            Span::styled("d", Style::default().fg(Color::Red)),
            Span::raw(" drop  "),
            Span::styled("n", Style::default().fg(Color::Yellow)),
            Span::raw(" new  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" close"),
        ]
    };
    frame.render_widget(Paragraph::new(Line::from(hints)), chunks[2]);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    if let Some(input) = state.stash.naming.as_mut() {
        match key.code {
            KeyCode::Esc => state.stash.naming = None,
            KeyCode::Enter => {
                let message = input.as_str().trim().to_string();
                state.stash.naming = None;
                state.pending_action = Some(PendingAction::StashPush { message });
            }
            _ => {
                input.handle_key(key);
            }
        }
        return Ok(());
    }
    let last = state.stash.entries.len().saturating_sub(1);
    state.stash.idx = state.stash.idx.min(last);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => state.stash.idx = (state.stash.idx + 1).min(last),
        KeyCode::Char('k') | KeyCode::Up => state.stash.idx = state.stash.idx.saturating_sub(1),
        KeyCode::Char(' ') | KeyCode::Enter => {
            if let Some(entry) = state.stash.selected() {
                state.pending_action = Some(PendingAction::StashApply {
                    sha: entry.sha.clone(),
                });
            }
        }
        KeyCode::Char('g') => {
            if let Some(entry) = state.stash.selected() {
                state.pending_action = Some(PendingAction::StashPop {
                    sha: entry.sha.clone(),
                });
            }
        }
        KeyCode::Char('d') => {
            if let Some(entry) = state.stash.selected().cloned() {
                let detail = if entry.auto {
                    format!(
                        "{}\nlg made this one on its own, before a pull or flow that did not finish. It may hold work that is nowhere else.",
                        entry.subject
                    )
                } else {
                    entry.subject.clone()
                };
                state.confirm_action(
                    "Drop stash",
                    format!("Drop stash@{{{}}} ({})?", entry.index, entry.short_sha()),
                    detail,
                    PendingAction::StashDrop { sha: entry.sha },
                );
            }
        }
        KeyCode::Char('n') => start_naming(state),
        KeyCode::Esc => state.modal = Modal::None,
        _ => {}
    }
    Ok(())
}
