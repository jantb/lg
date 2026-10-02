use anyhow::{Context, Result};
use chrono::Utc;
use notify::RecommendedWatcher;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::{
    event::{self, Event},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::{
    io::{BufWriter, Stdout, Write},
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use crate::{
    config::{
        BACKGROUND_FETCH_INTERVAL_SECS, ERROR_MSG_LIFETIME_SECS, FRAME_BUFFER_BYTES,
        MAX_EVENTS_PER_FRAME, STATUS_MSG_LIFETIME_SECS,
    },
    state::AppState,
};

mod actions;
mod conflict_assist;
mod conflict_editor;
mod footer;
mod frame;
mod github;
mod header;
mod input;
mod jobs;
mod mouse;
mod refresh;
mod render;
mod review_agent;
mod review_assist;
mod session;
mod spawn;
mod watch;
mod workflow;

pub(crate) use conflict_assist::spawn_conflict_resolve;
pub(crate) use conflict_editor::{prepare_conflict_editor, reopen_conflicts, save_conflict_editor};
pub(crate) use spawn::{
    checkout_branch_async, checkout_nested_branch_async, checkout_nested_remote_branch_async,
    checkout_remote_branch_async,
};
pub(crate) use workflow::{
    abort_conflict_operation, run_flow_action, start_conflict_session,
    validate_conflict_resolution, workflow_steps,
};

use refresh::{prime_roots, refresh_snapshot, startup_roots, watch_repo};
use review_assist::{
    spawn_assisted_review, spawn_review_assist, spawn_review_chat, spawn_review_pr_text,
    spawn_review_style_flags,
};
pub(crate) use spawn::open_model_modal;
use spawn::{
    git_job_running, load_diff_text, open_author_modal, selected_commit_ref, selected_diff_source,
    spawn_force_push, spawn_operation, spawn_operation_with_progress, spawn_pull, spawn_push,
};

pub struct App {
    pub state: AppState,
    pub terminal: Terminal<CrosstermBackend<BufWriter<Stdout>>>,
    file_events: Receiver<notify::Result<notify::Event>>,
    /// Held so the watcher keeps running; replaced when lg switches checkout.
    file_watcher: RecommendedWatcher,
    /// What the watcher is watching, to tell which events matter.
    watch_roots: watch::WatchRoots,
    /// File events waiting for the burst they came in to settle.
    file_batch: watch::FileBatch,
    last_fetch_started: Instant,
    /// Whether the terminal is currently set up for a session to hold the
    /// keyboard: pastes reported as pastes, and keys spelled out rather than
    /// flattened. lg's own panels want neither.
    session_keyboard: bool,
    /// Whether the terminal can spell keys out at all, asked once at startup
    /// because the answer costs a round trip.
    keys_can_disambiguate: bool,
}

pub struct HeadlessApp<B: Backend> {
    pub state: AppState,
    pub terminal: Terminal<B>,
}

fn drain_pending_terminal_events() {
    // Drain in two passes with a brief wait between, since the terminal
    // may keep flushing in-flight mouse-event escape sequences for a few
    // milliseconds after DisableMouseCapture is sent. Without this drain
    // those bytes leak into the shell's stdin after we exit and print
    // as raw escape characters at the prompt.
    for pass in 0..2 {
        for _ in 0..16384 {
            match event::poll(Duration::from_millis(0)) {
                Ok(true) => {
                    let _ = event::read();
                }
                _ => break,
            }
        }
        if pass == 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Whether the enhancement flags are currently on the terminal's stack. A
/// static because the panic hook restores the terminal without an `App` to ask.
static KEYS_DISAMBIGUATED: AtomicBool = AtomicBool::new(false);

/// Ask the terminal to spell keys out rather than flatten them, which is what
/// tells Shift+Enter apart from Enter. Only while a session holds the keyboard:
/// lg's own panels are written against what a plain terminal sends.
fn set_key_disambiguation<W: Write>(output: &mut W, on: bool) {
    if KEYS_DISAMBIGUATED.load(Ordering::Relaxed) == on {
        return;
    }
    let changed = if on {
        execute!(
            output,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
    } else {
        execute!(output, PopKeyboardEnhancementFlags)
    };
    if changed.is_ok() {
        KEYS_DISAMBIGUATED.store(on, Ordering::Relaxed);
    }
}

/// The terminal editor a file is edited in: `$VISUAL`, else `$EDITOR`, else
/// `vi`, split into the program and the arguments it was given.
fn terminal_editor() -> (String, Vec<String>) {
    let configured = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "vi".to_string());
    let mut words = configured.split_whitespace().map(str::to_string);
    let program = words.next().unwrap_or_else(|| "vi".to_string());
    (program, words.collect())
}

/// How `program` is told to open `path` at `line`. Most terminal editors take
/// `+LINE`; the few that do not take `path:line`.
fn editor_line_args(program: &str, path: &str, line: usize) -> Vec<String> {
    let name = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    match name {
        "hx" | "helix" | "zed" => vec![format!("{path}:{line}")],
        "code" | "cursor" | "codium" => vec!["-g".into(), format!("{path}:{line}")],
        _ => vec![format!("+{line}"), path.to_string()],
    }
}

fn restore_terminal<W: Write>(output: &mut W) {
    set_key_disambiguation(output, false);
    let _ = execute!(output, DisableMouseCapture, DisableBracketedPaste);
    let _ = output.flush();
    drain_pending_terminal_events();
    let _ = execute!(output, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    let _ = output.flush();
}

// ─── HeadlessApp ─────────────────────────────────────────────────────────────

impl<B: Backend> HeadlessApp<B>
where
    B::Error: Send + Sync + 'static,
{
    pub fn new(backend: B) -> Result<Self> {
        let terminal = Terminal::new(backend).context("create headless terminal")?;
        Ok(Self {
            state: AppState::new(),
            terminal,
        })
    }
}

// ─── App ─────────────────────────────────────────────────────────────────────

impl App {
    /// Hand the terminal to the editor on `path` at `line`, and take it back
    /// once the editor exits.
    ///
    /// lg leaves its screen exactly as it does when quitting, so the editor
    /// gets a plain terminal, and puts it back the way [`App::new`] set it up.
    /// Nothing is read from the keyboard meanwhile: the loop is right here,
    /// waiting.
    pub(super) fn edit_in_terminal(&mut self, path: &str, line: usize) -> Result<()> {
        let root = self
            .state
            .repo_root
            .clone()
            .context("no checkout to edit in")?;
        let (program, mut args) = terminal_editor();
        args.extend(editor_line_args(&program, path, line));
        restore_terminal(self.terminal.backend_mut());
        let status = std::process::Command::new(&program)
            .args(&args)
            .current_dir(&root)
            .status();
        enable_raw_mode().context("enable raw mode")?;
        execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture
        )
        .context("enter alt screen")?;
        // restore_terminal turned off what session mode had on; the next
        // frame puts back whatever the screen it returns to wants.
        self.session_keyboard = false;
        self.sync_session_keyboard();
        self.terminal.clear().context("redraw after the editor")?;
        let status = status.with_context(|| format!("could not start {program}"))?;
        if !status.success() {
            anyhow::bail!("{program} exited with {status}");
        }
        Ok(())
    }

    pub fn new() -> Result<Self> {
        // Pin git to the directory lg opens on up front: every command runs
        // against a directory lg chose, not whichever one the process happens
        // to sit in, which is what lets checkouts be switched underneath.
        let roots = startup_roots()?;
        let start_dir = roots.start_dir;
        crate::git::set_active_repo(&start_dir);

        let (file_watcher, file_events, watch_roots) = watch_repo(&start_dir)?;

        // Off the startup path on purpose: asking the model server what it
        // serves is a round trip, and nothing here waits on the answer.
        if crate::preferences::ai_enabled() {
            crate::llm::prime_models_async();
        }

        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let mut stdout = std::io::stdout();
            restore_terminal(&mut stdout);
            prev_hook(info);
        }));

        enable_raw_mode().context("enable raw mode")?;
        let mut stdout = BufWriter::with_capacity(FRAME_BUFFER_BYTES, std::io::stdout());
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context("enter alt screen")?;

        // Asked before the loop starts: the query waits on the terminal's
        // reply, which is not something to do between frames.
        let keys_can_disambiguate = supports_keyboard_enhancement().unwrap_or(false);

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("create terminal")?;

        let mut app = Self {
            state: AppState::new(),
            terminal,
            file_events,
            file_watcher,
            watch_roots,
            file_batch: watch::FileBatch::default(),
            last_fetch_started: Instant::now()
                - Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS),
            session_keyboard: false,
            keys_can_disambiguate,
        };
        app.state.workspace_root = roots
            .workspace
            .map(|workspace| workspace.to_string_lossy().into_owned());
        prime_roots(&mut app.state);
        app.state.enable_history();
        // Whether this author has them turned off takes four git calls to
        // find out; the refresh below finds out, and until it lands the
        // setting stands on its own.
        app.state.decorative_animations = crate::preferences::load()
            .config
            .tools
            .decorative_animations;
        app.start_refresh(true);
        app.start_fetch();
        Ok(app)
    }

    pub fn run(&mut self) -> Result<()> {
        let mut clock = frame::FrameClock::default();
        loop {
            if self.state.should_quit {
                break;
            }

            let delivered = crate::state::deliveries();
            let activity = self.state.sessions.activity_signature();
            let status = self.status_mark();
            self.drain_generation();
            self.drain_review_assist();
            self.drain_review_pr_text();
            self.drain_review_agent();
            self.drain_review_flag_job();
            self.drain_conflict_resolve_job();
            self.drain_review_chat();
            self.drain_push_job()?;
            self.drain_checkout_job()?;
            self.drain_operation_job()?;
            self.drain_fetch_job();
            self.drain_refresh_job();
            self.drain_release_status_job();
            self.drain_settings_suggest_job();
            self.drain_commit_log_job();
            self.drain_diff_job();
            self.drain_review_job();
            self.drain_workflow_job()?;
            render::poll_panels(&mut self.state);
            let sessions = self.drain_sessions();
            self.state.reap_deferred_threads();
            let refreshing = self.drain_file_events()?;
            self.maybe_start_periodic_fetch();

            let now = Instant::now();
            if refreshing
                || crate::state::deliveries() != delivered
                || self.state.sessions.activity_signature() != activity
                || self.status_mark() != status
                || sessions.ended_any
            {
                clock.changed();
            }
            if sessions.shown_changed {
                clock.changed();
                clock.shown_live(now);
            }
            let pace = frame::pace(&self.state, clock.is_shown_live(now), sessions.more);
            self.sync_session_keyboard();
            if clock.due(now, pace) {
                self.render()?;
                clock.drawn(Instant::now());
            }

            let timeout = clock.timeout(Instant::now(), pace, self.next_deadline());
            if event::poll(timeout)? {
                // Take everything already queued rather than one event per
                // frame. Each frame is a redraw and a pass over every job, so
                // spreading a wheel burst across frames made scrolling crawl
                // along behind the trackpad.
                let mut handled = 0usize;
                loop {
                    match event::read()? {
                        Event::Key(k) => self.handle_key(k)?,
                        Event::Mouse(m) => self.handle_mouse(m)?,
                        Event::Paste(text) => {
                            if !crate::panel::settings::handle_paste(&mut self.state, &text)
                                && !crate::panel::conflict::handle_paste(&mut self.state, &text)
                                && !crate::panel::github::handle_paste(&mut self.state, &text)
                                && !crate::panel::guided::handle_paste(&mut self.state, &text)
                            {
                                session::forward_paste(&mut self.state, &text);
                            }
                        }
                        Event::Resize(_, _) => {}
                        _ => {}
                    }
                    handled += 1;
                    if !keep_reading_events(&self.state, handled) || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
                clock.input();
                if self.state.session_input_active() {
                    // What was typed echoes back in a moment.
                    clock.shown_live(Instant::now());
                }
            }

            // Dispatch pending IO action.
            if let Some(action) = self.state.pending_action.take() {
                self.dispatch_pending(action);
                clock.input();
            }

            // Expire stale status messages.
            if let Some(ref s) = self.state.status.clone() {
                let lifetime = if s.is_error {
                    ERROR_MSG_LIFETIME_SECS
                } else {
                    STATUS_MSG_LIFETIME_SECS
                };
                if (Utc::now() - s.at).num_seconds() >= lifetime {
                    self.state.status = None;
                    clock.changed();
                }
            }
        }
        Ok(())
    }

    /// Which status message is up, to notice one put up by anything the loop
    /// did between frames.
    fn status_mark(&self) -> Option<(chrono::DateTime<Utc>, bool)> {
        self.state
            .status
            .as_ref()
            .map(|status| (status.at, status.is_error))
    }

    /// The next moment the loop has to be awake for, besides input and frames:
    /// a batch of file events whose quiet period ends.
    fn next_deadline(&self) -> Option<Instant> {
        self.file_batch.deadline()
    }
}

/// Whether another queued event may be taken before drawing again. A pending
/// action has to run first, because a second event would replace it before it
/// ever executed, and quitting stops the batch there and then.
fn keep_reading_events(state: &AppState, handled: usize) -> bool {
    state.pending_action.is_none() && !state.should_quit && handled < MAX_EVENTS_PER_FRAME
}

impl Drop for App {
    fn drop(&mut self) {
        // The terminal comes back first, so a session that is slow to die does
        // not hold it hostage. Then the sessions are closed here rather than
        // left to whenever the state happens to be dropped, which is what the
        // quit prompt promised.
        restore_terminal(self.terminal.backend_mut());
        self.state.sessions.close_all();
        self.join_background_jobs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_editor_is_told_the_line_the_way_it_reads_it() {
        assert_eq!(editor_line_args("vim", "src/a.rs", 12), ["+12", "src/a.rs"]);
        assert_eq!(editor_line_args("/usr/bin/nvim", "a.rs", 3), ["+3", "a.rs"]);
        assert_eq!(editor_line_args("hx", "a.rs", 3), ["a.rs:3"]);
        assert_eq!(editor_line_args("code", "a.rs", 3), ["-g", "a.rs:3"]);
    }
    use crate::state::PendingAction;

    #[test]
    fn a_burst_of_events_is_taken_in_one_batch() {
        let state = AppState::new();
        assert!(keep_reading_events(&state, 0));
        assert!(keep_reading_events(&state, MAX_EVENTS_PER_FRAME - 1));
    }

    #[test]
    fn the_batch_stops_so_a_pending_action_can_run() {
        let mut state = AppState::new();
        state.pending_action = Some(PendingAction::Quit);
        assert!(
            !keep_reading_events(&state, 1),
            "a second event would replace the action before it ever ran"
        );
    }

    #[test]
    fn the_batch_stops_on_quit() {
        let mut state = AppState::new();
        state.should_quit = true;
        assert!(!keep_reading_events(&state, 1));
    }

    #[test]
    fn a_flood_still_yields_to_the_redraw() {
        let state = AppState::new();
        assert!(
            !keep_reading_events(&state, MAX_EVENTS_PER_FRAME),
            "scrolling is only visible if the frame is drawn"
        );
    }
}
