//! Terminal sessions lg keeps alive — one of each kind per checkout.
//!
//! A session is a program running on a pseudo terminal in one worktree, plus
//! the parsed screen it has drawn so far. Two kinds are on offer: claude, and
//! the user's own shell. Sessions keep running and keep being read while lg
//! shows something else, so several checkouts can be worked on at once and
//! switched between.

use anyhow::Result;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::term::{PtyMsg, PtyProcess, Spawn};

/// How many lines of scrolled-off output each session keeps.
const SCROLLBACK: usize = 1000;

/// Size a session starts at before the pane it lives in has been laid out.
const DEFAULT_SIZE: (u16, u16) = (24, 80);

/// Shell a terminal session falls back to when `SHELL` says nothing useful.
const FALLBACK_SHELL: &str = "/bin/sh";

/// Screen lines an `LG_SESSION_TRACE` entry keeps — enough to hold the status
/// line and whatever claude drew under it.
const TRACE_TAIL_LINES: usize = 6;

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

/// Which program a session runs.
///
/// Both kinds are the same thing to lg — a program on a pseudo terminal in one
/// checkout — and differ only in what is started and how much it says about
/// itself. claude reports what it is doing through hooks; a shell reports
/// nothing, so its dot stays green unless something it runs puts a question on
/// screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionKind {
    Claude,
    /// The user's login shell, for the commands claude is the wrong tool for.
    Terminal,
}

impl SessionKind {
    /// What this kind is called in the UI: the pane title, the tree row, and
    /// the footer that says where the keyboard is pointing.
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Terminal => "terminal",
        }
    }
}

/// What a running session is doing.
///
/// Busy or ready comes from claude itself, through the hooks lg starts it with
/// (see [`crate::hooks`]). Being asked a question is still read off the screen:
/// the questions worth a red dot include the ones claude puts up before it has
/// run a single hook. A program that neither reports nor asks reads as
/// [`SessionActivity::Idle`], which is the honest answer for anything that is
/// not claude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionActivity {
    /// Sitting at its prompt with nothing to do — ready for a command.
    Idle,
    /// Busy, and interruptible. Nothing is being asked of us.
    Working,
    /// Blocked on a question only the user can answer.
    NeedsInput,
}

/// Openings of the questions claude blocks on: tool permissions, file edits,
/// and the trust prompt a new directory gets.
const QUESTION_MARKERS: &[&str] = &["do you want", "would you like", "do you trust"];

/// What a chosen option is ticked with once the question has been answered.
const ANSWERED_MARKS: &[char] = &['\u{2714}', '\u{2713}'];

/// Whether the screen is showing a question claude is waiting on an answer to.
///
/// A choice list with one row selected is claude's own shape for asking, and
/// nothing it writes in an answer looks like that. The wording on its own is not
/// enough — an answer can easily contain "do you want" — so a question phrase
/// only counts when there are numbered options under it to pick from.
fn is_asking(text: &str) -> bool {
    if text
        .lines()
        .any(|line| pending_choice(line).is_some_and(|selected| selected))
    {
        return true;
    }
    let lowered = text.to_lowercase();
    QUESTION_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
        && text.lines().any(|line| pending_choice(line).is_some())
}

/// Whether `line` is a row of a choice list still waiting to be answered, and
/// whether it is the selected one.
///
/// The caret alone does not make a choice list — it is also claude's ordinary
/// input prompt — so the number after it is what counts. An answered question
/// stays on screen with its chosen row ticked, which is how a prompt that has
/// already been dealt with is told from one that has not.
fn pending_choice(line: &str) -> Option<bool> {
    let line = line.trim();
    if line.contains(ANSWERED_MARKS) {
        return None;
    }
    let (selected, rest) = match line.strip_prefix('\u{276f}') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, line),
    };
    let digits = leading_digits(rest);
    (digits > 0 && rest[digits..].starts_with('.')).then_some(selected)
}

/// Bytes of ASCII digits at the start of `text`.
fn leading_digits(text: &str) -> usize {
    text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len()
}

/// Records a hook reporting in, and what the session reads as now.
fn trace_event(label: &str, event: crate::hooks::HookEvent, activity: SessionActivity) {
    trace(&format!("{label} hook {} -> {activity:?}", event.name));
}

/// Records a question appearing or going away, with the screen lines it was read
/// off, so a shape this misses can be recovered afterwards instead of having to
/// be caught live.
fn trace_reading(label: &str, activity: SessionActivity, screen: &str) {
    let mut tail: Vec<&str> = screen
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(TRACE_TAIL_LINES)
        .collect();
    tail.reverse();
    trace(&format!(
        "{label} screen -> {activity:?} | {}",
        tail.join(" \u{23ce} ")
    ));
}

/// Appends a line to the file named by `LG_SESSION_TRACE`, when there is one.
fn trace(line: &str) {
    let Some(path) = std::env::var_os("LG_SESSION_TRACE") else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

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
    pub cwd: PathBuf,
    pub sandboxed: bool,
    pub kind: SessionKind,
    pub status: SessionStatus,
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
            SessionStatus::Running => self.activity,
        }
    }

    /// Line for the session pane's frame.
    pub fn title(&self) -> String {
        let mut title = format!("{} \u{b7} {}", self.kind.label(), self.label);
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
    /// whether anything changed.
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
        // Scrolled back, the visible screen is history, and a question in it
        // has long since been answered. The last live reading stands until the
        // view returns to the bottom.
        if changed && self.parser.screen().scrollback() == 0 {
            let screen = self.parser.screen().contents();
            let asking = is_asking(&screen);
            if asking != self.asking {
                self.asking = asking;
                trace_reading(&self.label, self.activity(), &screen);
            }
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
            changed = true;
        }
        changed
    }
}

/// A session that has stopped and been dropped from the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedSession {
    pub id: SessionId,
    /// The checkout it was running in.
    pub label: String,
    /// How it ended, in the words the session settled on: "exited", "killed by
    /// 9", "stopped".
    pub notice: String,
}

impl EndedSession {
    /// What is worth keeping about `session` once it is gone.
    fn of(session: &Session) -> Self {
        Self {
            id: session.id,
            label: session.label.clone(),
            notice: match &session.status {
                SessionStatus::Ended(notice) => notice.clone(),
                SessionStatus::Running => "stopped".to_string(),
            },
        }
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

    /// Every session running in `dir`, in the order they were started. A
    /// checkout can have one of each kind, so this is what the tree lists under
    /// it.
    pub fn for_dir(&self, dir: &Path) -> impl Iterator<Item = &Session> {
        self.items
            .iter()
            .filter(move |session| same_dir(&session.cwd, dir))
    }

    /// The `kind` session running in `dir`, if there is one. One session of
    /// each kind per checkout is the whole point: asking for a terminal in a
    /// worktree twice finds the first one again.
    pub fn for_dir_kind(&self, dir: &Path, kind: SessionKind) -> Option<SessionId> {
        self.for_dir(dir)
            .find(|session| session.kind == kind)
            .map(|session| session.id)
    }

    /// Whether anything at all is running in `dir`. Removing a checkout out
    /// from under any of it would pull the rug on a live process.
    pub fn any_in_dir(&self, dir: &Path) -> bool {
        self.for_dir(dir).next().is_some()
    }

    /// Number of sessions that have drawn something since they were last
    /// looked at. This is what the caret on a tree row means: unread output,
    /// which for a session mid-turn is nearly always.
    pub fn attention_count(&self) -> usize {
        self.items
            .iter()
            .filter(|session| session.attention)
            .count()
    }

    /// How many sessions are in each state worth saying out loud: blocked on a
    /// question, and busy. Idle sessions are the rest, and need no number.
    ///
    /// Unread output is deliberately not folded in here. A session that has
    /// printed a line is not waiting for anybody, and counting it as though it
    /// were makes the badge cry wolf for as long as anything is running.
    pub fn activity_counts(&self) -> (usize, usize) {
        self.items
            .iter()
            .fold((0, 0), |(needs_input, working), session| {
                match session.activity() {
                    SessionActivity::NeedsInput => (needs_input + 1, working),
                    SessionActivity::Working => (needs_input, working + 1),
                    SessionActivity::Idle => (needs_input, working),
                }
            })
    }

    /// Start the session `spec` asks for, or hand back the one of that kind
    /// already running there.
    pub fn start(&mut self, spec: SessionSpec, size: (u16, u16)) -> Result<SessionId> {
        if let Some(existing) = self.for_dir_kind(&spec.cwd, spec.kind) {
            self.focus(existing);
            return Ok(existing);
        }
        match spec.kind {
            SessionKind::Claude => self.start_claude(spec, size),
            SessionKind::Terminal => {
                let spawn = shell_spawn(&spec.cwd, spec.sandboxed);
                self.start_with(spec, &spawn, size)
            }
        }
    }

    /// Start claude, wired up to report what it is doing. A checkout with
    /// nowhere to keep a hook file still gets a session; it just has to do
    /// without claude saying what it is up to.
    fn start_claude(&mut self, spec: SessionSpec, size: (u16, u16)) -> Result<SessionId> {
        let hooks = crate::hooks::install(&spec.cwd).ok();
        let settings = hooks.as_ref().map(|channel| channel.settings.as_path());
        let spawn = claude_spawn(&spec.cwd, spec.sandboxed, settings, spec.prompt.as_deref());
        let id = self.start_with(spec, &spawn, size)?;
        if let Some(session) = self.get_mut(id) {
            session.events = hooks.map(|channel| channel.events);
        }
        Ok(id)
    }

    /// Start a session running something other than claude. This is the seam a
    /// different session backend plugs into, and what tests drive.
    pub fn start_with(
        &mut self,
        spec: SessionSpec,
        spawn: &Spawn,
        size: (u16, u16),
    ) -> Result<SessionId> {
        if let Some(existing) = self.for_dir_kind(&spec.cwd, spec.kind) {
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
            kind: spec.kind,
            status: SessionStatus::Running,
            attention: false,
            activity: SessionActivity::Idle,
            asking: false,
            events: None,
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

    /// Read every session's output, and let go of the ones whose program has
    /// ended. A stopped session has nothing left to draw and nothing to type
    /// into, so it goes as soon as it stops rather than staying on as a row
    /// waiting to be dismissed by hand.
    ///
    /// Returns what was dropped, so the caller can say what happened to it.
    pub fn pump(&mut self) -> Vec<EndedSession> {
        let focused = self.focused;
        for session in &mut self.items {
            session.pump(focused == Some(session.id));
        }
        let ended: Vec<EndedSession> = self
            .items
            .iter()
            .filter(|session| !session.is_running())
            .map(EndedSession::of)
            .collect();
        for session in &ended {
            self.close(session.id);
        }
        ended
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
///
/// `settings` is the hook settings file from [`crate::hooks::install`], which is
/// how the session comes to report what it is doing. Without one it still runs —
/// it just says nothing.
///
/// `prompt` is what the session opens on, for one started in answer to
/// something lg already knows about — a conflict it stopped on, say. It is
/// claude's first turn, not a flag, so it goes last of all.
pub fn claude_spawn(
    cwd: &Path,
    sandboxed: bool,
    settings: Option<&Path>,
    prompt: Option<&str>,
) -> Spawn {
    // terrarium resolves the project path before looking up its profile, so the
    // path handed to it has to be resolved too.
    let cwd = &crate::terrarium::resolve(cwd);
    let (program, mut args) = if sandboxed {
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
    // Last, so it lands after the `--` a sandboxed session goes through: these
    // are claude's arguments, not terrarium's.
    if let Some(settings) = settings {
        args.push("--settings".to_string());
        args.push(settings.to_string_lossy().into_owned());
    }
    if let Some(prompt) = prompt.map(str::trim).filter(|prompt| !prompt.is_empty()) {
        args.push(prompt.to_string());
    }
    Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: session_env(),
        env_remove: nested_claude_markers(),
    }
}

/// Shell to run a terminal session in: the one the user has chosen, falling
/// back to something every unix has. It is started with no arguments, which on
/// a pty is an interactive shell and so reads the usual rc file.
fn login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| FALLBACK_SHELL.to_string())
}

/// How to launch a shell in `cwd`. Sandboxed goes through terrarium exactly as
/// a claude session does, so a terminal opened on a worktree is confined to the
/// same checkout the claude session next to it is.
pub fn shell_spawn(cwd: &Path, sandboxed: bool) -> Spawn {
    // terrarium resolves the project path before looking up its profile, so the
    // path handed to it has to be resolved too.
    let cwd = &crate::terrarium::resolve(cwd);
    let shell = login_shell();
    let (program, args) = if sandboxed {
        (
            "terrarium".to_string(),
            vec![
                "run".to_string(),
                "--project".to_string(),
                cwd.to_string_lossy().into_owned(),
                "--".to_string(),
                shell,
            ],
        )
    } else {
        (shell, Vec::new())
    };
    Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: session_env(),
        // A shell is where a claude gets started by hand, so it needs the
        // markers dropped for the same reason a claude session does.
        env_remove: nested_claude_markers(),
    }
}

/// What every session is told about the terminal it is drawn on.
fn session_env() -> Vec<(String, String)> {
    vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ]
}

/// Variables that tell claude it is running inside another claude. lg may
/// itself have been started from inside a session; without dropping these the
/// child would think it is nested in one.
fn nested_claude_markers() -> Vec<String> {
    vec![
        "CLAUDECODE".to_string(),
        "CLAUDE_CODE_CHILD_SESSION".to_string(),
        "CLAUDE_CODE_ENTRYPOINT".to_string(),
    ]
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
mod tests {
    use super::*;

    #[test]
    fn a_sandboxed_session_goes_through_terrarium_in_its_own_worktree() {
        let spawn = claude_spawn(Path::new("/dev/lg.worktrees/feat-x"), true, None, None);
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
        let spawn = claude_spawn(Path::new("/dev/lg"), false, None, None);
        assert_eq!(spawn.program, "claude");
        assert_eq!(spawn.args, ["--permission-mode", "auto"]);
        assert_eq!(spawn.cwd, Path::new("/dev/lg"));
    }

    /// The hook settings are claude's argument. A sandboxed session is started by
    /// terrarium, so they have to land after the `--` that separates the two
    /// command lines, or terrarium would try to take them itself.
    #[test]
    fn hook_settings_are_passed_to_claude_and_not_to_terrarium() {
        let settings = Path::new("/dev/lg/.git/lg/sessions/dev-lg/settings.json");

        let sandboxed = claude_spawn(Path::new("/dev/lg"), true, Some(settings), None);
        assert_eq!(
            sandboxed.args,
            [
                "run",
                "--project",
                "/dev/lg",
                "--",
                "claude",
                "--settings",
                "/dev/lg/.git/lg/sessions/dev-lg/settings.json"
            ]
        );

        let plain = claude_spawn(Path::new("/dev/lg"), false, Some(settings), None);
        assert_eq!(
            plain.args,
            [
                "--permission-mode",
                "auto",
                "--settings",
                "/dev/lg/.git/lg/sessions/dev-lg/settings.json"
            ]
        );
    }

    /// The sandbox is what a worktree session is for, and it has to hold for a
    /// shell as much as for claude — a terminal outside it could write into the
    /// checkout next door.
    #[test]
    fn a_sandboxed_terminal_runs_its_shell_through_terrarium() {
        temp_env("SHELL", "/bin/zsh", || {
            let spawn = shell_spawn(Path::new("/dev/lg.worktrees/feat-x"), true);
            assert_eq!(spawn.program, "terrarium");
            assert_eq!(
                spawn.args,
                [
                    "run",
                    "--project",
                    "/dev/lg.worktrees/feat-x",
                    "--",
                    "/bin/zsh"
                ]
            );
            assert_eq!(spawn.cwd, Path::new("/dev/lg.worktrees/feat-x"));
        });
    }

    #[test]
    fn an_unsandboxed_terminal_runs_the_users_shell_with_no_arguments() {
        temp_env("SHELL", "/bin/zsh", || {
            let spawn = shell_spawn(Path::new("/dev/lg"), false);
            assert_eq!(spawn.program, "/bin/zsh");
            assert!(spawn.args.is_empty(), "an interactive shell needs no flags");
            assert_eq!(spawn.cwd, Path::new("/dev/lg"));
        });
    }

    #[test]
    fn a_terminal_falls_back_to_a_shell_every_machine_has() {
        temp_env("SHELL", "", || {
            assert_eq!(shell_spawn(Path::new("/dev/lg"), false).program, "/bin/sh");
        });
    }

    /// A shell is where claude gets started by hand, so it must not inherit the
    /// markers that would make that claude think it is nested.
    #[test]
    fn a_terminal_declares_a_terminal_and_drops_the_nested_claude_markers() {
        let spawn = shell_spawn(Path::new("/dev/lg"), false);
        assert!(
            spawn
                .env
                .contains(&("TERM".to_string(), "xterm-256color".to_string()))
        );
        assert!(spawn.env_remove.contains(&"CLAUDECODE".to_string()));
    }

    /// Runs `body` with `SHELL` set to `value`, restoring it afterwards. The
    /// shell tests share the process environment, so they also share this lock.
    fn temp_env(name: &str, value: &str, body: impl FnOnce()) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let previous = std::env::var_os(name);
        // SAFETY: the lock keeps these tests from racing each other, and the
        // rest of the suite does not read SHELL.
        unsafe { std::env::set_var(name, value) };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        match previous {
            Some(previous) => unsafe { std::env::set_var(name, previous) },
            None => unsafe { std::env::remove_var(name) },
        }
        drop(guard);
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    /// A prompt is claude's first turn, so it has to land after every flag —
    /// and after the `--` a sandboxed session goes through, or terrarium would
    /// take it for its own.
    #[test]
    fn an_opening_prompt_is_the_last_argument_claude_gets() {
        let settings = Path::new("/dev/lg/.git/lg/sessions/dev-lg/settings.json");
        let spawn = claude_spawn(
            Path::new("/dev/lg"),
            true,
            Some(settings),
            Some("resolve the conflict"),
        );
        assert_eq!(
            spawn.args.last().map(String::as_str),
            Some("resolve the conflict")
        );
        assert_eq!(
            spawn.args,
            [
                "run",
                "--project",
                "/dev/lg",
                "--",
                "claude",
                "--settings",
                "/dev/lg/.git/lg/sessions/dev-lg/settings.json",
                "resolve the conflict"
            ]
        );
    }

    #[test]
    fn a_blank_prompt_is_no_prompt_at_all() {
        let spawn = claude_spawn(Path::new("/dev/lg"), false, None, Some("   "));
        assert_eq!(spawn.args, ["--permission-mode", "auto"]);
    }

    #[test]
    fn sessions_declare_a_terminal_and_drop_the_nested_claude_markers() {
        let spawn = claude_spawn(Path::new("/dev/lg"), false, None, None);
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
        fake_of(id, cwd, SessionKind::Claude)
    }

    fn fake_of(id: u64, cwd: &str, kind: SessionKind) -> Session {
        Session {
            id: SessionId(id),
            label: format!("session {id}"),
            cwd: PathBuf::from(cwd),
            sandboxed: false,
            kind,
            status: SessionStatus::Running,
            attention: false,
            activity: SessionActivity::Idle,
            asking: false,
            events: None,
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

    /// Busy or ready is claude's to report, not lg's to guess: the spinner line
    /// says nothing lg reads any more. Matching it went wrong twice — once when
    /// `esc to interrupt` stopped being printed, once when a turn past a minute
    /// grew a `1m ` its clock did not have before.
    #[test]
    fn the_spinner_line_is_not_read_at_all() {
        for line in [
            "\u{273b} Enchanting\u{2026} (12s \u{b7} still thinking with xhigh effort)",
            "\u{273b} Enchanting\u{2026} (1m 12s \u{b7} esc to interrupt)",
            "\u{273b} Brewed for 4s",
        ] {
            assert!(!is_asking(line), "{line:?} is not a question");
        }
    }

    /// The prompt every fresh checkout opens on, and the reason the screen is
    /// still read: it is up before claude has run a single hook.
    #[test]
    fn the_trust_prompt_reads_as_needing_input() {
        let screen = "Quick safety check: Is this a project you created or one you trust?\n\
                      \u{276f} 1. Yes, I trust this folder\n  2. No, exit";
        assert!(is_asking(screen));
    }

    /// An answered question stays on screen with its chosen row ticked. Reading
    /// that as a live question is what left a session red after it had been
    /// dealt with.
    #[test]
    fn an_answered_question_stops_reading_as_one() {
        assert!(!is_asking("\u{276f} 1. Yes, I trust this folder \u{2714}"));
    }

    /// A permission prompt comes up mid-turn, so the hooks have the session down
    /// as working when it is really blocked on an answer. The question wins.
    #[test]
    fn a_question_outranks_what_the_hooks_last_said() {
        let mut session = fake(1, "/a");
        session.activity = SessionActivity::Working;
        session.asking = true;

        assert_eq!(session.activity(), SessionActivity::NeedsInput);
    }

    #[test]
    fn the_ordinary_prompt_caret_is_not_a_question() {
        for line in [
            "\u{276f} Try \"fix lint errors\"",
            "\u{276f} think carefully and write an essay",
            "\u{23f5}\u{23f5} auto mode on (shift+tab to cycle)",
        ] {
            assert!(
                !is_asking(line),
                "{line:?} is claude waiting for a command, not asking one"
            );
        }
    }

    #[test]
    fn prose_that_merely_mentions_a_question_is_not_a_question() {
        assert!(
            !is_asking("I can add the flag if you want. Do you want me to also update the docs?"),
            "an answer that uses the words is not a prompt with options to pick"
        );
    }

    #[test]
    fn a_quiet_screen_reads_as_ready() {
        assert!(!is_asking("Welcome to claude\n\n"));
    }

    /// Nothing has reported in yet, and nothing is being asked. A session that
    /// has said nothing is not busy.
    #[test]
    fn a_session_that_has_reported_nothing_reads_as_ready() {
        assert_eq!(fake(1, "/a").activity(), SessionActivity::Idle);
    }

    #[test]
    fn an_ended_session_is_never_reported_as_busy() {
        let mut session = fake(1, "/a");
        session.activity = SessionActivity::Working;
        assert_eq!(session.activity(), SessionActivity::Working);

        session.status = SessionStatus::Ended("stopped".into());
        assert_eq!(
            session.activity(),
            SessionActivity::Idle,
            "a dead session is not still working"
        );
    }

    #[test]
    fn a_checkout_has_at_most_one_session_of_each_kind() {
        let mut sessions = registry(&["/a", "/b"]);
        sessions.items.push(fake_of(3, "/b", SessionKind::Terminal));

        assert_eq!(
            sessions.for_dir_kind(Path::new("/b"), SessionKind::Claude),
            Some(SessionId(2))
        );
        assert_eq!(
            sessions.for_dir_kind(Path::new("/b"), SessionKind::Terminal),
            Some(SessionId(3)),
            "a terminal alongside claude is a second session, not the same one"
        );
        assert_eq!(
            sessions.for_dir_kind(Path::new("/c"), SessionKind::Claude),
            None
        );
    }

    #[test]
    fn a_checkout_lists_every_session_running_in_it() {
        let mut sessions = registry(&["/a", "/b"]);
        sessions.items.push(fake_of(3, "/b", SessionKind::Terminal));

        let ids: Vec<SessionId> = sessions
            .for_dir(Path::new("/b"))
            .map(|session| session.id)
            .collect();
        assert_eq!(ids, [SessionId(2), SessionId(3)]);
        assert!(sessions.any_in_dir(Path::new("/a")));
        assert!(!sessions.any_in_dir(Path::new("/c")));
    }

    /// The header reports these, and it must not cry wolf: a session that has
    /// only printed something is busy, not blocked.
    #[test]
    fn unread_output_is_not_counted_as_a_session_needing_input() {
        let mut sessions = registry(&["/a", "/b", "/c"]);
        sessions.get_mut(SessionId(1)).unwrap().activity = SessionActivity::Working;
        sessions.get_mut(SessionId(1)).unwrap().attention = true;
        sessions.get_mut(SessionId(2)).unwrap().asking = true;
        sessions.get_mut(SessionId(2)).unwrap().attention = true;

        assert_eq!(sessions.attention_count(), 2);
        assert_eq!(
            sessions.activity_counts(),
            (1, 1),
            "one blocked, one busy, one idle"
        );
    }

    /// Whatever a session last reported, a dead one is not waiting on anybody.
    #[test]
    fn an_ended_session_is_counted_as_neither_blocked_nor_busy() {
        let mut sessions = registry(&["/a"]);
        let session = sessions.get_mut(SessionId(1)).unwrap();
        session.asking = true;
        session.status = SessionStatus::Ended("exited".into());

        assert_eq!(sessions.activity_counts(), (0, 0));
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
