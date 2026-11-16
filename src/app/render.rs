use anyhow::Result;
use ratatui::crossterm::{
    cursor::Hide,
    execute, queue,
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
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
        poll_panels(&mut self.state);
        prepare(&mut self.state, area);
        let mut selected = None;
        self.terminal
            .draw(|frame| selected = draw(frame, &self.state))?;
        copy_selection(&mut self.state, selected);
        Ok(())
    }
}

impl App {
    /// The frame is written with the cursor hidden and inside a synchronized
    /// update. Otherwise a cursor left showing in a text field is dragged
    /// across the screen by every cell the frame rewrites, which a modal's
    /// animated border does thirty times a second.
    pub(super) fn render(&mut self) -> Result<()> {
        let area = terminal_area(self.terminal.size()?);
        prepare(&mut self.state, area);
        let mut selected = None;
        queue!(self.terminal.backend_mut(), BeginSynchronizedUpdate, Hide)?;
        let drawn = self
            .terminal
            .draw(|frame| selected = draw(frame, &self.state))
            .map(|_| ());
        // Ended even when the draw failed, so the terminal does not sit on a
        // held screen.
        execute!(self.terminal.backend_mut(), EndSynchronizedUpdate)?;
        drawn?;
        copy_selection(&mut self.state, selected);
        Ok(())
    }
}

/// Hand a finished selection's text to the clipboard. It has to wait for a
/// frame in which nothing else is pending, and stays highlighted meanwhile.
fn copy_selection(state: &mut AppState, text: Option<String>) {
    let Some(text) = text else {
        return;
    };
    if state.pending_action.is_some() {
        return;
    }
    if let Some(selection) = state.selection.as_mut() {
        selection.copy_requested = false;
    }
    state.pending_action = Some(crate::state::PendingAction::CopyToClipboard {
        label: "selection".into(),
        text,
    });
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

/// Take delivery of what the panels' own workers sent. The loop does this on
/// every pass, whether or not it then draws.
pub(super) fn poll_panels(state: &mut AppState) {
    panel::settings::poll(state);
    panel::github::poll(state);
    panel::guided::poll(state);
    panel::environments::poll_nested_repo_detail(state);
}

/// Everything that has to be measured against the layout before the frame is
/// drawn: viewport sizes, scroll offsets, and the size of a running session.
fn prepare(state: &mut AppState, area: Rect) {
    let rects = layout_for(state, area);
    state.advance_animation();
    super::prepare_conflict_editor(state);
    state.diff_viewport_height = rects.main.height.saturating_sub(2);
    state.diff_viewport_width = rects.main.width.saturating_sub(2);
    state.diff_offset = state.diff_offset.min(panel::main::max_scroll_offset(state));
    sync_selection_scroll_offsets(state, &rects, area);
    resize_focused_session(state, rects.main);
    panel::guided::fit_chat(state, area);
    panel::github::fit_threads_chat(state, rects.main);
}

/// Draw one frame. Returns the text of a selection whose copy is due, read
/// off the frame just drawn, since the cells are the only place it exists.
fn draw(frame: &mut Frame, state: &AppState) -> Option<String> {
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
    if state.github_view() {
        panel::github::render(state, rects.main, frame, focused_pane == Pane::Main);
    } else {
        panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);
    }

    footer::draw(frame, rects.footer, state);

    match state.modal {
        Modal::None => {}
        Modal::Commit => panel::commit::render(state, area, frame),
        Modal::StageAllBeforeCommit => panel::stage_all::render(state, area, frame),
        Modal::Push => panel::push::render(state, area, frame),
        Modal::Author => panel::author::render(state, area, frame),
        Modal::Environments => panel::deployment::render(state, area, frame),
        Modal::Commands => panel::commands::render(state, area, frame),
        Modal::Settings => panel::settings::render(state, area, frame),
        Modal::Model => panel::model::render(state, area, frame),
        Modal::Help => panel::help::render(state, area, frame),
        Modal::Flow => panel::flow::render(state, area, frame),
        Modal::Agent => panel::agent::render(state, area, frame),
        Modal::RepoActions => panel::environments::menu::render(state, area, frame),
        Modal::Conflict => panel::conflict::render(state, area, frame),
        Modal::DeleteBranch => panel::delete_branch::render(state, area, frame),
        Modal::Worktree => panel::worktree::render(state, area, frame),
        Modal::ConfirmDestructive => panel::confirm::render(state, area, frame),
        Modal::GuidedReview => panel::guided::render(state, area, frame),
        Modal::Stash => panel::stash::render(state, area, frame),
        Modal::StatusDetails => panel::details::render(state, area, frame),
    }
    // Last, so a selection inside a modal reads what the modal drew.
    state.selection.and_then(|selection| {
        selection.highlight(frame.buffer_mut());
        selection
            .copy_requested
            .then(|| selection.text(frame.buffer_mut()))
    })
}

/// Keep the program's window the same size as the pane it is drawn in. The
/// resize also makes it repaint, which is how a session that was in the
/// background comes back looking right. A session that is also the review
/// chat is sized by the chat, which is drawn over the pane.
fn resize_focused_session(state: &mut AppState, main: Rect) {
    let Some(id) = state.session_view() else {
        return;
    };
    if panel::guided::chat_on_screen(state) == Some(id)
        || panel::github::threads_chat_on_screen(state) == Some(id)
    {
        return;
    }
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ratatui::backend::TestBackend;

    use super::*;
    use crate::panel::guided::{Guided, Source};

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(started.elapsed() < Duration::from_secs(10), "{what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The review chat can also be the session the main pane shows, under
    /// the review. Only the chat is on screen, so only its size counts:
    /// a program told of a new size on every frame repaints on every frame.
    #[test]
    fn a_review_chat_also_shown_in_the_main_pane_is_resized_only_once() {
        let winches = tempfile::NamedTempFile::new().unwrap();
        let script = format!(
            "trap 'echo x >> {}' WINCH; echo ready; while :; do sleep 0.02; done",
            winches.path().display()
        );
        let spawn = crate::term::Spawn {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script],
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            env_remove: Vec::new(),
        };
        let spec = crate::session::SessionSpec {
            label: "review chat".into(),
            cwd: std::env::temp_dir(),
            sandboxed: false,
            kind: crate::session::SessionKind::Claude,
            prompt: None,
        };
        let mut app = HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
        let state = &mut app.state;
        let id = state.sessions.start_with(spec, &spawn, (24, 80)).unwrap();
        wait_for("the chat never started", || {
            state.sessions.pump();
            state
                .sessions
                .get(id)
                .is_some_and(|session| session.screen().contents().contains("ready"))
        });
        let mut guided = Guided::new(Source::Branch {
            branch: "render-test-branch".into(),
            base_ref: "main".into(),
        });
        guided.steps = crate::git::guided::parse_files(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n",
        );
        guided.chat = Some(id);
        state.guided = Some(Box::new(guided));
        state.main_view = crate::state::MainView::Session(id);
        state.modal = Modal::GuidedReview;

        app.render().unwrap();
        let winched = || {
            std::fs::read_to_string(winches.path())
                .unwrap()
                .lines()
                .count()
        };
        wait_for("the chat was never told its size", || winched() > 0);
        std::thread::sleep(Duration::from_millis(200));
        let after_first_frame = winched();
        for _ in 0..5 {
            app.render().unwrap();
        }
        std::thread::sleep(Duration::from_millis(300));
        let after_more_frames = winched();
        app.state.sessions.close_all();

        assert_eq!(after_more_frames, after_first_frame);
    }
}
