//! What each key does in a guided review.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::Mode;
use crate::state::{AppState, PendingAction};

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let Some(mode) = state.guided.as_ref().map(|g| g.mode.clone()) else {
        state.modal = crate::state::Modal::None;
        return Ok(());
    };
    match mode {
        Mode::Browse => browse(state, key),
        Mode::Note { .. } | Mode::Question { .. } | Mode::Fix { .. } => text_key(state, key),
        Mode::Summary { .. } => summary_key(state, key),
    }
    Ok(())
}

/// Text pasted into the field being written, if one is open.
pub fn handle_paste(state: &mut AppState, pasted: &str) -> bool {
    if state.modal != crate::state::Modal::GuidedReview {
        return false;
    }
    let Some(guided) = state.guided.as_mut() else {
        return false;
    };
    match &mut guided.mode {
        Mode::Note { text, .. } | Mode::Question { text } | Mode::Fix { text, .. } => {
            text.insert_str(pasted);
            true
        }
        Mode::Summary { body, .. } => {
            body.insert_str(pasted);
            true
        }
        Mode::Browse => false,
    }
}

fn browse(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Only the very next key can confirm deleting a note.
    if key.code != KeyCode::Char('x')
        && let Some(guided) = state.guided.as_mut()
    {
        guided.delete_armed = None;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => super::leave(state),
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Tab => {
            super::next_step(state)
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => super::previous_step(state),
        KeyCode::Char(']') => super::step_file(state, true),
        KeyCode::Char('[') => super::step_file(state, false),
        KeyCode::Char('d') if ctrl => scroll_side(state, 8),
        KeyCode::Char('u') if ctrl => scroll_side(state, -8),
        KeyCode::Char('u') => super::next_unreviewed(state),
        KeyCode::Char('m') => super::toggle_reviewed(state),
        KeyCode::Down | KeyCode::Char('j') => super::move_cursor(state, true),
        KeyCode::Up | KeyCode::Char('k') => super::move_cursor(state, false),
        KeyCode::PageDown | KeyCode::Char('J') => scroll_side(state, 8),
        KeyCode::PageUp | KeyCode::Char('K') => scroll_side(state, -8),
        KeyCode::Char('g') | KeyCode::Home => go_overview(state),
        KeyCode::Char('G') | KeyCode::End => {
            let last = state.guided.as_ref().map(|g| g.total()).unwrap_or(0);
            super::go_to(state, last, false);
        }
        KeyCode::Char('c') | KeyCode::Enter => start_note(state),
        KeyCode::Char('x') => super::delete_note_at_cursor(state),
        KeyCode::Char('a') => set_mode(
            state,
            Mode::Question {
                text: Default::default(),
            },
        ),
        KeyCode::Char('r') => super::regenerate(state),
        KeyCode::Char('R') => {
            super::reload(state);
            state.set_status("reading the change again\u{2026}", false);
        }
        KeyCode::Char('e') => super::edit_at_cursor(state, false),
        KeyCode::Char('o') => super::edit_at_cursor(state, true),
        KeyCode::Char('f') => start_fix(state, false),
        KeyCode::Char('F') => start_fix(state, true),
        KeyCode::Char('s') => super::open_summary(state),
        KeyCode::Char('y') => copy_notes(state),
        _ => {}
    }
}

fn go_overview(state: &mut AppState) {
    super::go_to(state, super::OVERVIEW, false);
}

fn set_mode(state: &mut AppState, mode: Mode) {
    if let Some(guided) = state.guided.as_mut() {
        guided.mode = mode;
    }
}

pub(super) fn scroll_side(state: &mut AppState, by: i32) {
    if let Some(guided) = state.guided.as_mut() {
        guided.side_scroll = guided.side_scroll.min(guided.side_max.get());
        guided.side_scroll = if by >= 0 {
            guided.side_scroll.saturating_add(by as u16)
        } else {
            guided.side_scroll.saturating_sub(by.unsigned_abs() as u16)
        };
    }
}

/// Write a note on the cursor line, or change the one already there.
fn start_note(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.step().is_none() {
        state.set_status("notes go on a hunk: \u{2192} to the first one", false);
        return;
    }
    let editing = guided.note_at_cursor();
    let text = editing
        .and_then(|index| guided.progress.notes.get(index))
        .map(|note| note.body.clone())
        .unwrap_or_default();
    guided.mode = Mode::Note {
        text: text.into(),
        editing,
    };
}

/// Ask Claude for a change: what the note on the cursor line says, or what is
/// typed; with `all_notes`, every note in the review.
fn start_fix(state: &mut AppState, all_notes: bool) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if !guided.source.editable() {
        state.set_status(
            "this pull request is not checked out here \u{2014} check it out to have claude change it",
            false,
        );
        return;
    }
    if all_notes && guided.progress.notes.is_empty() {
        state.set_status("no notes to address yet", false);
        return;
    }
    if !all_notes && guided.step().is_none() {
        state.set_status("pick a hunk to change", false);
        return;
    }
    let text = if all_notes {
        String::new()
    } else {
        guided
            .note_at_cursor()
            .and_then(|index| guided.progress.notes.get(index))
            .map(|note| note.body.clone())
            .unwrap_or_default()
    };
    guided.mode = Mode::Fix {
        text: text.into(),
        all_notes,
    };
}

fn copy_notes(state: &mut AppState) {
    let Some(guided) = state.guided.as_ref() else {
        return;
    };
    if guided.progress.notes.is_empty() {
        state.set_status("no notes to copy yet", false);
        return;
    }
    let text = super::notes_markdown(guided);
    state.pending_action = Some(PendingAction::CopyToClipboard {
        label: "review notes".to_string(),
        text,
    });
}

/// Typing into a note, a question or a fix request. Enter sends it; Alt+Enter
/// or Ctrl+J starts a new line; Esc drops it.
fn text_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let (Mode::Note { text, .. } | Mode::Question { text } | Mode::Fix { text, .. }) =
        &mut guided.mode
    else {
        return;
    };
    match key.code {
        KeyCode::Esc => guided.mode = Mode::Browse,
        KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => {
            text.insert_char('\n')
        }
        KeyCode::Char('j') if ctrl => text.insert_char('\n'),
        KeyCode::Char('u') if ctrl => text.clear(),
        KeyCode::Char('w') if ctrl => text.delete_word_before(),
        KeyCode::Enter => {
            let send: fn(&mut AppState) = match guided.mode {
                Mode::Note { .. } => super::commit_note,
                Mode::Question { .. } => super::ask_question,
                _ => super::start_fix,
            };
            send(state);
        }
        _ => {
            text.handle_key(key);
        }
    }
}

/// The summary: every note, and for a pull request the review to submit.
fn summary_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let is_pr = guided.source.is_pull_request();
    let Mode::Summary { event, body } = &mut guided.mode else {
        return;
    };
    match key.code {
        KeyCode::Esc => guided.mode = Mode::Browse,
        KeyCode::Tab if is_pr => *event = (*event + 1) % crate::github::ReviewEvent::ALL.len(),
        KeyCode::BackTab if is_pr => {
            let n = crate::github::ReviewEvent::ALL.len();
            *event = (*event + n - 1) % n;
        }
        KeyCode::Char('s') if ctrl && is_pr => super::submit(state),
        KeyCode::Enter if is_pr && !key.modifiers.contains(KeyModifiers::ALT) => {
            super::submit(state)
        }
        KeyCode::Enter => body.insert_char('\n'),
        KeyCode::Char('y') if ctrl || !is_pr => copy_notes(state),
        KeyCode::Char('f') if ctrl || !is_pr => {
            guided.mode = Mode::Browse;
            start_fix(state, true);
        }
        KeyCode::Char('j') if ctrl => body.insert_char('\n'),
        KeyCode::Down if !is_pr => scroll_side(state, 1),
        KeyCode::Up if !is_pr => scroll_side(state, -1),
        KeyCode::Char('j') if !is_pr => scroll_side(state, 1),
        KeyCode::Char('k') if !is_pr => scroll_side(state, -1),
        KeyCode::Char('q') if !is_pr => {
            if let Some(guided) = state.guided.as_mut() {
                guided.mode = Mode::Browse;
            }
        }
        KeyCode::Char('u') if ctrl && is_pr => body.clear(),
        _ if is_pr => {
            body.handle_key(key);
        }
        _ => {}
    }
}
