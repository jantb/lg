use anyhow::Result;
use ratatui::{
    backend::Backend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};

use crate::{
    panel,
    state::{AppState, Modal, Pane, PendingAction},
    ui,
};

use super::{
    App, HeadlessApp, mouse, open_author_modal, selected_commit_ref, spawn_assisted_review,
    spawn_pull, spawn_push,
};

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

fn handle_modal_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> bool {
    match state.modal {
        Modal::None => false,
        Modal::Commit => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                let _ = panel::commit::place_cursor_at(state, area, m.column, m.row);
            }
            true
        }
        _ => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            true
        }
    }
}

impl<B: Backend> HeadlessApp<B>
where
    B::Error: Send + Sync + 'static,
{
    pub fn send_key(&mut self, k: KeyEvent) -> Result<()> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return self.render();
        }
        match self.state.modal {
            Modal::Help => {
                panel::help::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Commit => {
                panel::commit::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::StageAllBeforeCommit => {
                panel::stage_all::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Push => {
                panel::push::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Author => {
                panel::author::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Flow => {
                panel::flow::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Conflict => {
                panel::conflict::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::DeleteBranch => {
                panel::delete_branch::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::ReviewChat => {
                panel::review_chat::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::None => {}
        }
        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
            }
            KeyCode::Char('F') => {
                if self.state.flow_available() {
                    self.state.modal = Modal::Flow;
                }
            }
            KeyCode::Char('q') => {
                self.state.should_quit = true;
            }
            KeyCode::Esc => {}
            KeyCode::Char('1') => {
                self.state.focus = Pane::Status;
            }
            KeyCode::Char('2') => {
                self.state.focus = Pane::Files;
            }
            KeyCode::Char('3') => {
                self.state.focus = Pane::Branches;
            }
            KeyCode::Char('4') => {
                self.state.focus = Pane::Commits;
            }
            KeyCode::Char('0') => {
                self.state.focus = Pane::Main;
            }
            KeyCode::Tab => {
                self.state.focus = next_pane(self.state.focus);
            }
            KeyCode::BackTab => {
                self.state.focus = prev_pane(self.state.focus);
            }
            KeyCode::Char('c') => {
                self.state.open_commit_or_stage_all_prompt();
            }
            KeyCode::Char('a') => {
                open_author_modal(&mut self.state);
            }
            KeyCode::Char('p') => {
                if self.state.pull_available() {
                    self.state.pending_action = Some(PendingAction::Pull);
                } else {
                    self.state.set_status("nothing to pull", false);
                }
            }
            KeyCode::Char('f') => {
                self.state
                    .set_status("fetch unavailable in headless", false);
            }
            KeyCode::Char('P') => {
                if !self.state.has_unpushed_commits() {
                    self.state.set_status("nothing to push", false);
                } else {
                    spawn_push(&mut self.state);
                }
            }
            KeyCode::Char('R') => {
                spawn_assisted_review(&mut self.state);
            }
            _ => match self.state.focus {
                Pane::Status => panel::environments::handle_key(&mut self.state, k)?,
                Pane::Files => panel::files::handle_key(&mut self.state, k)?,
                Pane::Branches => panel::branches::handle_key(&mut self.state, k)?,
                Pane::Commits => panel::commits::handle_key(&mut self.state, k)?,
                Pane::Main => panel::main::handle_key(&mut self.state, k)?,
            },
        }
        self.render()
    }
}

impl App {
    pub(super) fn handle_key(&mut self, k: KeyEvent) -> Result<()> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return Ok(());
        }

        match self.state.modal {
            Modal::Help => {
                panel::help::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Commit => {
                panel::commit::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::StageAllBeforeCommit => {
                panel::stage_all::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Push => {
                panel::push::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Author => {
                panel::author::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Flow => {
                panel::flow::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Conflict => {
                panel::conflict::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::DeleteBranch => {
                panel::delete_branch::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::ReviewChat => {
                panel::review_chat::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::None => {}
        }

        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
                return Ok(());
            }
            KeyCode::Char('F') => {
                self.start_refresh(false);
                if self.state.flow_available() {
                    self.state.modal = Modal::Flow;
                }
                return Ok(());
            }
            KeyCode::Char('q') => {
                self.state.should_quit = true;
                return Ok(());
            }
            KeyCode::Esc => {
                return Ok(());
            }
            KeyCode::Char('1') => {
                self.state.focus = Pane::Status;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('2') => {
                self.state.focus = Pane::Files;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('3') => {
                self.state.focus = Pane::Branches;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('4') => {
                self.state.focus = Pane::Commits;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('0') => {
                self.state.focus = Pane::Main;
                return Ok(());
            }
            KeyCode::Tab => {
                self.state.focus = next_pane(self.state.focus);
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::BackTab => {
                self.state.focus = prev_pane(self.state.focus);
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('c') => {
                self.state.open_commit_or_stage_all_prompt();
                return Ok(());
            }
            KeyCode::Char('a') => {
                open_author_modal(&mut self.state);
                return Ok(());
            }
            KeyCode::Char('p') => {
                spawn_pull(&mut self.state);
                return Ok(());
            }
            KeyCode::Char('f') => {
                self.start_fetch();
                return Ok(());
            }
            KeyCode::Char('P') => {
                if !self.state.has_unpushed_commits() {
                    self.state.set_status("nothing to push", false);
                    return Ok(());
                }
                spawn_push(&mut self.state);
                return Ok(());
            }
            KeyCode::Char('R') => {
                spawn_assisted_review(&mut self.state);
                return Ok(());
            }
            _ => {}
        }

        let focus_before = self.state.focus;
        let commit_ref_before = selected_commit_ref(&self.state);

        match focus_before {
            Pane::Status => panel::environments::handle_key(&mut self.state, k)?,
            Pane::Files => panel::files::handle_key(&mut self.state, k)?,
            Pane::Branches => panel::branches::handle_key(&mut self.state, k)?,
            Pane::Commits => panel::commits::handle_key(&mut self.state, k)?,
            Pane::Main => panel::main::handle_key(&mut self.state, k)?,
        }

        if self.state.pending_action.is_none()
            && (matches!(focus_before, Pane::Files | Pane::Branches | Pane::Commits)
                || matches!(
                    self.state.focus,
                    Pane::Files | Pane::Branches | Pane::Commits
                ))
        {
            self.start_diff_job(false);
        }
        if selected_commit_ref(&self.state) != commit_ref_before {
            self.sync_commit_log_to_selection();
        }
        Ok(())
    }

    pub(super) fn handle_mouse(&mut self, m: MouseEvent) -> Result<()> {
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        if handle_modal_mouse(&mut self.state, area, &m) {
            return Ok(());
        }

        let rects = ui::split_layout_with_sizes(
            area,
            self.state.flow_available(),
            self.state.left_column_width,
            self.state.left_panel_heights,
        );
        let divider_col = rects.main.x.saturating_sub(1);
        let on_divider = m.row >= rects.status.y
            && m.row < rects.footer.y
            && (m.column == divider_col || m.column == rects.main.x);

        match m.kind {
            MouseEventKind::Down(MouseButton::Left)
                if on_divider && !m.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.state.column_drag_active = true;
                self.state.row_drag_active = None;
                self.state.left_column_width = Some(ui::clamp_left_column_width(
                    rects.status.width.saturating_add(rects.main.width),
                    m.column.saturating_sub(area.x).saturating_add(1),
                ));
                return Ok(());
            }
            MouseEventKind::Drag(MouseButton::Left) if self.state.column_drag_active => {
                self.state.left_column_width = Some(ui::clamp_left_column_width(
                    rects.status.width.saturating_add(rects.main.width),
                    m.column.saturating_sub(area.x).saturating_add(1),
                ));
                return Ok(());
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.state.column_drag_active = false;
                self.state.row_drag_active = None;
                return Ok(());
            }
            _ => {}
        }

        let show_environments = self.state.flow_available();
        match m.kind {
            MouseEventKind::Down(MouseButton::Left)
                if !m.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                if let Some(pair) =
                    mouse::row_divider_pair_at(&rects, show_environments, m.column, m.row)
                {
                    self.state.column_drag_active = false;
                    self.state.row_drag_active = Some(pair);
                    self.state.left_panel_heights = Some(mouse::current_left_panel_heights(&rects));
                    mouse::resize_left_panel_pair(
                        &mut self.state,
                        &rects,
                        pair,
                        m.row,
                        show_environments,
                    );
                    return Ok(());
                }

                if let Some(pane) = mouse::pane_at(&rects, m.column, m.row) {
                    let commit_ref_before = selected_commit_ref(&self.state);
                    self.state.focus = pane;
                    mouse::select_mouse_row(&mut self.state, pane, &rects, m.row);
                    if !matches!(pane, Pane::Main) {
                        self.start_diff_job(false);
                    }
                    if selected_commit_ref(&self.state) != commit_ref_before {
                        self.sync_commit_log_to_selection();
                    }
                    return Ok(());
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(pair) = self.state.row_drag_active {
                    mouse::resize_left_panel_pair(
                        &mut self.state,
                        &rects,
                        pair,
                        m.row,
                        show_environments,
                    );
                    return Ok(());
                }
            }
            _ => {}
        }

        if matches!(
            m.kind,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        ) {
            if let Some(pane @ (Pane::Status | Pane::Files | Pane::Branches | Pane::Commits)) =
                mouse::pane_at(&rects, m.column, m.row)
            {
                let focus_before = self.state.focus;
                let commit_ref_before = selected_commit_ref(&self.state);
                self.state.focus = pane;
                let changed = mouse::scroll_list(
                    &mut self.state,
                    pane,
                    matches!(m.kind, MouseEventKind::ScrollDown),
                    3,
                );
                if changed || focus_before != pane {
                    self.start_diff_job(false);
                }
                if selected_commit_ref(&self.state) != commit_ref_before {
                    self.sync_commit_log_to_selection();
                }
                return Ok(());
            }
        }

        let in_main = m.column >= rects.main.x
            && m.column < rects.main.x + rects.main.width
            && m.row >= rects.main.y
            && m.row < rects.main.y + rects.main.height;
        if !in_main {
            return Ok(());
        }
        match m.kind {
            MouseEventKind::ScrollDown => {
                panel::main::scroll(&mut self.state, true, 3);
            }
            MouseEventKind::ScrollUp => {
                panel::main::scroll(&mut self.state, false, 3);
            }
            _ => {}
        }
        Ok(())
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

    #[test]
    fn commit_modal_mouse_still_places_cursor_and_consumes_click() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();
        state.modal = Modal::Commit;
        state.commit_message = "one\ntwo".into();

        assert!(handle_modal_mouse(&mut state, area, &left_click(12, 6)));

        assert_eq!(state.commit_cursor, 5);
    }

    #[test]
    fn mouse_is_not_consumed_without_modal() {
        let area = Rect::new(0, 0, 100, 30);
        let mut state = AppState::new();

        assert!(!handle_modal_mouse(&mut state, area, &left_click(80, 10)));
    }
}
