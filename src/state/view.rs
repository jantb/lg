//! Which panes are on screen, which one has focus, and what the main pane shows.

use super::{AppState, Modal, TreeRow};

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

/// Which hunk of the diff pane the hunk keys act on: the `index`th hunk of
/// those on `side`. Held per side rather than as one position, so staging a
/// hunk — which moves it from the worktree half of the pane to the staged
/// half — leaves the cursor on the next unstaged hunk instead of wherever the
/// shift put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HunkCursor {
    pub side: crate::git::hunk::HunkSide,
    pub index: usize,
}

/// What the diff pane's highlighted rows were built from: the row-count key,
/// plus the text's length and buffer, which also catch a text replaced without
/// [`AppState::set_diff_text`].
pub type DiffRenderKey = (DiffRowCountKey, usize, usize);

/// The diff pane's highlighted rows and what they were built from.
pub type DiffRenderCache =
    std::cell::RefCell<Option<(DiffRenderKey, Vec<ratatui::text::Line<'static>>)>>;

/// The file pane's rows and the files and folds they were built from.
#[derive(Debug)]
pub struct TreeRowsCache {
    files: Vec<crate::git::FileEntry>,
    collapsed: std::collections::HashSet<String>,
    filter: String,
    rows: std::sync::Arc<Vec<TreeRow>>,
}

/// The commit graph's lanes and the commits they were worked out for.
#[derive(Debug)]
pub struct PipeSetsCache {
    commits: Vec<crate::git::Commit>,
    pipes: std::sync::Arc<Vec<Vec<crate::graph::Pipe>>>,
}

/// Everything the rendered row count of the diff pane depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffRowCountKey {
    pub text_version: u64,
    pub viewport_width: u16,
    pub view_mode: DiffViewMode,
    pub log_view: bool,
}

/// Where the marker line a capped diff ends with starts.
pub const DIFF_TRUNCATED_MARKER: &str = "\u{2026} diff truncated";

/// `text` cut to at most `cap` lines, ending with a line that says how many
/// were left out.
///
/// The cut is made where a file or a hunk starts when there is one in the last
/// half of what is kept, so every hunk shown is whole and can be staged as it
/// is; the hunks past the cut are not shown and cannot be picked.
pub fn cap_diff_text(text: String, cap: usize) -> String {
    let total = text.lines().count();
    if total <= cap {
        return text;
    }
    let lines: Vec<&str> = text.lines().take(cap + 1).collect();
    let boundary = |line: &str| {
        line.starts_with("@@ ") || line.starts_with("diff --git ") || line.starts_with("== ")
    };
    let cut = (cap / 2..=cap.min(lines.len().saturating_sub(1)))
        .rev()
        .find(|&at| boundary(lines[at]))
        .unwrap_or(cap);
    let mut out = lines[..cut].join("\n");
    out.push('\n');
    out.push_str(&format!(
        "{DIFF_TRUNCATED_MARKER} \u{2014} {} more lines not shown (o opens the file in the IDE; git diff outside lg shows all of it)",
        total - cut
    ));
    out.push('\n');
    out
}

impl AppState {
    /// Replace the main pane's text and keep the line count in step. Scrolling
    /// is bounded by that count, so the two must not drift apart.
    ///
    /// The same text again changes nothing, so a refresh that reloads an
    /// unchanged diff keeps the highlighted rows already built for it.
    pub fn set_diff_text(&mut self, text: String) {
        let text = cap_diff_text(text, crate::config::DIFF_LINE_CAP);
        if text == self.diff_text {
            return;
        }
        self.diff_text = text;
        self.diff_text_version = self.diff_text_version.wrapping_add(1);
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

    /// The file pane's rows, built again only when the files, the folded
    /// folders or the `/` filter differ from what the last rows were built
    /// from. A filter leaves the files whose path has it in, and the folders
    /// above them.
    pub fn tree_rows(&self) -> std::sync::Arc<Vec<TreeRow>> {
        let filter = if self.files_filter.active() {
            self.files_filter.text.trim().to_lowercase()
        } else {
            String::new()
        };
        let mut cache = self.tree_rows_cache.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.files == self.files
            && cached.collapsed == self.collapsed_dirs
            && cached.filter == filter
        {
            return cached.rows.clone();
        }
        let rows = std::sync::Arc::new(super::tree::build_tree_rows_where(
            &self.files,
            &self.collapsed_dirs,
            |file| filter.is_empty() || file.path.to_lowercase().contains(&filter),
        ));
        *cache = Some(TreeRowsCache {
            files: self.files.clone(),
            collapsed: self.collapsed_dirs.clone(),
            filter,
            rows: rows.clone(),
        });
        rows
    }

    /// The commit graph's lanes, one set per commit, worked out again only
    /// when the commits differ from the ones they were last worked out for.
    pub fn commit_pipe_sets(&self) -> std::sync::Arc<Vec<Vec<crate::graph::Pipe>>> {
        let mut cache = self.pipe_sets_cache.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.commits == self.commits
        {
            return cached.pipes.clone();
        }
        let pipes = std::sync::Arc::new(crate::graph::pipe_sets(&self.commits));
        *cache = Some(PipeSetsCache {
            commits: self.commits.clone(),
            pipes: pipes.clone(),
        });
        pipes
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

    /// Show a session in the main pane, and hand it the keyboard. The rest of
    /// lg follows it to its checkout: a session is work being done in one
    /// worktree, and the diff, branches and commits beside it are only useful
    /// if they are that worktree's rather than whichever one was last looked
    /// at.
    pub fn show_session(&mut self, id: crate::session::SessionId) {
        self.sessions.focus(id);
        self.main_view = MainView::Session(id);
        self.focus = Pane::Main;
        self.follow_session_checkout(id);
    }

    /// Queue the switch to the checkout a session runs in, unless lg is
    /// already showing it. Nothing to switch to if lg has no repository at all
    /// yet: the switch is anchored on one.
    fn follow_session_checkout(&mut self, id: crate::session::SessionId) {
        let Some(cwd) = self.sessions.get(id).map(|session| session.cwd.clone()) else {
            return;
        };
        if self.workspace_root.is_none() && self.repo_root.is_none() {
            return;
        }
        if self
            .repo_root
            .as_ref()
            .is_some_and(|root| crate::session::same_dir(std::path::Path::new(root), &cwd))
        {
            return;
        }
        self.pending_action = Some(crate::state::PendingAction::SwitchRepository {
            target: crate::state::RepoTarget::Path(cwd),
        });
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

    /// Whether the workspace pane has anything to show.
    ///
    /// A checkout is enough. The pane lists every checkout and the sessions
    /// running in them, and starting a session is something it alone can do —
    /// so hiding it from a lone project, as this once did when there were no
    /// deploy branches and nothing nested, left that project with no way to
    /// open an agent or a terminal at all.
    pub fn environments_visible(&self) -> bool {
        self.repo_root.is_some()
            || self.workspace_root.is_some()
            || !self.nested_repositories.is_empty()
            || self.has_linked_worktrees()
    }

    /// Whether something on screen is animating and so wants redrawing at the
    /// animation clock's rate rather than the idle one.
    ///
    /// Any open modal does: light travels its frame, and the branch-action
    /// menu also draws a marker along the route the flow would take — a
    /// picture redrawn slower than it moves reads as a stutter rather than a
    /// motion. So does a status message that is still settling or flashing,
    /// and a working session, whose yellow dot pulses until it finishes or
    /// needs input. Jobs are not listed here: they already poll at their own
    /// faster rate.
    pub fn wants_animation(&self) -> bool {
        self.decorative_motion()
            || (self.decorative_animations && self.sessions.activity_counts().1 > 0)
    }

    /// The part of [`AppState::wants_animation`] that is smooth motion rather
    /// than a slow pulse: a modal's orbiting frame and a settling status line.
    pub fn decorative_motion(&self) -> bool {
        // Every modal carries the orbiting frame, so an open one keeps the
        // screen repainting. The review chat is docked into the main pane
        // rather than drawn as a box, so it has no frame to move.
        self.decorative_animations
            && (!matches!(self.modal, Modal::None | Modal::ReviewChat)
                || self.status.as_ref().is_some_and(|status| {
                    crate::ui::palette::status_animating(status.age_ms(), status.is_error)
                }))
    }

    /// Which set of keys the main pane is listening for right now.
    pub fn main_keys(&self) -> MainKeys {
        if self.session_view().is_some() {
            MainKeys::Session
        } else if matches!(self.diff_source, DiffSource::Review) && self.review.assisted.is_some() {
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

    #[test]
    fn reloading_the_same_diff_keeps_what_was_built_for_it() {
        let mut state = AppState::new();
        state.set_diff_text("diff --git a/x b/x\n+one".to_string());
        let version = state.diff_text_version;

        state.set_diff_text("diff --git a/x b/x\n+one".to_string());
        assert_eq!(
            state.diff_text_version, version,
            "an unchanged diff must not throw away its highlighted rows"
        );

        state.set_diff_text("diff --git a/x b/x\n+two".to_string());
        assert_ne!(state.diff_text_version, version, "a changed one must");
    }
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    #[test]
    fn a_diff_within_the_cap_is_left_alone() {
        let text = "a\nb\nc\n".to_string();
        assert_eq!(cap_diff_text(text.clone(), 3), text);
    }

    /// The cut falls where a hunk starts, so the hunk before it is whole.
    #[test]
    fn a_long_diff_is_cut_where_a_hunk_starts() {
        let text = "@@ -1,2 +1,2 @@\n-a\n+b\n c\n@@ -9,2 +9,2 @@\n-d\n+e\n f\n".to_string();

        let capped = cap_diff_text(text, 6);

        let kept: Vec<&str> = capped.lines().collect();
        assert_eq!(kept[..4], ["@@ -1,2 +1,2 @@", "-a", "+b", " c"]);
        assert!(kept[4].starts_with(DIFF_TRUNCATED_MARKER), "{capped}");
        assert!(kept[4].contains("4 more lines"), "{capped}");
        assert_eq!(kept.len(), 5);
    }
}
