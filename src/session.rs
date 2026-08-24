//! Terminal sessions lg keeps alive — one per checkout.
//!
//! A session is a program (claude, by default) running on a pseudo terminal in
//! one worktree, plus the parsed screen it has drawn so far. Sessions keep
//! running and keep being read while lg shows something else, so several
//! checkouts can be worked on at once and switched between.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::term::{PtyMsg, PtyProcess, Spawn};

/// How many lines of scrolled-off output each session keeps.
const SCROLLBACK: usize = 1000;

/// Size a session starts at before the pane it lives in has been laid out.
const DEFAULT_SIZE: (u16, u16) = (24, 80);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u64);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    /// The program ended; the string is what to show the user.
    Ended(String),
}

/// What to run, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    /// What this session is called in the UI — usually the branch.
    pub label: String,
    pub cwd: PathBuf,
    pub sandboxed: bool,
}

pub struct Session {
    pub id: SessionId,
    pub label: String,
    pub cwd: PathBuf,
    pub sandboxed: bool,
    pub status: SessionStatus,
    /// Output arrived while this session was not the one being shown.
    pub attention: bool,
    parser: vt100::Parser,
    process: Option<PtyProcess>,
}

impl Session {
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn is_running(&self) -> bool {
        self.status == SessionStatus::Running
    }

    /// Line for the session pane's frame.
    pub fn title(&self) -> String {
        let mut title = format!("claude \u{b7} {}", self.label);
        if self.sandboxed {
            title.push_str(" \u{b7} sandboxed");
        }
        if let SessionStatus::Ended(notice) = &self.status {
            title.push_str(" \u{b7} ");
            title.push_str(notice);
        }
        title
    }

    /// Send bytes to the program. Ignored once it has ended.
    pub fn send(&mut self, bytes: &[u8]) {
        // Typing is about the live screen, so it returns the view there rather
        // than leaving the reply to happen off-screen.
        self.parser.screen_mut().set_scrollback(0);
        if let Some(process) = self.process.as_mut() {
            process.write(bytes);
        }
    }

    /// Move the view `lines` back through scrollback, or the same distance
    /// toward the live screen. Returns whether the view moved, so a caller can
    /// let the event fall through when there is nothing left to scroll.
    pub fn scroll(&mut self, back: bool, lines: usize) -> bool {
        let current = self.parser.screen().scrollback();
        let target = if back {
            current.saturating_add(lines).min(SCROLLBACK)
        } else {
            current.saturating_sub(lines)
        };
        if target == current {
            return false;
        }
        self.parser.screen_mut().set_scrollback(target);
        true
    }

    /// Hand one wheel notch to the program at a cell of its own screen, for
    /// programs that asked to be told about the mouse. Returns whether it was
    /// sent; when it was not, the wheel is lg's to act on.
    ///
    /// Every full-screen program is in this position: the alternate screen has
    /// no scrollback for lg to move on its behalf, so scrolling one any other
    /// way is not scrolling at all.
    pub fn send_wheel(&mut self, up: bool, column: u16, row: u16) -> bool {
        let screen = self.parser.screen();
        let Some(bytes) = crate::term::encode_wheel(
            up,
            column,
            row,
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
        ) else {
            return false;
        };
        self.send(&bytes);
        true
    }

    /// Match the program's window to the pane it is drawn in. Resizing also
    /// makes it repaint, which is how a backgrounded session comes back clean.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if self.parser.screen().size() == (rows, cols) {
            return;
        }
        self.parser.screen_mut().set_size(rows, cols);
        if let Some(process) = self.process.as_mut() {
            process.resize((rows, cols));
        }
    }

    /// Whether the program switched the arrow keys to application mode, which
    /// changes the bytes they send.
    pub fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    /// Whether the program asked for pastes to be marked as such.
    pub fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    /// Where the program's cursor is, unless it hid it.
    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        if self.parser.screen().hide_cursor() {
            return None;
        }
        Some(self.parser.screen().cursor_position())
    }

    /// Read whatever the program has written since the last call. Returns
    /// whether anything changed, so the caller can skip a redraw.
    fn pump(&mut self, focused: bool) -> bool {
        let Some(process) = self.process.as_ref() else {
            return false;
        };
        let mut changed = false;
        let mut ended = None;
        loop {
            match process.try_recv() {
                Ok(PtyMsg::Output(bytes)) => {
                    self.parser.process(&bytes);
                    changed = true;
                    if !focused {
                        self.attention = true;
                    }
                }
                Ok(PtyMsg::Exited(notice)) => {
                    ended = Some(notice);
                    break;
                }
                // Disconnected without an exit notice means the pump thread is
                // gone; treat it the same as an exit so the session settles.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    ended = Some("stopped".to_string());
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        if let Some(notice) = ended {
            self.status = SessionStatus::Ended(notice);
            // Keep the final screen readable, but let go of the pty.
            self.process = None;
            self.attention = !focused;
            changed = true;
        }
        changed
    }
}

/// Every live session, and which one is being shown.
pub struct Sessions {
    items: Vec<Session>,
    focused: Option<SessionId>,
    next_id: u64,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            focused: None,
            next_id: 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Session> {
        self.items.iter()
    }

    pub fn get(&self, id: SessionId) -> Option<&Session> {
        self.items.iter().find(|session| session.id == id)
    }

    pub fn get_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.items.iter_mut().find(|session| session.id == id)
    }

    pub fn focused(&self) -> Option<SessionId> {
        self.focused
    }

    pub fn focused_session(&self) -> Option<&Session> {
        self.focused.and_then(|id| self.get(id))
    }

    /// Show this session, and clear the "look at me" mark it may carry.
    pub fn focus(&mut self, id: SessionId) {
        if let Some(session) = self.get_mut(id) {
            session.attention = false;
            self.focused = Some(id);
        }
    }

    /// The session running in `dir`, if there is one. One session per checkout
    /// is the whole point: switching to a worktree finds its session again.
    pub fn for_dir(&self, dir: &Path) -> Option<SessionId> {
        self.items
            .iter()
            .find(|session| same_dir(&session.cwd, dir))
            .map(|session| session.id)
    }

    /// Number of sessions waiting to be looked at.
    pub fn attention_count(&self) -> usize {
        self.items
            .iter()
            .filter(|session| session.attention)
            .count()
    }

    /// Start a claude session for `spec`, or hand back the one already running
    /// there.
    pub fn start(&mut self, spec: SessionSpec, size: (u16, u16)) -> Result<SessionId> {
        let spawn = claude_spawn(&spec.cwd, spec.sandboxed);
        self.start_with(spec, &spawn, size)
    }

    /// Start a session running something other than claude. This is the seam a
    /// different session backend plugs into, and what tests drive.
    pub fn start_with(
        &mut self,
        spec: SessionSpec,
        spawn: &Spawn,
        size: (u16, u16),
    ) -> Result<SessionId> {
        if let Some(existing) = self.for_dir(&spec.cwd) {
            self.focus(existing);
            return Ok(existing);
        }

        let size = (size.0.max(1), size.1.max(1));
        let process = PtyProcess::start(spawn, size)?;
        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.items.push(Session {
            id,
            label: spec.label,
            cwd: spec.cwd,
            sandboxed: spec.sandboxed,
            status: SessionStatus::Running,
            attention: false,
            parser: vt100::Parser::new(size.0, size.1, SCROLLBACK),
            process: None,
        });
        // Attaching after the push keeps the struct literal readable.
        if let Some(session) = self.items.last_mut() {
            session.process = Some(process);
        }
        self.focused = Some(id);
        Ok(id)
    }

    /// Stop a session and forget it. The next session in the list takes focus,
    /// so closing one does not leave the pane pointing at nothing.
    pub fn close(&mut self, id: SessionId) {
        let Some(idx) = self.items.iter().position(|session| session.id == id) else {
            return;
        };
        self.items.remove(idx);
        if self.focused == Some(id) {
            self.focused = self
                .items
                .get(idx)
                .or_else(|| self.items.last())
                .map(|session| session.id);
        }
    }

    /// Read every session's output. Returns whether any of them changed.
    pub fn pump(&mut self) -> bool {
        let focused = self.focused;
        let mut changed = false;
        for session in &mut self.items {
            changed |= session.pump(focused == Some(session.id));
        }
        changed
    }

    /// Sessions that ended and can be cleaned up in one go.
    pub fn ended_ids(&self) -> Vec<SessionId> {
        self.items
            .iter()
            .filter(|session| !session.is_running())
            .map(|session| session.id)
            .collect()
    }

    /// The session after (or before) the one being shown, wrapping round. Used
    /// for cycling between sessions without going through the tree.
    pub fn neighbour(&self, forward: bool) -> Option<SessionId> {
        if self.items.is_empty() {
            return None;
        }
        let current = self
            .focused
            .and_then(|id| self.items.iter().position(|session| session.id == id))
            .unwrap_or(0);
        let len = self.items.len();
        let next = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        Some(self.items[next].id)
    }

    /// Kill everything, on the way out.
    pub fn close_all(&mut self) {
        self.items.clear();
        self.focused = None;
    }
}

/// The permission mode an unsandboxed session runs under.
const AUTO_PERMISSION_MODE: &str = "auto";

/// How to launch claude in `cwd`. Sandboxed sessions go through terrarium,
/// which confines the process to that worktree. An unsandboxed one runs in
/// auto mode: nothing is holding it back anyway, so stopping to ask buys
/// little.
pub fn claude_spawn(cwd: &Path, sandboxed: bool) -> Spawn {
    // terrarium resolves the project path before looking up its profile, so the
    // path handed to it has to be resolved too.
    let cwd = &crate::terrarium::resolve(cwd);
    let (program, args) = if sandboxed {
        (
            "terrarium".to_string(),
            vec![
                "run".to_string(),
                "--project".to_string(),
                cwd.to_string_lossy().into_owned(),
                "--".to_string(),
                "claude".to_string(),
            ],
        )
    } else {
        (
            "claude".to_string(),
            vec![
                "--permission-mode".to_string(),
                AUTO_PERMISSION_MODE.to_string(),
            ],
        )
    };
    Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: vec![
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
        ],
        // lg may itself have been started from inside a claude session; without
        // dropping these the child would think it is nested in one.
        env_remove: vec![
            "CLAUDECODE".to_string(),
            "CLAUDE_CODE_ENTRYPOINT".to_string(),
        ],
    }
}

/// The size a session should run at, given the pane it is drawn in.
pub fn size_for_pane(area: ratatui::layout::Rect) -> (u16, u16) {
    let rows = area.height.saturating_sub(2).max(1);
    let cols = area.width.saturating_sub(2).max(1);
    (rows, cols)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

impl Default for SessionSpec {
    fn default() -> Self {
        Self {
            label: String::new(),
            cwd: PathBuf::from("."),
            sandboxed: false,
        }
    }
}

/// The size sessions start at before their pane has been measured.
pub fn default_size() -> (u16, u16) {
    DEFAULT_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandboxed_session_goes_through_terrarium_in_its_own_worktree() {
        let spawn = claude_spawn(Path::new("/dev/lg.worktrees/feat-x"), true);
        assert_eq!(spawn.program, "terrarium");
        assert_eq!(
            spawn.args,
            [
                "run",
                "--project",
                "/dev/lg.worktrees/feat-x",
                "--",
                "claude"
            ]
        );
        assert_eq!(spawn.cwd, Path::new("/dev/lg.worktrees/feat-x"));
    }

    #[test]
    fn an_unsandboxed_session_runs_claude_in_auto_mode() {
        let spawn = claude_spawn(Path::new("/dev/lg"), false);
        assert_eq!(spawn.program, "claude");
        assert_eq!(spawn.args, ["--permission-mode", "auto"]);
        assert_eq!(spawn.cwd, Path::new("/dev/lg"));
    }

    #[test]
    fn sessions_declare_a_terminal_and_drop_the_nested_claude_markers() {
        let spawn = claude_spawn(Path::new("/dev/lg"), false);
        assert!(
            spawn
                .env
                .contains(&("TERM".to_string(), "xterm-256color".to_string()))
        );
        assert!(spawn.env_remove.contains(&"CLAUDECODE".to_string()));
    }

    #[test]
    fn a_pane_maps_to_the_screen_size_inside_its_border() {
        assert_eq!(
            size_for_pane(ratatui::layout::Rect::new(0, 0, 82, 26)),
            (24, 80)
        );
        assert_eq!(
            size_for_pane(ratatui::layout::Rect::new(0, 0, 1, 1)),
            (1, 1),
            "a pane too small for its border still gets a usable size"
        );
    }

    /// Sessions that do not need a real process: enough to exercise the
    /// registry's bookkeeping.
    fn fake(id: u64, cwd: &str) -> Session {
        Session {
            id: SessionId(id),
            label: format!("session {id}"),
            cwd: PathBuf::from(cwd),
            sandboxed: false,
            status: SessionStatus::Running,
            attention: false,
            parser: vt100::Parser::new(24, 80, 0),
            process: None,
        }
    }

    fn registry(dirs: &[&str]) -> Sessions {
        let mut sessions = Sessions::new();
        for (idx, dir) in dirs.iter().enumerate() {
            let id = idx as u64 + 1;
            sessions.items.push(fake(id, dir));
            sessions.next_id = id + 1;
        }
        sessions.focused = sessions.items.first().map(|session| session.id);
        sessions
    }

    #[test]
    fn a_checkout_has_at_most_one_session() {
        let sessions = registry(&["/a", "/b"]);
        assert_eq!(sessions.for_dir(Path::new("/b")), Some(SessionId(2)));
        assert_eq!(sessions.for_dir(Path::new("/c")), None);
    }

    #[test]
    fn focusing_a_session_clears_its_attention_mark() {
        let mut sessions = registry(&["/a", "/b"]);
        sessions.get_mut(SessionId(2)).unwrap().attention = true;
        assert_eq!(sessions.attention_count(), 1);
        sessions.focus(SessionId(2));
        assert_eq!(sessions.focused(), Some(SessionId(2)));
        assert_eq!(sessions.attention_count(), 0);
    }

    #[test]
    fn closing_the_shown_session_moves_focus_to_a_neighbour() {
        let mut sessions = registry(&["/a", "/b", "/c"]);
        sessions.focus(SessionId(2));
        sessions.close(SessionId(2));
        assert_eq!(sessions.focused(), Some(SessionId(3)));

        sessions.close(SessionId(3));
        assert_eq!(sessions.focused(), Some(SessionId(1)));

        sessions.close(SessionId(1));
        assert_eq!(sessions.focused(), None);
        assert!(sessions.is_empty());
    }

    #[test]
    fn closing_a_background_session_leaves_the_shown_one_alone() {
        let mut sessions = registry(&["/a", "/b"]);
        sessions.focus(SessionId(1));
        sessions.close(SessionId(2));
        assert_eq!(sessions.focused(), Some(SessionId(1)));
    }

    #[test]
    fn a_session_title_says_where_it_runs_and_how_it_ended() {
        let mut session = fake(1, "/a");
        session.label = "feat/x".to_string();
        session.sandboxed = true;
        assert_eq!(session.title(), "claude \u{b7} feat/x \u{b7} sandboxed");
        session.status = SessionStatus::Ended("exited".to_string());
        assert_eq!(
            session.title(),
            "claude \u{b7} feat/x \u{b7} sandboxed \u{b7} exited"
        );
    }

    #[test]
    fn resizing_moves_the_parsed_screen_too() {
        let mut session = fake(1, "/a");
        session.resize(10, 40);
        assert_eq!(session.screen().size(), (10, 40));
    }
}
