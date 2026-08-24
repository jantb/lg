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
    App, HeadlessApp, mouse, open_author_modal, open_model_modal, selected_commit_ref,
    spawn_assisted_review, spawn_pull, spawn_push,
};

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

fn handle_modal_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> bool {
    match state.modal {
        Modal::None => false,
        Modal::Help => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            state.review_chat_drag_active = false;
            match m.kind {
                MouseEventKind::ScrollDown => panel::help::scroll(state, area, true, 3),
                MouseEventKind::ScrollUp => panel::help::scroll(state, area, false, 3),
                _ => {}
            }
            true
        }
        Modal::ReviewChat if review_chat_is_docked(state) => false,
        Modal::Commit => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            state.review_chat_drag_active = false;
            if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                let _ = panel::commit::place_cursor_at(state, area, m.column, m.row);
            }
            true
        }
        _ => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            state.review_chat_drag_active = false;
            true
        }
    }
}

fn review_chat_is_docked(state: &AppState) -> bool {
    state.modal == Modal::ReviewChat
}

fn focused_review_panel(state: &AppState) -> bool {
    state.focus == Pane::Main
        && matches!(state.diff_source, crate::state::DiffSource::Review)
        && state.review.is_some()
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn resize_review_chat(state: &mut AppState, main: Rect, row: u16) {
    let min_review_height = 6.min(main.height);
    let max_chat_height = main.height.saturating_sub(min_review_height);
    let min_chat_height = 5.min(max_chat_height);
    let bottom = main.y.saturating_add(main.height);
    let chat_height = bottom
        .saturating_sub(row)
        .max(min_chat_height)
        .min(max_chat_height);
    state.review_chat_height = Some(chat_height);
}

fn handle_docked_review_chat_mouse(
    state: &mut AppState,
    rects: &ui::LayoutRects,
    m: &MouseEvent,
) -> bool {
    if !review_chat_is_docked(state) {
        return false;
    }

    let chunks = panel::main::review_chat_layout(state, rects.main);
    let chat_area = chunks[1];
    let in_main = rect_contains(rects.main, m.column, m.row);
    let in_chat = rect_contains(chat_area, m.column, m.row);
    let on_splitter = in_main && (m.row == chat_area.y || m.row.saturating_add(1) == chat_area.y);

    match m.kind {
        MouseEventKind::Down(MouseButton::Left)
            if on_splitter && !m.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            state.column_drag_active = false;
            state.row_drag_active = None;
            state.review_chat_drag_active = true;
            resize_review_chat(state, rects.main, m.row);
            true
        }
        MouseEventKind::Drag(MouseButton::Left) if state.review_chat_drag_active => {
            resize_review_chat(state, rects.main, m.row);
            true
        }
        MouseEventKind::Up(MouseButton::Left) if state.review_chat_drag_active => {
            state.review_chat_drag_active = false;
            true
        }
        MouseEventKind::ScrollDown if in_chat => {
            panel::review_chat::scroll(state, true, 3);
            true
        }
        MouseEventKind::ScrollUp if in_chat => {
            panel::review_chat::scroll(state, false, 3);
            true
        }
        _ => in_chat,
    }
}

fn handle_review_mouse_scroll(state: &mut AppState, m: &MouseEvent) -> bool {
    if !matches!(state.diff_source, crate::state::DiffSource::Review) || state.review.is_none() {
        return false;
    }
    match m.kind {
        MouseEventKind::ScrollDown => {
            panel::main::scroll(state, true, 3);
            true
        }
        MouseEventKind::ScrollUp => {
            panel::main::scroll(state, false, 3);
            true
        }
        _ => false,
    }
}

/// What key and mouse dispatch needs from the app around it. The real app kicks
/// off background work; the headless one only moves state, and that is the whole
/// difference between the two — the decision table itself is shared.
pub(super) trait AppHost {
    fn state(&self) -> &AppState;
    fn state_mut(&mut self) -> &mut AppState;
    fn area(&self) -> Result<Rect>;

    /// Load the diff for whatever focus now sits on, revealing it when that
    /// pane's selection drives one.
    fn diff_for_focus(&mut self);
    /// Reload the commit list for the newly selected ref.
    fn sync_commit_log(&mut self);
    fn start_fetch(&mut self);
    fn start_pull(&mut self);
    fn cycle_session(&mut self, forward: bool);
    /// Hand the keyboard back to lg from a session holding it.
    fn release_session_keyboard(&mut self);
    fn before_flow_modal(&mut self);
}

/// Give the main pane back to the diff when focus sits on a pane whose selection
/// drives one. The repository pane is left alone: it is where a session is
/// picked, so focusing it must not background one.
fn reveals_diff(state: &AppState) -> bool {
    matches!(state.focus, Pane::Files | Pane::Branches | Pane::Commits)
}

impl AppHost for App {
    fn state(&self) -> &AppState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    fn area(&self) -> Result<Rect> {
        self.terminal_area()
    }

    fn diff_for_focus(&mut self) {
        if reveals_diff(&self.state) {
            self.state.background_session_for_diff();
        }
        self.start_diff_job(false);
    }

    fn sync_commit_log(&mut self) {
        self.sync_commit_log_to_selection();
    }

    fn start_fetch(&mut self) {
        App::start_fetch(self);
    }

    fn start_pull(&mut self) {
        if self.state.pull_available() {
            spawn_pull(&mut self.state);
        } else {
            self.state.set_status("nothing to pull", false);
        }
    }

    fn cycle_session(&mut self, forward: bool) {
        App::cycle_session(self, forward);
    }

    fn release_session_keyboard(&mut self) {
        self.set_session_capture(false);
        self.state
            .set_status("keyboard back in lg \u{2014} i returns it", false);
    }

    fn before_flow_modal(&mut self) {
        self.start_refresh(false);
    }
}

impl<B: Backend> AppHost for HeadlessApp<B>
where
    B::Error: Send + Sync + 'static,
{
    fn state(&self) -> &AppState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    fn area(&self) -> Result<Rect> {
        self.terminal_area()
    }

    fn diff_for_focus(&mut self) {
        if reveals_diff(&self.state) {
            self.state.background_session_for_diff();
        }
    }

    fn sync_commit_log(&mut self) {}

    fn start_fetch(&mut self) {
        self.state
            .set_status("fetch unavailable in headless", false);
    }

    fn start_pull(&mut self) {
        if self.state.pull_available() {
            self.state.pending_action = Some(PendingAction::Pull);
        } else {
            self.state.set_status("nothing to pull", false);
        }
    }

    fn cycle_session(&mut self, forward: bool) {
        super::session::cycle(&mut self.state, forward);
    }

    fn release_session_keyboard(&mut self) {
        self.state.session_capture = false;
    }

    fn before_flow_modal(&mut self) {}
}

/// Route one key press. Modals get it first, then lg's global bindings, then the
/// focused pane.
fn dispatch_key<H: AppHost>(host: &mut H, k: KeyEvent) -> Result<()> {
    // A session holding the keyboard sees everything, Ctrl-C included —
    // interrupting the program inside it matters more than quitting lg, which
    // Ctrl-] then q still does.
    if host.state().session_input_active() {
        if super::session::is_release_key(&k) {
            host.release_session_keyboard();
            return Ok(());
        }
        super::session::forward_key(host.state_mut(), k);
        return Ok(());
    }
    if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
        host.state_mut().request_quit();
        return Ok(());
    }

    match host.state().modal {
        Modal::Help => {
            let area = host.area()?;
            panel::help::handle_key(host.state_mut(), k, area)?;
            return Ok(());
        }
        Modal::Commit => {
            panel::commit::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::StageAllBeforeCommit => {
            panel::stage_all::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Push => {
            panel::push::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Author => {
            panel::author::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Model => {
            panel::model::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Flow => {
            panel::flow::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Conflict => {
            panel::conflict::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::DeleteBranch => {
            panel::delete_branch::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Worktree => {
            panel::worktree::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::ConfirmDestructive => {
            panel::confirm::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::ReviewChat => {
            panel::review_chat::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::None => {}
    }

    match k.code {
        KeyCode::F(2) => {
            host.state_mut().toggle_mode();
            return Ok(());
        }
        KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            host.cycle_session(true);
            return Ok(());
        }
        KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            host.cycle_session(false);
            return Ok(());
        }
        KeyCode::Char('?') => {
            let focus = host.state().focus;
            let state = host.state_mut();
            state.prev_focus = focus;
            state.help_offset = 0;
            state.modal = Modal::Help;
            return Ok(());
        }
        KeyCode::Char('F') => {
            if host.state().focus == Pane::Branches && host.state().branch_actions_available() {
                host.before_flow_modal();
                host.state_mut().modal = Modal::Flow;
            } else {
                let reason = flow_unavailable_reason(host.state());
                host.state_mut().set_status(reason, false);
            }
            return Ok(());
        }
        KeyCode::Char('q') => {
            host.state_mut().request_quit();
            return Ok(());
        }
        KeyCode::Esc if host.state().status.as_ref().is_some_and(|s| s.is_error) => {
            host.state_mut().status = None;
            return Ok(());
        }
        KeyCode::Esc if host.state().llm_job_running() => {
            if let Some(message) = host.state_mut().cancel_llm_jobs() {
                host.state_mut().set_status(message, false);
            }
            return Ok(());
        }
        KeyCode::Esc if host.state().focus == Pane::Status => {
            panel::environments::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        KeyCode::Esc => {
            return Ok(());
        }
        KeyCode::Char('1') => {
            host.state_mut().focus_pane(Pane::Status);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('2') => {
            host.state_mut().focus_pane(Pane::Files);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('3') => {
            host.state_mut().focus_pane(Pane::Branches);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('4') => {
            host.state_mut().focus_pane(Pane::Commits);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('0') => {
            host.state_mut().focus_pane(Pane::Main);
            return Ok(());
        }
        KeyCode::Tab => {
            host.state_mut().focus = cycle_pane(host.state(), true);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::BackTab => {
            host.state_mut().focus = cycle_pane(host.state(), false);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('c') => {
            host.state_mut().open_commit_or_stage_all_prompt();
            return Ok(());
        }
        KeyCode::Char('a') => {
            open_author_modal(host.state_mut());
            return Ok(());
        }
        KeyCode::Char('L') => {
            open_model_modal(host.state_mut());
            return Ok(());
        }
        KeyCode::Char('p') => {
            host.start_pull();
            return Ok(());
        }
        KeyCode::Char('f') if focused_review_panel(host.state()) => {
            panel::main::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        KeyCode::Char('f') => {
            host.start_fetch();
            return Ok(());
        }
        KeyCode::Char('P') => {
            if !host.state().has_unpushed_commits() {
                host.state_mut().set_status("nothing to push", false);
                return Ok(());
            }
            spawn_push(host.state_mut());
            return Ok(());
        }
        KeyCode::Char('R') => {
            spawn_assisted_review(host.state_mut());
            return Ok(());
        }
        _ => {}
    }

    let focus_before = host.state().focus;
    let commit_ref_before = selected_commit_ref(host.state());

    match focus_before {
        Pane::Status => panel::environments::handle_key(host.state_mut(), k)?,
        Pane::Files => panel::files::handle_key(host.state_mut(), k)?,
        Pane::Branches => panel::branches::handle_key(host.state_mut(), k)?,
        Pane::Commits => panel::commits::handle_key(host.state_mut(), k)?,
        Pane::Main => panel::main::handle_key(host.state_mut(), k)?,
    }

    if host.state().pending_action.is_none()
        && (matches!(focus_before, Pane::Files | Pane::Branches | Pane::Commits)
            || reveals_diff(host.state()))
    {
        host.diff_for_focus();
    }
    if selected_commit_ref(host.state()) != commit_ref_before {
        host.sync_commit_log();
    }
    Ok(())
}

/// Route one mouse event. Modals and the docked review chat get it first, then
/// the pane dividers, then whichever pane it landed in.
fn dispatch_mouse<H: AppHost>(host: &mut H, m: MouseEvent) -> Result<()> {
    let area = host.area()?;
    if handle_modal_mouse(host.state_mut(), area, &m) {
        return Ok(());
    }

    let rects = super::render::layout_for(host.state(), area);
    if handle_docked_review_chat_mouse(host.state_mut(), &rects, &m) {
        return Ok(());
    }
    if handle_review_mouse_scroll(host.state_mut(), &m) {
        return Ok(());
    }
    let divider_col = rects.main.x.saturating_sub(1);
    let on_divider = m.row >= rects.status.y
        && m.row < rects.footer.y
        && (m.column == divider_col || m.column == rects.main.x);

    match m.kind {
        MouseEventKind::Down(MouseButton::Left)
            if on_divider && !m.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            let width = ui::clamp_left_column_width(
                rects.status.width.saturating_add(rects.main.width),
                m.column.saturating_sub(area.x).saturating_add(1),
            );
            let state = host.state_mut();
            state.column_drag_active = true;
            state.row_drag_active = None;
            state.left_column_width = Some(width);
            return Ok(());
        }
        MouseEventKind::Drag(MouseButton::Left) if host.state().column_drag_active => {
            let width = ui::clamp_left_column_width(
                rects.status.width.saturating_add(rects.main.width),
                m.column.saturating_sub(area.x).saturating_add(1),
            );
            host.state_mut().left_column_width = Some(width);
            return Ok(());
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let state = host.state_mut();
            state.column_drag_active = false;
            state.row_drag_active = None;
            state.review_chat_drag_active = false;
            return Ok(());
        }
        _ => {}
    }

    let show_environments = host.state().environments_visible();
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if !m.modifiers.contains(KeyModifiers::SHIFT) => {
            if let Some(pair) =
                mouse::row_divider_pair_at(&rects, show_environments, m.column, m.row)
            {
                let heights = mouse::current_left_panel_heights(&rects);
                let state = host.state_mut();
                state.column_drag_active = false;
                state.row_drag_active = Some(pair);
                state.left_panel_heights = Some(heights);
                mouse::resize_left_panel_pair(
                    host.state_mut(),
                    &rects,
                    pair,
                    m.row,
                    show_environments,
                );
                return Ok(());
            }

            if let Some(pane) = mouse::pane_at(&rects, m.column, m.row) {
                let commit_ref_before = selected_commit_ref(host.state());
                host.state_mut().focus = pane;
                mouse::select_mouse_row(host.state_mut(), pane, &rects, m.row);
                if !matches!(pane, Pane::Main) {
                    host.diff_for_focus();
                }
                if selected_commit_ref(host.state()) != commit_ref_before {
                    host.sync_commit_log();
                }
                return Ok(());
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(pair) = host.state().row_drag_active {
                mouse::resize_left_panel_pair(
                    host.state_mut(),
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
    ) && let Some(pane @ (Pane::Status | Pane::Files | Pane::Branches | Pane::Commits)) =
        mouse::pane_at(&rects, m.column, m.row)
    {
        let focus_before = host.state().focus;
        let commit_ref_before = selected_commit_ref(host.state());
        host.state_mut().focus = pane;
        let changed = mouse::scroll_list(
            host.state_mut(),
            pane,
            matches!(m.kind, MouseEventKind::ScrollDown),
            3,
        );
        if changed || focus_before != pane {
            host.diff_for_focus();
        }
        if selected_commit_ref(host.state()) != commit_ref_before {
            host.sync_commit_log();
        }
        return Ok(());
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
            panel::main::scroll(host.state_mut(), true, 3);
        }
        MouseEventKind::ScrollUp => {
            panel::main::scroll(host.state_mut(), false, 3);
        }
        _ => {}
    }
    Ok(())
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
