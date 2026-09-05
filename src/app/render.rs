use anyhow::Result;
use ratatui::{Frame, backend::Backend, layout::Rect};

use crate::{
    panel,
    state::{AppMode, AppState, Modal, Pane},
    ui,
};

use super::{App, HeadlessApp, footer, header};

impl<B: Backend> HeadlessApp<B>
where
    B::Error: Send + Sync + 'static,
{
    pub fn render(&mut self) -> Result<()> {
        let area = terminal_area(self.terminal.size()?);
        prepare(&mut self.state, area);
        let state = &self.state;
        self.terminal.draw(|frame| draw(frame, state))?;
        Ok(())
    }
}

impl App {
    pub(super) fn render(&mut self) -> Result<()> {
        let area = terminal_area(self.terminal.size()?);
        prepare(&mut self.state, area);
        let state = &self.state;
        self.terminal.draw(|frame| draw(frame, state))?;
        Ok(())
    }
}

fn terminal_area(size: ratatui::layout::Size) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: size.width,
        height: size.height,
    }
}

/// The rectangles for the mode lg is in. Used before drawing, to size the things
/// that live in state, and again while drawing.
pub(super) fn layout_for(state: &AppState, area: Rect) -> ui::LayoutRects {
    match state.mode {
        AppMode::Git => ui::split_layout_with_sizes(
            area,
            state.environments_visible(),
            state.left_column_width,
            state.left_panel_heights,
        ),
        AppMode::Workspace => ui::split_workspace_layout(area, state.left_column_width),
    }
}

/// Everything that has to be measured against the layout before the frame is
/// drawn: viewport sizes, scroll offsets, and the size of a running session.
fn prepare(state: &mut AppState, area: Rect) {
    let rects = layout_for(state, area);
    state.advance_animation();
    state.diff_viewport_height = if state.modal == Modal::ReviewChat {
        panel::main::review_chat_layout(state, rects.main)[0]
            .height
            .saturating_sub(2)
    } else {
        rects.main.height.saturating_sub(2)
    };
    state.diff_viewport_width = rects.main.width.saturating_sub(2);
    state.diff_offset = state.diff_offset.min(panel::main::max_scroll_offset(state));
    sync_selection_scroll_offsets(state, &rects, area);
    resize_focused_session(state, rects.main);
}

fn draw(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let rects = layout_for(state, area);
    let focused_pane = state.focus;

    header::draw(frame, rects.header, state);
    panel::environments::render(
        state,
        rects.environments,
        frame,
        focused_pane == Pane::Status,
    );
    if state.git_panes_visible() {
        panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
        panel::files::render(state, rects.files, frame, focused_pane == Pane::Files);
        panel::branches::render(state, rects.branches, frame, focused_pane == Pane::Branches);
        panel::commits::render(state, rects.commits, frame, focused_pane == Pane::Commits);
    }
    panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);

    footer::draw(frame, rects.footer, state);

    match state.modal {
        Modal::None => {}
        Modal::Commit => panel::commit::render(state, area, frame),
        Modal::StageAllBeforeCommit => panel::stage_all::render(state, area, frame),
        Modal::Push => panel::push::render(state, area, frame),
        Modal::Author => panel::author::render(state, area, frame),
        Modal::Model => panel::model::render(state, area, frame),
        Modal::Help => panel::help::render(state, area, frame),
        Modal::Flow => panel::flow::render(state, area, frame),
        Modal::Agent => panel::agent::render(state, area, frame),
        Modal::Conflict => panel::conflict::render(state, area, frame),
        Modal::DeleteBranch => panel::delete_branch::render(state, area, frame),
        Modal::Worktree => panel::worktree::render(state, area, frame),
        Modal::ConfirmDestructive => panel::confirm::render(state, area, frame),
        Modal::ReviewChat => {}
    }
}

/// Keep the program's window the same size as the pane it is drawn in. The
/// resize also makes it repaint, which is how a session that was in the
/// background comes back looking right.
fn resize_focused_session(state: &mut AppState, main: Rect) {
    let Some(id) = state.session_view() else {
        return;
    };
    let (rows, cols) = crate::session::size_for_pane(main);
    if let Some(session) = state.sessions.get_mut(id) {
        session.resize(rows, cols);
    }
}

fn sync_selection_scroll_offsets(state: &mut AppState, rects: &ui::LayoutRects, area: Rect) {
    panel::environments::sync_scroll_offset(state, rects.environments);
    if state.git_panes_visible() {
        panel::files::sync_scroll_offset(state, rects.files);
        panel::branches::sync_scroll_offset(state, rects.branches);
        panel::commits::sync_scroll_offset(state, rects.commits);
    }

    match state.modal {
        Modal::Commit => panel::commit::sync_scroll_offset(state, area),
        Modal::Flow => panel::flow::sync_scroll_offset(state, area),
        Modal::Conflict => panel::conflict::sync_scroll_offset(state, area),
        _ => {}
    }
}
