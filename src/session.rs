//! Terminal sessions lg keeps alive — one agent of each kind per checkout,
//! and as many shells as are wanted.
//!
//! A session is a program running on a pseudo terminal in one worktree, plus
//! the parsed screen it has drawn so far. Two sorts are on offer: a coding
//! agent — claude, codex or pi — and the user's own shell. Sessions keep
//! running and keep being read while lg shows something else, so several
//! checkouts can be worked on at once and switched between, and one checkout
//! can hold every kind at the same time. Agents are one to a checkout, since
//! two claudes editing the same files is a mistake rather than a feature;
//! shells are not, because running a build in one and reading a log in the
//! next is ordinary work.

use anyhow::Result;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::term::{PtyMsg, PtyProcess, Spawn};

mod reading;
mod registry;
mod spawn;
use reading::*;
pub use registry::{EndedSession, PumpReport, Sessions};
pub(crate) use spawn::nested_claude_markers;
pub use spawn::{claude_spawn, codex_spawn, pi_spawn, shell_spawn};

/// How many lines of scrolled-off output each session keeps.
const SCROLLBACK: usize = 1000;

/// Size a session starts at before the pane it lives in has been laid out.
const DEFAULT_SIZE: (u16, u16) = (24, 80);

/// Shell a terminal session falls back to when `SHELL` says nothing useful.
const FALLBACK_SHELL: &str = "/bin/sh";

/// How much of a session's output one frame parses. A program that floods its
/// terminal — a build dumping a log, `cat` on a large file — would otherwise
/// hold the frame until every queued byte had gone through the parser; what is
/// left over waits in the channel for the next frame, which comes right away.
const PUMP_BUDGET_BYTES: usize = 256 * 1024;

/// How often a session that keeps writing has its screen re-read for a
/// question. Reading means copying out and lowercasing the whole screen, which
/// is too much to do for every chunk of a stream; a question stays up until it
/// is answered, so being a tenth of a second late to it loses nothing.
const READING_INTERVAL_MS: u128 = 100;

/// Screen lines an `LG_SESSION_TRACE` entry keeps — enough to hold the status
/// line and whatever claude drew under it.
const TRACE_TAIL_LINES: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u64);

#[cfg(test)]
impl SessionId {
    /// An id no session was started under, for code that only holds one.
    pub(crate) fn unstarted(id: u64) -> Self {
        Self(id)
    }
}

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

/// Which program a session runs.
///
/// Every kind is the same thing to lg — a program on a pseudo terminal in one
/// checkout — and they differ only in what is started and how much each says
/// about itself. claude reports what it is doing through hooks; the others
/// are read off the terminal: writing to the screen is working, and a choice
/// list waiting on an answer is a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionKind {
    Claude,
    Codex,
    Pi,
    /// The user's login shell, for the commands an agent is the wrong tool for.
    Terminal,
}

impl SessionKind {
    /// The coding agents, in the order the picker offers them.
    pub const AGENTS: [Self; 3] = [Self::Claude, Self::Codex, Self::Pi];

    /// What this kind is called in the UI: the pane title, the tree row, and
    /// the footer that says where the keyboard is pointing. It is also the
    /// program's name, which is the point — a row saying `codex` is saying
    /// which binary is running there.
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Terminal => "terminal",
        }
    }

    /// Whether this kind is a coding agent rather than a plain shell. An agent
    /// takes a starting prompt and is what lg hands a conflict to; a shell
    /// takes neither, because the person typing at it supplies both.
    pub fn is_agent(self) -> bool {
        !matches!(self, Self::Terminal)
    }

    /// The letter that picks this agent outright in the picker. Not the
    /// initial in every case: `p` is pull and `c` is commit before either
    /// reaches a pane, so codex answers to the `x` in its name.
    pub fn pick_key(self) -> char {
        match self {
            Self::Claude => 'c',
            Self::Codex => 'x',
            Self::Pi => 'p',
            Self::Terminal => 't',
        }
    }
}

/// What a running session is doing.
///
/// Busy or ready comes from claude itself, through the hooks lg starts it with
/// (see [`crate::hooks`]). Being asked a question is still read off the screen:
/// the questions worth a red dot include the ones claude puts up before it has
/// run a single hook. A program with no hooks — codex, pi, a shell — is busy
/// while it is still writing to its screen (see [`OUTPUT_ACTIVE_MS`]) and idle
/// once it has fallen quiet, which is how a person tells the same thing from
/// across the room. A shell also hands its terminal to whatever it runs, so a
/// command that is running but silent reads as [`SessionActivity::Running`]
/// rather than as a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionActivity {
    /// Sitting at its prompt with nothing to do — ready for a command.
    Idle,
    /// Busy, and interruptible. Nothing is being asked of us.
    Working,
    /// A shell with a command in the foreground that has fallen quiet: a
    /// build waiting on the network, or an editor open. Not ready, and not
    /// visibly doing anything either.
    Running,
    /// Blocked on a question only the user can answer.
    NeedsInput,
}

/// How long after its last output a program without hooks still counts as
/// working. Long enough to bridge the pauses in streamed text and a build's
/// quiet stretches, short enough that a prompt reads as ready before the eye
/// has moved on.
pub const OUTPUT_ACTIVE_MS: u128 = 1_500;

/// What to run, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    /// What this session is called in the UI — usually the branch.
    pub label: String,
    pub cwd: PathBuf,
    pub sandboxed: bool,
    pub kind: SessionKind,
    /// Something for the session to start on, for a session begun in answer to
    /// a problem lg already knows about. Only claude takes one; a shell is
    /// given its prompt by the person typing at it.
    pub prompt: Option<String>,
}

pub struct Session {
    pub id: SessionId,
    pub label: String,
    /// Which shell in this checkout this is, counting from one. Only shells
    /// can be several in one place, so for everything else it is always 1.
    /// The number is handed out at startup and kept for the session's life, so
    /// closing terminal 1 does not renumber terminal 2 under the user's hands.
    pub seq: usize,
    pub cwd: PathBuf,
    pub sandboxed: bool,
    pub kind: SessionKind,
    pub status: SessionStatus,
    launch: Option<(SessionSpec, Spawn)>,
    /// Output arrived while this session was not the one being shown.
    pub attention: bool,
    /// What claude last reported through its hooks. Starts idle: a session that
    /// has not said anything yet has nothing to show for it.
    activity: SessionActivity,
    /// Whether the last screen it drew was showing a question. Recomputed on
    /// output rather than on render, so the repo tree can show it for every
    /// session at once without re-reading five screens a frame.
    asking: bool,
    /// Hooks reporting in, for a session started with them.
    events: Option<crate::hooks::HookEvents>,
    /// When the program last wrote to its screen, for the kinds that have no
    /// hooks to say what they are doing.
    last_output: Option<std::time::Instant>,
    /// Whether the screen still has to be read for a question, and when it was
    /// last read.
    reading: Reading,
    parser: vt100::Parser,
    process: Option<PtyProcess>,
}

/// Bookkeeping for [`READING_INTERVAL_MS`].
#[derive(Debug, Default)]
struct Reading {
    /// The screen changed since it was last read.
    due: bool,
    last: Option<std::time::Instant>,
}

/// What reading a session's output did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pumped {
    /// Something about the session changed: its screen, its activity, or
    /// whether it is still running.
    pub changed: bool,
    /// Work was left for the next frame: output past the frame's budget, or a
    /// screen still waiting to be read for a question.
    pub more: bool,
}

impl Session {
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn is_running(&self) -> bool {
        self.status == SessionStatus::Running
    }

    /// What it is doing, for the dot the repo tree puts in front of it. A
    /// session that has ended is doing nothing, whatever it last reported.
    ///
    /// A question on screen outranks the hooks. claude can be mid-turn and still
    /// be blocked on an answer — that is what a permission prompt is — and being
    /// asked something is the state worth interrupting a person for.
    pub fn activity(&self) -> SessionActivity {
        match self.status {
            SessionStatus::Ended(_) => SessionActivity::Idle,
            SessionStatus::Running if self.asking => SessionActivity::NeedsInput,
            SessionStatus::Running if self.events.is_none() && self.still_writing() => {
                SessionActivity::Working
            }
            SessionStatus::Running if self.command_in_foreground() => SessionActivity::Running,
            SessionStatus::Running => self.activity,
        }
    }

    /// Whether a shell session has a command running, read off the pty. Only
    /// a shell hands its terminal to what it runs; an agent keeps the
    /// foreground itself, so the answer would say nothing about it.
    fn command_in_foreground(&self) -> bool {
        self.kind == SessionKind::Terminal
            && self
                .process
                .as_ref()
                .and_then(PtyProcess::running_command)
                .unwrap_or(false)
    }

    /// Whether the program wrote to its screen within the last
    /// [`OUTPUT_ACTIVE_MS`]. The working signal for anything without hooks.
    fn still_writing(&self) -> bool {
        self.last_output
            .is_some_and(|at| at.elapsed().as_millis() < OUTPUT_ACTIVE_MS)
    }

    /// What this session is called by its kind: the program's name, and the
    /// number that tells two shells in the same checkout apart.
    pub fn kind_label(&self) -> String {
        match self.seq {
            0 | 1 => self.kind.label().to_string(),
            seq => format!("{} {seq}", self.kind.label()),
        }
    }

    /// Line for the session pane's frame.
    pub fn title(&self) -> String {
        let mut title = format!("{} \u{b7} {}", self.kind_label(), self.label);
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

    /// Read what the program has written since the last call, up to
    /// [`PUMP_BUDGET_BYTES`].
    fn pump(&mut self, focused: bool) -> Pumped {
        let Some(process) = self.process.as_ref() else {
            return Pumped::default();
        };
        let mut changed = false;
        let mut ended = None;
        let mut parsed = 0usize;
        let mut more = false;
        loop {
            if parsed >= PUMP_BUDGET_BYTES {
                more = true;
                break;
            }
            match process.try_recv() {
                Ok(PtyMsg::Output(bytes)) => {
                    self.parser.process(&bytes);
                    parsed += bytes.len();
                    self.last_output = Some(std::time::Instant::now());
                    self.reading.due = true;
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
        if self.reread_question() {
            changed = true;
        }
        for event in self
            .events
            .as_mut()
            .map_or_else(Vec::new, |events| events.drain())
        {
            self.activity = event.activity;
            // The dot moved, even if nothing was drawn.
            changed = true;
            trace_event(&self.label, event, self.activity());
        }
        if let Some(notice) = ended {
            self.status = SessionStatus::Ended(notice);
            // Keep the final screen readable, but let go of the pty.
            self.process = None;
            self.attention = !focused;
            self.reading.due = false;
            changed = true;
        }
        let unread = self.reading.due && self.parser.screen().scrollback() == 0;
        Pumped {
            changed,
            more: more || unread,
        }
    }

    /// Read the screen for a question, if it changed since it was last read
    /// and the last reading is at least [`READING_INTERVAL_MS`] old. Returns
    /// whether the answer changed.
    ///
    /// Scrolled back, the visible screen is history, and a question in it has
    /// long since been answered. The last live reading stands until the view
    /// returns to the bottom.
    fn reread_question(&mut self) -> bool {
        if !self.reading.due || self.parser.screen().scrollback() != 0 {
            return false;
        }
        if self
            .reading
            .last
            .is_some_and(|at| at.elapsed().as_millis() < READING_INTERVAL_MS)
        {
            return false;
        }
        self.reading.due = false;
        self.reading.last = Some(std::time::Instant::now());
        let screen = self.parser.screen().contents();
        let asking = is_asking(&screen);
        if asking == self.asking {
            return false;
        }
        self.asking = asking;
        trace_reading(&self.label, self.activity(), &screen);
        true
    }
}

/// The size a session should run at, given the pane it is drawn in.
pub fn size_for_pane(area: ratatui::layout::Rect) -> (u16, u16) {
    let rows = area.height.saturating_sub(2).max(1);
    let cols = area.width.saturating_sub(2).max(1);
    (rows, cols)
}

/// Whether two paths name the same directory, by spelling or once resolved.
pub(crate) fn same_dir(a: &Path, b: &Path) -> bool {
    crate::git::same_dir(a, b)
}

impl Default for SessionSpec {
    fn default() -> Self {
        Self {
            label: String::new(),
            cwd: PathBuf::from("."),
            sandboxed: false,
            kind: SessionKind::Claude,
            prompt: None,
        }
    }
}

/// The size sessions start at before their pane has been measured.
pub fn default_size() -> (u16, u16) {
    DEFAULT_SIZE
}

#[cfg(test)]
mod tests;
