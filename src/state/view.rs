//! Which panes are on screen, which one has focus, and what the main pane shows.

use std::path::Path;

use super::{AppState, Modal, TreeRow, build_tree_rows};

/// Which shape lg is in. Git mode is the full git view; workspace mode trades
/// the git panes for one tall list of checkouts and their sessions, with the
/// focused session filling the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Git,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Status,
    Files,
    Branches,
    Commits,
    Main,
}

/// Which of the main pane's three key sets is live. [`MainView`] says whether a
/// session is up; this folds in review mode, which the diff source decides. The
/// footer, the help overlay and the unbound-key hint all read it, so they cannot
/// end up describing different keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainKeys {
    Diff,
    Review,
    /// A terminal session, which has its own keys and its own way out.
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    None,
    All,
    File(String),   // path
    Folder(String), // folder prefix (no trailing slash)
    Commit(String), // sha
    Branch(String), // branch name
    Review,
}

/// What the main pane is showing. The diff sources say *what* is diffed; this
/// says whether a diff is what is on screen at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainView {
    Diff,
    Session(crate::session::SessionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffViewMode {
    Unified,
    SideBySide,
}

impl AppState {
    /// Replace the main pane's text and keep the line count in step. Scrolling
    /// is bounded by that count, so the two must not drift apart.
    pub fn set_diff_text(&mut self, text: String) {
        self.diff_text = text;
        self.diff_line_count = self.diff_text.lines().count().min(u16::MAX as usize) as u16;
    }

    pub fn file_counts(&self) -> (usize, usize, usize) {
        self.files
            .iter()
            .fold((0, 0, 0), |(staged, unstaged, untracked), f| {
                (
                    staged + usize::from(f.x != ' ' && f.x != '?'),
                    unstaged + usize::from(f.y != ' ' && f.y != '?'),
                    untracked + usize::from(f.x == '?' || f.y == '?'),
                )
            })
    }

    pub fn tree_rows(&self) -> Vec<TreeRow> {
        build_tree_rows(&self.files, &self.collapsed_dirs)
    }

    /// The session the main pane is showing, if it is showing one and that
    /// session still exists.
    pub fn session_view(&self) -> Option<crate::session::SessionId> {
        match self.main_view {
            MainView::Session(id) if self.sessions.get(id).is_some() => Some(id),
            _ => None,
        }
    }

    /// Whether keys typed right now belong to a session rather than to lg.
    pub fn session_input_active(&self) -> bool {
        self.modal == Modal::None
            && self.focus == Pane::Main
            && self.session_capture
            && self.session_view().is_some()
    }

    /// Swap between the git view and the session view. Workspace mode starts on
    /// the tree so the checkouts are there to pick from, and going back to git
    /// mode leaves the sessions running.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            AppMode::Git => AppMode::Workspace,
            AppMode::Workspace => AppMode::Git,
        };
        if self.mode == AppMode::Workspace {
            // A session already on screen keeps the focus; otherwise the tree
            // is the only thing worth pointing at.
            if self.session_view().is_none() {
                self.focus = Pane::Status;
            }
        } else if self.focus != Pane::Main {
            self.focus = Pane::Status;
        }
    }

    /// Point the focus at a pane, unless that pane is not on screen in this
    /// mode — the numbered focus keys then do nothing rather than focusing
    /// something invisible.
    pub fn focus_pane(&mut self, pane: Pane) -> bool {
        if !self.git_panes_visible() && !matches!(pane, Pane::Status | Pane::Main) {
            return false;
        }
        self.focus = pane;
        true
    }

    /// Whether the git panes are on screen at all.
    pub fn git_panes_visible(&self) -> bool {
        self.mode == AppMode::Git
    }

    /// Show a session in the main pane, and hand it the keyboard.
    pub fn show_session(&mut self, id: crate::session::SessionId) {
        self.sessions.focus(id);
        self.main_view = MainView::Session(id);
        self.focus = Pane::Main;
    }

    /// Go back to the diff, releasing the keyboard.
    pub fn show_diff(&mut self) {
        self.main_view = MainView::Diff;
        self.session_capture = false;
    }

    /// Give the main pane back to the diff for a newly selected file, branch or
    /// commit. A session drawn there goes to the background: it keeps running,
    /// and Ctrl-N returns to it. Returns whether one was backgrounded.
    pub fn background_session_for_diff(&mut self) -> bool {
        if self.session_view().is_none() {
            return false;
        }
        self.show_diff();
        self.set_status(
            "session in the background \u{2014} Ctrl-N returns to it",
            false,
        );
        true
    }

    /// Whether this repository is checked out in more than one place, which is
    /// what gives the repository tree worktree rows to show.
    pub fn has_linked_worktrees(&self) -> bool {
        self.worktrees.len() > 1
    }

    pub fn environments_visible(&self) -> bool {
        self.flow_available()
            || !self.nested_repositories.is_empty()
            || self.has_linked_worktrees()
            || match (self.workspace_root.as_deref(), self.repo_root.as_deref()) {
                (Some(workspace), Some(repo)) => Path::new(workspace) != Path::new(repo),
                _ => false,
            }
    }

    /// Whether something on screen is animating and so wants redrawing at the
    /// animation clock's rate rather than the idle one.
    ///
    /// Only the branch-action menu does: its preview draws a marker travelling
    /// the route the flow would take, and a picture redrawn slower than it
    /// moves reads as a stutter rather than a motion.
    pub fn wants_animation(&self) -> bool {
        self.modal == Modal::Flow && self.workflow_job.is_none()
    }

    /// Which set of keys the main pane is listening for right now.
    pub fn main_keys(&self) -> MainKeys {
        if self.session_view().is_some() {
            MainKeys::Session
        } else if matches!(self.diff_source, DiffSource::Review) && self.review.is_some() {
            MainKeys::Review
        } else {
            MainKeys::Diff
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_the_diff_text_keeps_the_line_count_in_step() {
        let mut state = AppState::new();
        state.set_diff_text("one\ntwo\nthree".to_string());
        assert_eq!(state.diff_line_count, 3);

        state.set_diff_text("just one".to_string());
        assert_eq!(
            state.diff_line_count, 1,
            "a shorter text must not keep the old bound"
        );
    }
}
