use anyhow::Result;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::{
    app,
    state::{AppState, MergeAction, PendingAction, clamp_index},
    ui,
};

use super::scroll;

mod merge;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = area.width.saturating_sub(2).max(1).min(area.width);
    let h = area.height.saturating_sub(2).max(1).min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(7),
            Constraint::Length(5),
        ])
        .split(modal);

    let header = vec![
        Line::from(Span::styled(
            "Git conflict detected",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "Resolve here with Enter, externally with o, or with c ({}); v validates and continues.",
            state.preferred_agent.label()
        )),
        if state.conflict_resolve_job.is_some() {
            local_pass_line(state)
        } else if !state.conflict_log.is_empty() {
            Line::from(state.conflict_log.clone())
        } else {
            local_pass_line(state)
        },
    ];
    frame.render_widget(
        Paragraph::new(header).block(ui::bordered(
            state
                .conflicts
                .get(state.conflict_idx)
                .map_or("Conflict", String::as_str),
        )),
        chunks[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(if area.width >= 120 { 24 } else { 0 }),
            Constraint::Min(1),
        ])
        .split(chunks[1]);

    let items: Vec<ListItem> = state
        .conflicts
        .iter()
        .map(|path| ListItem::new(conflict_row(state, path)))
        .collect();
    let list = List::new(items)
        .block(ui::bordered("Files"))
        .highlight_style(crate::ui::palette::selection())
        .highlight_symbol("\u{203a} ");
    let selected_idx = clamp_index(state.conflict_idx, state.conflicts.len());
    let offset = scroll::selection_scroll_offset(
        selected_idx,
        state.conflicts.len(),
        scroll::list_viewport_height(body[0].height),
        state.conflict_scroll_offset,
    );
    let mut list_state = scroll::list_state(selected_idx, offset);
    frame.render_stateful_widget(list, body[0], &mut list_state);

    match state
        .conflict_preview
        .as_ref()
        .map(|preview| &preview.editor)
    {
        Some(Ok(editor)) => merge::render(editor, body[1], frame),
        preview => {
            let detail = match preview {
                Some(Err(error)) => format!("{error}\n\nPress o to open externally, Ctrl-r to reload, or v to validate resolved/staged/merged state."),
                _ => "Select a conflicted file. Press o to open externally, l for the local model, or v to validate resolved/staged/merged state.".into(),
            };
            frame.render_widget(
                Paragraph::new(detail)
                    .block(ui::bordered("Merge preview"))
                    .wrap(Wrap { trim: false }),
                body[1],
            );
        }
    }

    let editing = state
        .conflict_preview
        .as_ref()
        .is_some_and(|p| p.editor.as_ref().is_ok_and(|e| e.editing));
    let controls = if editing {
        vec![
            Line::from(
                "EDIT RESULT   arrows/Home/End move   Enter newline   Tab indent   Ctrl-z undo",
            ),
            Line::from("Esc finish editing   Ctrl-s save file   Paste supported"),
            Line::from(state.conflict_log.clone()),
        ]
    } else if modal.width < 100 {
        vec![
            Line::from("1 ours  2 theirs  3 both  0 accept  Enter edit  Ctrl-s save"),
            Line::from("j/k files  [/] conflicts  b base  PgUp/Dn scroll  ←/→ pan  u undo"),
            Line::from("Ctrl-r discard/reload  o open  v validate  l/c agents  a abort  Esc close"),
        ]
    } else {
        vec![
            Line::from(
                "j/k files   [/] conflicts   1 ours   2 theirs   3 both   0 accept result   Enter edit",
            ),
            Line::from(
                "b ancestor   PgUp/PgDn scroll   ←/→ pan   u undo   Ctrl-s save   Ctrl-r reload/discard",
            ),
            Line::from(
                "o open   v validate resolved/staged/merged state   l local model   c agent   a abort   Esc close",
            ),
        ]
    };
    frame.render_widget(
        Paragraph::new(controls).block(Block::default().borders(Borders::ALL)),
        chunks[2],
    );
}

/// The header's third line: what the local pass is doing, or what it did, or
/// the reminder that committing by hand is fine when it has not run.
fn local_pass_line(state: &AppState) -> Line<'static> {
    if let Some(job) = state.conflict_resolve_job.as_ref() {
        let progress = format!("local model: {}/{} file(s)", job.completed, job.total);
        let detail = job
            .active_path
            .as_deref()
            .map(|path| format!(" \u{2014} {path}"))
            .unwrap_or_default();
        return Line::from(Span::styled(
            format!("{progress}{detail}"),
            Style::default().fg(Color::LightGreen),
        ));
    }
    if state.conflict_resolved.is_empty() {
        return Line::from("Committing the resolution yourself is fine; v continues either way.");
    }
    Line::from(Span::styled(
        format!(
            "{} file(s) resolved in lg \u{2014} read them before v.",
            state.conflict_resolved.len()
        ),
        Style::default().fg(Color::LightGreen),
    ))
}

/// One row of the file list, marked when lg has already settled
/// it. The mark is what separates a file waiting to be read from one waiting
/// to be resolved; both are still conflicted as far as git is concerned.
fn conflict_row(state: &AppState, path: &str) -> Line<'static> {
    if state.conflict_resolved.contains(path) {
        Line::from(vec![
            Span::styled("\u{2713} ", Style::default().fg(Color::LightGreen)),
            Span::raw(path.to_string()),
        ])
    } else {
        Line::from(format!("  {path}"))
    }
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    let files_area = files_area(area);
    state.conflict_scroll_offset = scroll::selection_scroll_offset(
        clamp_index(state.conflict_idx, state.conflicts.len()),
        state.conflicts.len(),
        scroll::list_viewport_height(files_area.height),
        state.conflict_scroll_offset,
    );
}

fn files_area(area: Rect) -> Rect {
    let w = area.width.saturating_sub(2).max(1).min(area.width);
    let h = area.height.saturating_sub(2).max(1).min(area.height);
    let modal = ui::centered(area, w, h);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(7),
            Constraint::Length(5),
        ])
        .split(modal);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(if area.width >= 120 { 24 } else { 0 }),
            Constraint::Min(1),
        ])
        .split(chunks[1]);
    body[0]
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    state.conflict_idx = clamp_index(state.conflict_idx, state.conflicts.len()).unwrap_or(0);
    app::prepare_conflict_editor(state);
    if let Some(Ok(editor)) = state.conflict_preview.as_mut().map(|p| &mut p.editor) {
        editor.hovered = None;
    }
    if handle_merge_key(state, key) {
        return Ok(());
    }
    // The local pass is rewriting these files. Reading them is fine; settling,
    // aborting or starting a second resolver on top of it is not.
    if state.conflict_resolve_job.is_some()
        && matches!(
            key.code,
            KeyCode::Char('c' | 'C' | 'v' | 'V' | 'a' | 'A' | 'l' | 'L')
        )
    {
        state.set_status(
            "the local model is still working \u{2014} wait, or press Esc to stop it",
            false,
        );
        return Ok(());
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.conflict_idx = state
                .conflict_idx
                .saturating_add(1)
                .min(state.conflicts.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.conflict_idx = state.conflict_idx.saturating_sub(1);
        }
        KeyCode::Char('o') => {
            if let Some(path) = state.conflicts.get(state.conflict_idx) {
                state.pending_action = Some(PendingAction::OpenFile(path.clone()));
            } else {
                state.set_status("no conflicted file selected", false);
            }
        }
        KeyCode::Char('l') | KeyCode::Char('L') => app::spawn_conflict_resolve(state),
        KeyCode::Char('c') => app::start_conflict_session(state, true),
        KeyCode::Char('C') => app::start_conflict_session(state, false),
        KeyCode::Char('v') | KeyCode::Char('V') => app::validate_conflict_resolution(state),
        KeyCode::Char('a') | KeyCode::Char('A') => app::abort_conflict_operation(state),
        KeyCode::Esc => {
            // Esc stops LLM work everywhere else in lg, and a local pass
            // halfway through the files is exactly what someone hitting Esc
            // here means to stop. The modal stays up: the conflict is still
            // unresolved, and closing it would hide that.
            if state.conflict_resolve_job.is_some() {
                if let Some(message) = state.cancel_llm_jobs() {
                    state.set_status(message, false);
                }
            } else {
                state.modal = crate::state::Modal::None;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The result editor owns text input before any modal action sees it.
fn handle_merge_key(state: &mut AppState, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if state.conflict_resolve_job.is_some() {
        return false;
    }
    if ctrl && key.code == KeyCode::Char('r') {
        state.conflict_preview = None;
        state.conflict_log.clear();
        app::prepare_conflict_editor(state);
        return true;
    }
    if ctrl && key.code == KeyCode::Char('s') {
        app::save_conflict_editor(state);
        return true;
    }
    let Some(Ok(editor)) = state.conflict_preview.as_mut().map(|p| &mut p.editor) else {
        if key.code == KeyCode::Enter {
            state.set_status(
                "inline preview unavailable; press o to open externally",
                false,
            );
            return true;
        }
        return false;
    };
    if editor.editing {
        editor.follow_cursor = true;
        if key.code == KeyCode::Esc {
            editor.editing = false;
        } else {
            editor.current_mut().edit(key);
        }
        return true;
    }
    if !ctrl {
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => {
                editor.editing = true;
                editor.show_base = false;
            }
            KeyCode::Char(choice @ ('0'..='3')) => editor.current_mut().choose(choice),
            KeyCode::Char('u') => editor.current_mut().undo(),
            KeyCode::Char(']') => editor.navigate(true),
            KeyCode::Char('[') => editor.navigate(false),
            KeyCode::Char('b') => {
                editor.show_base = !editor.show_base;
                editor.scroll = 0;
            }
            KeyCode::PageDown => editor.scroll = editor.scroll.saturating_add(8),
            KeyCode::PageUp => editor.scroll = editor.scroll.saturating_sub(8),
            KeyCode::Right => editor.horizontal = editor.horizontal.saturating_add(8),
            KeyCode::Left => editor.horizontal = editor.horizontal.saturating_sub(8),
            _ => {
                if editor.dirty()
                    && matches!(
                        key.code,
                        KeyCode::Char(
                            'j' | 'k' | 'o' | 'v' | 'V' | 'a' | 'A' | 'l' | 'L' | 'c' | 'C'
                        ) | KeyCode::Esc
                            | KeyCode::Up
                            | KeyCode::Down
                    )
                {
                    state.set_status(
                        "unsaved merge edits: Ctrl-s saves; Ctrl-r reloads and discards the draft",
                        true,
                    );
                    return true;
                }
                return false;
            }
        }
        return true;
    }
    // Modified letters must not fall through to plain-letter merge actions.
    true
}

pub fn handle_paste(state: &mut AppState, text: &str) -> bool {
    if state.modal != crate::state::Modal::Conflict {
        return false;
    }
    if let Some(Ok(editor)) = state.conflict_preview.as_mut().map(|p| &mut p.editor)
        && editor.editing
        && state.conflict_resolve_job.is_none()
    {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let text = if editor.current().source.ours.contains("\r\n") {
            normalized.replace('\n', "\r\n")
        } else {
            normalized
        };
        editor.current_mut().insert(&text);
        editor.hovered = None;
        editor.follow_cursor = true;
    }
    true
}

pub(crate) fn handle_mouse(state: &mut AppState, area: Rect, event: &MouseEvent) {
    app::prepare_conflict_editor(state);
    if state.conflict_resolve_job.is_some() {
        return;
    }
    let files = files_area(area);
    let point = ratatui::layout::Position::new(event.column, event.row);
    if files.contains(point)
        && event.kind == MouseEventKind::Down(MouseButton::Left)
        && event.row > files.y
        && event.row < files.bottom().saturating_sub(1)
    {
        let dirty = state
            .conflict_preview
            .as_ref()
            .is_some_and(|p| p.editor.as_ref().is_ok_and(|e| e.dirty()));
        if dirty {
            state.set_status(
                "unsaved merge edits: Ctrl-s saves; Ctrl-r reloads and discards the draft",
                true,
            );
            return;
        }
        let index = state.conflict_scroll_offset + (event.row - files.y - 1) as usize;
        if index < state.conflicts.len() {
            state.conflict_idx = index;
        }
        return;
    }
    let modal = ui::centered(
        area,
        area.width.saturating_sub(2).max(1).min(area.width),
        area.height.saturating_sub(2).max(1).min(area.height),
    );
    let preview_area = Rect::new(
        files.right(),
        files.y,
        modal.right().saturating_sub(files.right()),
        files.height,
    );
    let action = state
        .conflict_preview
        .as_mut()
        .and_then(|p| p.editor.as_mut().ok())
        .and_then(|editor| merge::mouse(editor, preview_area, event));
    let Some(action) = action else {
        return;
    };
    if action == MergeAction::Save {
        app::save_conflict_editor(state);
        return;
    }
    let Some(Ok(editor)) = state.conflict_preview.as_mut().map(|p| &mut p.editor) else {
        return;
    };
    match action {
        MergeAction::ReplaceOurs => editor.current_mut().choose('1'),
        MergeAction::ReplaceTheirs => editor.current_mut().choose('2'),
        MergeAction::Both => editor.current_mut().choose('3'),
        MergeAction::Keep => editor.current_mut().choose('0'),
        MergeAction::InsertOurs | MergeAction::InsertTheirs => {
            let source = if action == MergeAction::InsertOurs {
                editor.current().source.ours.clone()
            } else {
                editor.current().source.theirs.clone()
            };
            editor.current_mut().insert(&source);
        }
        MergeAction::Edit => {
            editor.editing = true;
            editor.follow_cursor = true;
        }
        MergeAction::Base => {
            editor.show_base = !editor.show_base;
            editor.scroll = 0;
        }
        MergeAction::Previous => editor.navigate(false),
        MergeAction::Next => editor.navigate(true),
        MergeAction::Save => {}
    }
    if !matches!(
        action,
        MergeAction::Base | MergeAction::Previous | MergeAction::Next
    ) {
        editor.show_base = false;
    }
}
