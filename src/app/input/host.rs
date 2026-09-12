//! What key and mouse dispatch needs from the app around it.

use super::*;

/// What key and mouse dispatch needs from the app around it. The real app kicks
/// off background work; the headless one only moves state, and that is the whole
/// difference between the two — the decision table itself is shared.
pub(crate) trait AppHost {
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
pub(super) fn reveals_diff(state: &AppState) -> bool {
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
        crate::app::session::cycle(&mut self.state, forward);
    }

    fn release_session_keyboard(&mut self) {
        self.state.session_capture = false;
    }

    fn before_flow_modal(&mut self) {}
}
