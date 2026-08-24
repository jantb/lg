use anyhow::{Context, Result};
use chrono::Utc;
use notify::RecommendedWatcher;
use ratatui::crossterm::event::{DisableBracketedPaste, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::{
    io::{Stdout, Write},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use crate::{
    config::{
        BACKGROUND_FETCH_INTERVAL_SECS, ERROR_MSG_LIFETIME_SECS, JOB_TICK_MS, MAX_EVENTS_PER_FRAME,
        SESSION_TICK_MS, STATUS_MSG_LIFETIME_SECS, TICK_MS,
    },
    state::AppState,
};

mod actions;
mod footer;
mod header;
mod input;
mod jobs;
mod mouse;
mod refresh;
mod render;
mod review_assist;
mod session;
mod spawn;
mod workflow;

pub(crate) use spawn::{
    checkout_branch_async, checkout_nested_branch_async, checkout_nested_remote_branch_async,
    checkout_remote_branch_async,
};
pub(crate) use workflow::{
    abort_conflict_operation, run_flow_action, validate_conflict_resolution,
};

use refresh::{
    build_refresh_snapshot, prime_branches, prime_files, should_refresh_for_fs_event,
    startup_repo_root, watch_repo,
};
use review_assist::{
    spawn_assisted_review, spawn_review_assist, spawn_review_chat, spawn_review_pr_text,
    spawn_review_style_flags,
};
use spawn::{
    git_job_running, load_diff_text, open_author_modal, open_model_modal, selected_commit_ref,
    selected_diff_source, spawn_operation, spawn_operation_with_progress, spawn_pull, spawn_push,
};

pub struct App {
    pub state: AppState,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    file_events: Receiver<notify::Result<notify::Event>>,
    /// Held so the watcher keeps running; replaced when lg switches checkout.
    file_watcher: RecommendedWatcher,
    last_fetch_started: Instant,
    /// Whether the terminal is currently reporting pastes as pastes, which lg
    /// only wants while a session holds the keyboard.
    bracketed_paste: bool,
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

fn restore_terminal<W: Write>(output: &mut W) {
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
    pub fn new() -> Result<Self> {
        if !crate::git::is_repo() {
            anyhow::bail!("not a git repository (or any parent up to mount point)");
        }

        // Pin git to the repository root up front: every command then runs
        // against a directory lg chose, not whichever one the process happens
        // to sit in, which is what lets checkouts be switched underneath.
        let repo_root = startup_repo_root()?;
        crate::git::set_active_repo(&repo_root);

        let (file_watcher, file_events) = watch_repo(&repo_root)?;

        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let mut stdout = std::io::stdout();
            restore_terminal(&mut stdout);
            prev_hook(info);
        }));

        enable_raw_mode().context("enable raw mode")?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context("enter alt screen")?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("create terminal")?;

        let mut app = Self {
            state: AppState::new(),
            terminal,
            file_events,
            file_watcher,
            last_fetch_started: Instant::now()
                - Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS),
            bracketed_paste: false,
        };
        prime_branches(&mut app.state);
        prime_files(&mut app.state);
        app.start_refresh(true);
        app.start_fetch();
        Ok(app)
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            if self.state.should_quit {
                break;
            }

            self.sync_bracketed_paste();
            self.render()?;

            self.drain_generation();
            self.drain_review_assist();
            self.drain_review_pr_text();
            self.drain_review_flag_job();
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
            self.drain_sessions();
            self.state.reap_deferred_threads();
            self.drain_file_events()?;
            self.maybe_start_periodic_fetch();

            let poll_ms = if self.state.session_view().is_some() {
                SESSION_TICK_MS
            } else if self.state.any_job_running() {
                JOB_TICK_MS
            } else {
                TICK_MS
            };
            if event::poll(Duration::from_millis(poll_ms))? {
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
                            session::forward_paste(&mut self.state, &text);
                        }
                        Event::Resize(_, _) => {}
                        _ => {}
                    }
                    handled += 1;
                    if !keep_reading_events(&self.state, handled) || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }

            // Dispatch pending IO action.
            if let Some(action) = self.state.pending_action.take() {
                self.dispatch_pending(action);
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
                }
            }
        }
        Ok(())
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
