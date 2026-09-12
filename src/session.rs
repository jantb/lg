//! Terminal sessions lg keeps alive — one of each kind per checkout.
//!
//! A session is a program running on a pseudo terminal in one worktree, plus
//! the parsed screen it has drawn so far. Two sorts are on offer: a coding
//! agent — claude, codex or pi — and the user's own shell. Sessions keep
//! running and keep being read while lg shows something else, so several
//! checkouts can be worked on at once and switched between, and one checkout
//! can hold every kind at the same time.

use anyhow::Result;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::term::{PtyMsg, PtyProcess, Spawn};

mod reading;
mod registry;
mod spawn;
use reading::*;
pub use registry::{EndedSession, Sessions};
pub use spawn::{claude_spawn, codex_spawn, pi_spawn, shell_spawn};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
                    self.last_output = Some(std::time::Instant::now());
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

/// The size a session should run at, given the pane it is drawn in.
pub fn size_for_pane(area: ratatui::layout::Rect) -> (u16, u16) {
    let rows = area.height.saturating_sub(2).max(1);
    let cols = area.width.saturating_sub(2).max(1);
    (rows, cols)
}

/// Whether two paths name the same directory, by spelling or once resolved.
pub(crate) fn same_dir(a: &Path, b: &Path) -> bool {
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
        assert_eq!(
            spawn.program,
            crate::terrarium::executable().to_string_lossy()
        );
        assert_eq!(
            spawn.args,
            [
                "sandbox",
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
                "sandbox",
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

    /// Sandboxed, terrarium is the confinement, so codex's own sandbox — which
    /// would be nested inside it — is turned off rather than stacked.
    #[test]
    fn a_sandboxed_codex_leaves_the_confining_to_terrarium() {
        let spawn = codex_spawn(Path::new("/dev/lg.worktrees/feat-x"), true, None);
        assert_eq!(
            spawn.program,
            crate::terrarium::executable().to_string_lossy()
        );
        assert_eq!(
            spawn.args,
            [
                "sandbox",
                "run",
                "--project",
                "/dev/lg.worktrees/feat-x",
                "--",
                "codex",
                "--dangerously-bypass-approvals-and-sandbox",
            ]
        );
        assert_eq!(spawn.cwd, Path::new("/dev/lg.worktrees/feat-x"));
    }

    /// With nothing else holding it, codex holds itself to the checkout — as
    /// far as claude's auto mode goes here.
    #[test]
    fn an_unsandboxed_codex_confines_itself_to_the_checkout() {
        let spawn = codex_spawn(Path::new("/dev/lg"), false, None);
        assert_eq!(spawn.program, "codex");
        assert_eq!(spawn.args, ["--sandbox", "workspace-write"]);
        assert_eq!(spawn.cwd, Path::new("/dev/lg"));
    }

    #[test]
    fn a_sandboxed_pi_goes_through_terrarium_too() {
        let spawn = pi_spawn(Path::new("/dev/lg.worktrees/feat-x"), true, None);
        assert_eq!(
            spawn.program,
            crate::terrarium::executable().to_string_lossy()
        );
        assert_eq!(
            spawn.args,
            [
                "sandbox",
                "run",
                "--project",
                "/dev/lg.worktrees/feat-x",
                "--",
                "pi"
            ]
        );
    }

    #[test]
    fn an_unsandboxed_pi_runs_with_nothing_added() {
        let spawn = pi_spawn(Path::new("/dev/lg"), false, None);
        assert_eq!(spawn.program, "pi");
        assert!(spawn.args.is_empty(), "pi has no mode to ask for");
    }

    /// Every agent opens on the conflict lg hands it, and the prompt has to
    /// arrive as the first turn rather than as a flag — which is why pi's goes
    /// behind a `--`, since a prompt can start with a dash.
    #[test]
    fn every_agent_opens_on_the_prompt_it_is_given() {
        const PROMPT: &str = "- resolve the merge conflict";

        assert_eq!(
            claude_spawn(Path::new("/dev/lg"), false, None, Some(PROMPT))
                .args
                .last()
                .map(String::as_str),
            Some(PROMPT)
        );
        assert_eq!(
            codex_spawn(Path::new("/dev/lg"), false, Some(PROMPT))
                .args
                .last()
                .map(String::as_str),
            Some(PROMPT)
        );

        let pi = pi_spawn(Path::new("/dev/lg"), false, Some(PROMPT));
        assert_eq!(pi.args, ["--", PROMPT]);
    }

    #[test]
    fn a_blank_prompt_reaches_no_agent_as_an_empty_first_turn() {
        assert!(
            codex_spawn(Path::new("/dev/lg"), false, Some("   "))
                .args
                .iter()
                .all(|arg| arg.starts_with('-') || arg == "workspace-write")
        );
        assert!(
            pi_spawn(Path::new("/dev/lg"), false, Some("   "))
                .args
                .is_empty()
        );
    }

    /// One of each per checkout, so a worktree can be running claude, codex, pi
    /// and a shell at the same time without any of them replacing another.
    #[test]
    fn a_checkout_holds_one_session_of_every_kind_at_once() {
        let mut sessions = Sessions::default();
        for (id, kind) in [
            SessionKind::Claude,
            SessionKind::Codex,
            SessionKind::Pi,
            SessionKind::Terminal,
        ]
        .into_iter()
        .enumerate()
        {
            sessions.items.push(fake_of(id as u64, "/dev/lg", kind));
        }

        for kind in [
            SessionKind::Claude,
            SessionKind::Codex,
            SessionKind::Pi,
            SessionKind::Terminal,
        ] {
            assert!(
                sessions.for_dir_kind(Path::new("/dev/lg"), kind).is_some(),
                "{} should have its own session here",
                kind.label()
            );
        }
        assert_eq!(sessions.for_dir(Path::new("/dev/lg")).count(), 4);
    }

    #[test]
    fn a_terminal_runs_the_users_shell_with_no_arguments() {
        temp_env("SHELL", "/bin/zsh", || {
            let spawn = shell_spawn(Path::new("/dev/lg"));
            assert_eq!(spawn.program, "/bin/zsh");
            assert!(spawn.args.is_empty(), "an interactive shell needs no flags");
            assert_eq!(spawn.cwd, Path::new("/dev/lg"));
        });
    }

    #[test]
    fn a_terminal_falls_back_to_a_shell_every_machine_has() {
        temp_env("SHELL", "", || {
            assert_eq!(shell_spawn(Path::new("/dev/lg")).program, "/bin/sh");
        });
    }

    /// A shell is where claude gets started by hand, so it must not inherit the
    /// markers that would make that claude think it is nested.
    #[test]
    fn a_terminal_declares_a_terminal_and_drops_the_nested_claude_markers() {
        let spawn = shell_spawn(Path::new("/dev/lg"));
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
                "sandbox",
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
            launch: None,
            attention: false,
            activity: SessionActivity::Idle,
            asking: false,
            events: None,
            last_output: None,
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

    /// A shell or an agent without hooks has only its screen to be judged by:
    /// while it is writing it is working, and once it has been quiet for a
    /// moment it is ready again.
    #[test]
    fn a_program_without_hooks_is_working_while_it_writes() {
        let mut session = fake_of(1, "/a", SessionKind::Terminal);
        assert_eq!(session.activity(), SessionActivity::Idle);
        session.last_output = Some(std::time::Instant::now());
        assert_eq!(session.activity(), SessionActivity::Working);
        session.last_output = Some(
            std::time::Instant::now()
                - std::time::Duration::from_millis(OUTPUT_ACTIVE_MS as u64 + 500),
        );
        assert_eq!(session.activity(), SessionActivity::Idle);
    }

    /// codex asks before running a command the way claude does, with a choice
    /// list under the question, so it gets the same red dot. Prose that merely
    /// mentions approving something does not.
    #[test]
    fn a_codex_approval_prompt_is_a_question() {
        let screen =
            "Allow command?\n\u{276f} 1. Yes (y)\n  2. Yes, and don't ask again\n  3. No\n";
        assert!(is_asking(screen));
        assert!(!is_asking(
            "I approve of this change; the reviewer approved it too.\n1. Notes\n"
        ));
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

    #[test]
    fn only_working_sessions_request_animation_frames() {
        let mut state = crate::state::AppState::new();
        state.sessions = registry(&["/a"]);
        assert!(!state.wants_animation());
        state.sessions.get_mut(SessionId(1)).unwrap().activity = SessionActivity::Working;
        assert!(state.wants_animation());
        state.sessions.get_mut(SessionId(1)).unwrap().asking = true;
        assert!(
            !state.wants_animation(),
            "waiting for input should hold steady"
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
