use anyhow::Result;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use crate::{
    app,
    state::{AppState, MergeAction, PendingAction, clamp_index},
    ui,
};

use super::scroll;

mod merge;

/// The dialog's regions: header, file list, merge view, controls, and the
/// dividers that tell them apart inside the one frame.
struct Regions {
    header: Rect,
    files: Rect,
    preview: Rect,
    controls: Rect,
    dividers: Vec<Rect>,
}

/// Room for the file list: every path in full when the dialog can spare it,
/// never more than a third of the width, and nothing at all on a terminal too
/// narrow to hold both the list and a readable merge view.
fn files_width(state: &AppState, width: u16) -> u16 {
    if width < 90 {
        return 0;
    }
    let longest = state
        .conflicts
        .iter()
        .map(|path| path.chars().count())
        .max()
        .unwrap_or(0)
        .min(u16::MAX as usize) as u16;
    // Selection marker and resolved mark, then a column of air before the
    // divider.
    (longest + 4 + 1).clamp(24, width / 3)
}

/// The whole dialog, all but a row of the screen.
fn modal_area(area: Rect) -> Rect {
    let w = area.width.saturating_sub(2).max(1).min(area.width);
    let h = area.height.saturating_sub(2).max(1).min(area.height);
    ui::centered(area, w, h)
}

fn regions(state: &AppState, area: Rect) -> Regions {
    let inner = ui::modal_inner(modal_area(area));
    let (chunks, row_gaps) = ui::modal_row_areas(
        inner,
        &[
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ],
    );
    let files_width = files_width(state, area.width);
    if files_width == 0 {
        return Regions {
            header: chunks[0],
            files: Rect {
                width: 0,
                ..chunks[1]
            },
            preview: chunks[1],
            controls: chunks[2],
            dividers: row_gaps,
        };
    }
    let (body, col_gaps) = ui::modal_column_areas(
        chunks[1],
        &[Constraint::Length(files_width), Constraint::Min(1)],
    );
    Regions {
        header: chunks[0],
        files: body[0],
        preview: body[1],
        controls: chunks[2],
        dividers: [row_gaps, col_gaps].concat(),
    }
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = modal_area(area);
    let regions = regions(state, area);
    ui::modal_frame(
        frame,
        modal,
        state
            .conflicts
            .get(state.conflict_idx)
            .map_or("Conflict", String::as_str),
    );
    ui::draw_dividers(frame, &regions.dividers);

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
    frame.render_widget(Paragraph::new(header), regions.header);

    let items: Vec<ListItem> = state
        .conflicts
        .iter()
        .map(|path| ListItem::new(conflict_row(state, path)))
        .collect();
    let list = List::new(items)
        .highlight_style(crate::ui::palette::selection())
        .highlight_symbol("\u{203a} ");
    let selected_idx = clamp_index(state.conflict_idx, state.conflicts.len());
    let offset = scroll::selection_scroll_offset(
        selected_idx,
        state.conflicts.len(),
        regions.files.height as usize,
        state.conflict_scroll_offset,
    );
    let mut list_state = scroll::list_state(selected_idx, offset);
    frame.render_stateful_widget(list, regions.files, &mut list_state);
    ui::section_title(frame, regions.files, "Files");

    match state
        .conflict_preview
        .as_ref()
        .map(|preview| (&preview.editor, &preview.path))
    {
        Some((Ok(editor), path)) => {
            let notes = state
                .conflict_model_notes
                .get(path)
                .map_or(&[][..], Vec::as_slice);
            merge::render(editor, notes, regions.preview, frame)
        }
        preview => {
            let detail = match preview {
                Some((Err(error), _)) => format!("{error}\n\nPress o to open externally, Ctrl-r to reload, or v to validate resolved/staged/merged state."),
                _ => "Select a conflicted file. Press o to open externally, l for the local model, or v to validate resolved/staged/merged state.".into(),
            };
            frame.render_widget(
                Paragraph::new(detail).wrap(Wrap { trim: false }),
                regions.preview,
            );
            ui::section_title(frame, regions.preview, "Merge preview");
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
            Line::from("1 ours  2 theirs  3 both  0 keep  x revert  Enter edit  Ctrl-s save"),
            Line::from("j/k files  [/] conflicts  b base  PgUp/Dn scroll  ←/→ pan  u undo"),
            Line::from("Ctrl-r discard/reload  o open  v validate  l/c agents  a abort  Esc close"),
        ]
    } else {
        vec![
            Line::from(
                "j/k files   [/] conflicts   1 ours   2 theirs   3 both   0 keep   x revert   Enter edit   or click >> << X beside a conflict",
            ),
            Line::from(
                "b ancestor   PgUp/PgDn scroll   ←/→ pan   u undo   Ctrl-s save   Ctrl-r reload/discard",
            ),
            Line::from(
                "o open   v validate resolved/staged/merged state   l local model   c agent   a abort   Esc close",
            ),
        ]
    };
    frame.render_widget(Paragraph::new(controls), regions.controls);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &regions.dividers, frame);
    }
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
    let regions = regions(state, area);
    state.conflict_scroll_offset = scroll::selection_scroll_offset(
        clamp_index(state.conflict_idx, state.conflicts.len()),
        state.conflicts.len(),
        regions.files.height as usize,
        state.conflict_scroll_offset,
    );
    if let Some(Ok(editor)) = state.conflict_preview.as_mut().map(|p| &mut p.editor) {
        editor.viewport = Some(regions.preview);
    }
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
        KeyCode::Char('c') => app::start_conflict_session(state, false),
        KeyCode::Char('C') => app::start_conflict_session(state, true),
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
            autosave(state);
        } else {
            editor.current_mut().edit(key);
        }
        return true;
    }
    if !ctrl {
        let selected = editor.selected;
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => editor.apply(selected, MergeAction::Edit),
            KeyCode::Char('1') => editor.apply(selected, MergeAction::AcceptOurs),
            KeyCode::Char('2') => editor.apply(selected, MergeAction::AcceptTheirs),
            KeyCode::Char('3') => editor.apply(selected, MergeAction::Both),
            KeyCode::Char('0') => editor.apply(selected, MergeAction::Keep),
            KeyCode::Char('u') => editor.current_mut().undo(),
            KeyCode::Char(']') => editor.navigate(true),
            KeyCode::Char('[') => editor.navigate(false),
            KeyCode::Char('b') => editor.apply(selected, MergeAction::Base),
            KeyCode::PageDown => merge::scroll_by(editor, 8),
            KeyCode::PageUp => merge::scroll_by(editor, -8),
            KeyCode::Char('x') => editor.apply(selected, MergeAction::Reset),
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
        autosave(state);
        return true;
    }
    // Modified letters must not fall through to plain-letter merge actions.
    true
}

/// Write the file as soon as every conflict in it has an answer: a settled
/// file is what the reader wants on disk, and asking for a save on top of
/// the decision that settled it is a step for nothing. Edits made while a
/// conflict is still open wait for the next decision, or for Ctrl-s.
fn autosave(state: &mut AppState) {
    let settled = state
        .conflict_preview
        .as_ref()
        .is_some_and(|p| p.editor.as_ref().is_ok_and(|e| e.settled() && e.dirty()));
    if settled {
        app::save_conflict_editor(state);
    }
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
    let regions = regions(state, area);
    let files = regions.files;
    let point = ratatui::layout::Position::new(event.column, event.row);
    if files.contains(point) && event.kind == MouseEventKind::Down(MouseButton::Left) {
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
        let index = state.conflict_scroll_offset + (event.row - files.y) as usize;
        if index < state.conflicts.len() {
            state.conflict_idx = index;
        }
        return;
    }
    let action = state
        .conflict_preview
        .as_mut()
        .and_then(|p| p.editor.as_mut().ok())
        .and_then(|editor| merge::mouse(editor, regions.preview, event));
    let Some((hunk, action)) = action else {
        return;
    };
    if action == MergeAction::Save {
        app::save_conflict_editor(state);
        return;
    }
    if let Some(Ok(editor)) = state.conflict_preview.as_mut().map(|p| &mut p.editor) {
        editor.apply(hunk, action);
    }
    autosave(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    /// Clicking a file in the list opens that file, not its neighbour: the
    /// row the reader aimed at is the row the dialog drew the name on.
    #[test]
    fn clicking_a_file_in_the_list_selects_the_one_that_was_clicked() {
        let area = Rect::new(0, 0, 120, 26);
        let mut state = AppState::default();
        state.conflicts = vec![
            "src/first.rs".to_owned(),
            "src/second.rs".to_owned(),
            "src/third.rs".to_owned(),
        ];

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(&state, frame.area(), frame))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = (0..area.height)
            .find(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, *y)].symbol())
                    .collect::<String>()
                    .contains("src/third.rs")
            })
            .expect("the third file is drawn somewhere");

        let column = regions(&state, area).files.x + 4;
        handle_mouse(
            &mut state,
            area,
            &MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
        );

        assert_eq!(state.conflict_idx, 2);
    }
}
