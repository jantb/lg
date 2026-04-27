use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, Paragraph, Wrap},
};
use std::collections::HashSet;

use crate::{
    git::FileEntry,
    state::{AppState, Modal, PendingAction, SPINNER_FRAMES, TreeKind, build_tree_rows},
    ui,
};

use super::files::code_span;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = (area.width * 8 / 10).clamp(60, 120).min(area.width);
    let h = (area.height * 3 / 4).clamp(14, 34).min(area.height);
    let modal = ui::centered(area, w, h);

    frame.render_widget(Clear, modal);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(modal);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(chunks[0]);

    let (msg_view, title_text) = match &state.generation {
        Some(g) => {
            let spinner = SPINNER_FRAMES[g.spinner % SPINNER_FRAMES.len()];
            let title = format!("Commit message  {spinner} generating\u{2026}  (Esc=cancel)");
            let view = if g.output.is_empty() {
                "\u{2588}".to_owned()
            } else {
                format!("{}\u{2588}", g.output)
            };
            (view, title)
        }
        None => (
            format!("{}\u{2588}", state.commit_message),
            "Commit message  (Ctrl+S=commit  Enter=newline  Ctrl+R=regenerate  Esc=back)"
                .to_owned(),
        ),
    };

    let input = Paragraph::new(msg_view)
        .wrap(Wrap { trim: false })
        .block(ui::bordered(&title_text));
    frame.render_widget(input, left_chunks[0]);

    let generating = state.generation.is_some();
    let hints = if generating {
        Paragraph::new(Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel generation"),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("Ctrl+S", Style::default().fg(Color::Green)),
            Span::raw(" commit  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" newline  "),
            Span::styled("Ctrl+R", Style::default().fg(Color::Yellow)),
            Span::raw(" regenerate  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" back"),
        ]))
    };
    frame.render_widget(hints, left_chunks[1]);

    let staged_entries: Vec<FileEntry> = state
        .files
        .iter()
        .filter(|e| e.x != ' ' && e.x != '?')
        .cloned()
        .collect();
    let rows = build_tree_rows(&staged_entries, &HashSet::new());
    let items: Vec<ListItem> = rows
        .iter()
        .skip(1) // drop synthetic AllChanges header
        .map(|row| {
            let indent = "  ".repeat(row.depth as usize);
            let line = match &row.kind {
                TreeKind::Folder { .. } => Line::from(vec![
                    Span::raw(indent),
                    Span::styled("\u{25be}", Style::default().fg(Color::Cyan)),
                    Span::raw(" "),
                    Span::styled(
                        format!("{}/", row.label),
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                TreeKind::File { entry_idx } => {
                    let e = &staged_entries[*entry_idx];
                    Line::from(vec![
                        Span::raw(indent),
                        code_span(e.x),
                        Span::raw(" "),
                        Span::raw(row.label.clone()),
                    ])
                }
                TreeKind::AllChanges => unreachable!(),
            };
            ListItem::new(line)
        })
        .collect();

    let sidebar = List::new(items).block(ui::bordered("Staged"));
    frame.render_widget(sidebar, chunks[1]);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let generating = state.generation.is_some();
    match key.code {
        KeyCode::Esc => {
            if generating {
                state.generation = None;
                state.set_status("generation cancelled", false);
            } else {
                state.modal = Modal::None;
            }
        }
        KeyCode::Char('s') if ctrl => {
            if !generating && !state.commit_message.trim().is_empty() {
                state.pending_action = Some(PendingAction::Commit);
            }
        }
        KeyCode::Char('r') if ctrl => {
            if !generating {
                state.commit_message.clear();
                state.set_status("generating\u{2026}", false);
                state.pending_action = Some(PendingAction::GenerateMessage);
            }
        }
        KeyCode::Enter => {
            if !generating {
                state.commit_message.push('\n');
            }
        }
        KeyCode::Backspace => {
            if !generating {
                state.commit_message.pop();
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if !generating {
                state.commit_message.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}
