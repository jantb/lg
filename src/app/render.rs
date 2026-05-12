use anyhow::Result;
use ratatui::{backend::Backend, layout::Rect};

use crate::{
    panel,
    state::{Modal, Pane},
    ui,
};

use super::{App, HeadlessApp, footer, header};

impl<B: Backend> HeadlessApp<B>
where
    B::Error: Send + Sync + 'static,
{
    pub fn render(&mut self) -> Result<()> {
        self.state.advance_animation();

        // Compute viewport height before the draw closure so we can update state.
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects_pre = ui::split_layout_with_sizes(
            area,
            self.state.flow_available() || !self.state.nested_repositories.is_empty(),
            self.state.left_column_width,
            self.state.left_panel_heights,
        );
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);
        self.state.diff_viewport_width = rects_pre.main.width.saturating_sub(2);
        self.clamp_main_scroll_offset();
        sync_selection_scroll_offsets(&mut self.state, &rects_pre, area);

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout_with_sizes(
                area,
                state.flow_available() || !state.nested_repositories.is_empty(),
                state.left_column_width,
                state.left_panel_heights,
            );
            let focused_pane = state.focus;

            header::draw(frame, rects.header, state);
            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
            panel::environments::render(state, rects.environments, frame);
            panel::files::render(state, rects.files, frame, focused_pane == Pane::Files);
            panel::branches::render(state, rects.branches, frame, focused_pane == Pane::Branches);
            panel::commits::render(state, rects.commits, frame, focused_pane == Pane::Commits);
            panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);

            footer::draw(frame, rects.footer, state);

            match state.modal {
                Modal::None => {}
                Modal::Commit => panel::commit::render(state, area, frame),
                Modal::StageAllBeforeCommit => panel::stage_all::render(state, area, frame),
                Modal::Push => panel::push::render(state, area, frame),
                Modal::Author => panel::author::render(state, area, frame),
                Modal::Help => panel::help::render(state, area, frame),
                Modal::Flow => panel::flow::render(state, area, frame),
                Modal::Conflict => panel::conflict::render(state, area, frame),
                Modal::DeleteBranch => panel::delete_branch::render(state, area, frame),
                Modal::ReviewChat => panel::review_chat::render(state, area, frame),
            }
        })?;
        Ok(())
    }

    fn clamp_main_scroll_offset(&mut self) {
        self.state.diff_offset = self
            .state
            .diff_offset
            .min(panel::main::max_scroll_offset(&self.state));
    }
}

impl App {
    pub(super) fn render(&mut self) -> Result<()> {
        self.state.advance_animation();

        // Compute viewport height before the draw closure so we can update state.
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects_pre = ui::split_layout_with_sizes(
            area,
            self.state.flow_available() || !self.state.nested_repositories.is_empty(),
            self.state.left_column_width,
            self.state.left_panel_heights,
        );
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);
        self.state.diff_viewport_width = rects_pre.main.width.saturating_sub(2);
        self.clamp_main_scroll_offset();
        sync_selection_scroll_offsets(&mut self.state, &rects_pre, area);

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout_with_sizes(
                area,
                state.flow_available() || !state.nested_repositories.is_empty(),
                state.left_column_width,
                state.left_panel_heights,
            );
            let focused_pane = state.focus;

            header::draw(frame, rects.header, state);
            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
            panel::environments::render(state, rects.environments, frame);
            panel::files::render(state, rects.files, frame, focused_pane == Pane::Files);
            panel::branches::render(state, rects.branches, frame, focused_pane == Pane::Branches);
            panel::commits::render(state, rects.commits, frame, focused_pane == Pane::Commits);
            panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);

            footer::draw(frame, rects.footer, state);

            match state.modal {
                Modal::None => {}
                Modal::Commit => panel::commit::render(state, area, frame),
                Modal::StageAllBeforeCommit => panel::stage_all::render(state, area, frame),
                Modal::Push => panel::push::render(state, area, frame),
                Modal::Author => panel::author::render(state, area, frame),
                Modal::Help => panel::help::render(state, area, frame),
                Modal::Flow => panel::flow::render(state, area, frame),
                Modal::Conflict => panel::conflict::render(state, area, frame),
                Modal::DeleteBranch => panel::delete_branch::render(state, area, frame),
                Modal::ReviewChat => panel::review_chat::render(state, area, frame),
            }
        })?;
        Ok(())
    }

    fn clamp_main_scroll_offset(&mut self) {
        self.state.diff_offset = self
            .state
            .diff_offset
            .min(panel::main::max_scroll_offset(&self.state));
    }
}

fn sync_selection_scroll_offsets(
    state: &mut crate::state::AppState,
    rects: &ui::LayoutRects,
    area: Rect,
) {
    panel::files::sync_scroll_offset(state, rects.files);
    panel::branches::sync_scroll_offset(state, rects.branches);
    panel::commits::sync_scroll_offset(state, rects.commits);

    match state.modal {
        Modal::Commit => panel::commit::sync_scroll_offset(state, area),
        Modal::Flow => panel::flow::sync_scroll_offset(state, area),
        Modal::Conflict => panel::conflict::sync_scroll_offset(state, area),
        _ => {}
    }
}
