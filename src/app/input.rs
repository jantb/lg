use anyhow::Result;
use ratatui::{
    backend::Backend,
    crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::Rect,
};

use crate::{
    panel,
    state::{AppState, Modal, Pane, PendingAction},
    ui,
};

use super::{
    App, HeadlessApp, mouse, open_author_modal, open_model_modal, selected_commit_ref,
    spawn_assisted_review, spawn_pull, spawn_push,
};

mod host;
mod keys;
mod pointer;
pub(super) use host::AppHost;
use host::*;
use keys::*;
use pointer::*;

fn flow_unavailable_reason(state: &AppState) -> &'static str {
    if state.focus != Pane::Branches {
        "branch actions need the Branches pane"
    } else {
        "no branch actions available here"
    }
}

fn next_pane(p: Pane) -> Pane {
    match p {
        Pane::Status => Pane::Files,
        Pane::Files => Pane::Branches,
        Pane::Branches => Pane::Commits,
        Pane::Commits => Pane::Main,
        Pane::Main => Pane::Status,
    }
}

fn prev_pane(p: Pane) -> Pane {
    match p {
        Pane::Status => Pane::Main,
        Pane::Files => Pane::Status,
        Pane::Branches => Pane::Files,
        Pane::Commits => Pane::Branches,
        Pane::Main => Pane::Commits,
    }
}

/// Tab and its shortcuts only visit panes that are on screen, so in workspace
/// mode focus moves between the tree and the session and nowhere else.
fn cycle_pane(state: &AppState, forward: bool) -> Pane {
    if state.git_panes_visible() {
        return if forward {
            next_pane(state.focus)
        } else {
            prev_pane(state.focus)
        };
    }
    match state.focus {
        Pane::Main => Pane::Status,
        _ => Pane::Main,
    }
}

impl<B: Backend> HeadlessApp<B>
where
    B::Error: Send + Sync + 'static,
{
    pub(super) fn terminal_area(&self) -> Result<Rect> {
        let size = self.terminal.size()?;
        Ok(Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        })
    }

    pub fn send_key(&mut self, k: KeyEvent) -> Result<()> {
        dispatch_key(self, k)?;
        self.render()
    }

    pub fn send_mouse(&mut self, m: MouseEvent) -> Result<()> {
        dispatch_mouse(self, m)?;
        self.render()
    }
}

impl App {
    pub(super) fn terminal_area(&self) -> Result<Rect> {
        let size = self.terminal.size()?;
        Ok(Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        })
    }

    pub(super) fn handle_key(&mut self, k: KeyEvent) -> Result<()> {
        dispatch_key(self, k)
    }

    pub(super) fn handle_mouse(&mut self, m: MouseEvent) -> Result<()> {
        dispatch_mouse(self, m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn conflict_modal_mouse_is_consumed_before_background_focus() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();
        state.focus = Pane::Files;
        state.modal = Modal::Conflict;
        state.column_drag_active = true;
        state.row_drag_active = Some((2, 3));

        assert!(handle_modal_mouse(&mut state, area, &left_click(80, 10)));

        assert_eq!(state.focus, Pane::Files);
        assert!(!state.column_drag_active);
        assert_eq!(state.row_drag_active, None);
    }

    /// A commit message can take a minute to write. Clicking beside the
    /// modal puts it away without stopping the model, so other work can go
    /// on while the message is written.
    #[test]
    fn clicking_beside_the_commit_modal_hides_it_and_keeps_generating() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();
        state.repo_root = Some("/repo".into());
        state.modal = Modal::Commit;
        let (_tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(|| {});
        state.start_generation(rx, handle, crate::panel::commit_art::Feed::from_diff(""));

        assert!(handle_modal_mouse(&mut state, area, &left_click(0, 0)));

        assert_eq!(state.modal, Modal::None);
        assert!(state.generation.is_some(), "the model keeps writing");
        assert!(
            state
                .commit_draft
                .as_ref()
                .is_some_and(|d| d.dir == "/repo" && !d.ready),
            "the draft is listed under its checkout"
        );
        state.cancel_generation();
    }

    #[test]
    fn commit_modal_mouse_still_places_cursor_and_consumes_click() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();
        state.modal = Modal::Commit;
        state.commit_message = "one\ntwo".into();

        // One column into the second line, which places the cursor after
        // the first character of "two".
        let body = crate::panel::commit::editor_body_area(area);
        let click = left_click(body.x + 1, body.y + 1);

        assert!(handle_modal_mouse(&mut state, area, &click));

        assert_eq!(state.commit_cursor, 5);
    }

    #[test]
    fn wheel_over_staged_files_scrolls_the_list_and_stops_at_the_ends() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();
        state.modal = Modal::Commit;
        state.files = (0..80)
            .map(|i| crate::git::FileEntry {
                path: format!("src/file{i:02}.rs"),
                x: 'M',
                y: ' ',
            })
            .collect();
        let pane = crate::panel::commit::staged_area(area);
        let wheel = |kind| MouseEvent {
            kind,
            column: pane.x + 2,
            row: pane.y + 2,
            modifiers: KeyModifiers::NONE,
        };

        assert!(handle_modal_mouse(
            &mut state,
            area,
            &wheel(MouseEventKind::ScrollDown)
        ));
        assert!(state.commit_files_scroll > 0, "the list moved down");

        for _ in 0..1_000 {
            handle_modal_mouse(&mut state, area, &wheel(MouseEventKind::ScrollDown));
        }
        let visible = pane.height as usize;
        // 80 files plus the folder row, and the last of them stays on screen.
        assert_eq!(state.commit_files_scroll, 81 - visible);

        for _ in 0..1_000 {
            handle_modal_mouse(&mut state, area, &wheel(MouseEventKind::ScrollUp));
        }
        assert_eq!(state.commit_files_scroll, 0);
    }

    #[test]
    fn wheel_over_the_message_leaves_the_file_list_alone() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();
        state.modal = Modal::Commit;
        state.files = (0..80)
            .map(|i| crate::git::FileEntry {
                path: format!("f{i}.rs"),
                x: 'M',
                y: ' ',
            })
            .collect();
        let body = crate::panel::commit::editor_body_area(area);
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: body.x + 1,
            row: body.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        handle_modal_mouse(&mut state, area, &wheel);
        assert_eq!(state.commit_files_scroll, 0);
    }

    #[test]
    fn mouse_is_not_consumed_without_modal() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();

        assert!(!handle_modal_mouse(&mut state, area, &left_click(80, 10)));
    }
}
