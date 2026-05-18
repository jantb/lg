use anyhow::{Context, Result};
use chrono::Utc;
use notify::RecommendedWatcher;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
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
    config::{BACKGROUND_FETCH_INTERVAL_SECS, STATUS_MSG_LIFETIME_SECS, TICK_MS},
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
    watch_current_dir,
};
use review_assist::{
    spawn_assisted_review, spawn_review_assist, spawn_review_chat, spawn_review_style_flags,
};
use spawn::{
    git_job_running, load_diff_text, open_author_modal, selected_commit_ref, selected_diff_source,
    spawn_operation, spawn_pull, spawn_push,
};

pub struct App {
    pub state: AppState,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    file_events: Receiver<notify::Result<notify::Event>>,
    _file_watcher: RecommendedWatcher,
    last_fetch_started: Instant,
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
    let _ = execute!(output, DisableMouseCapture);
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

        let (_file_watcher, file_events) = watch_current_dir()?;

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
            _file_watcher,
            last_fetch_started: Instant::now()
                - Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS),
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

            self.render()?;

            self.drain_generation();
            self.drain_review_assist();
            self.drain_review_flag_job();
            self.drain_review_chat();
            self.drain_push_job()?;
            self.drain_checkout_job()?;
            self.drain_operation_job()?;
            self.drain_fetch_job();
            self.drain_refresh_job();
            self.drain_release_status_job();
            self.drain_commit_log_job();
            self.drain_diff_job();
            self.drain_review_job();
            self.drain_workflow_job()?;
            self.state.reap_deferred_threads();
            self.drain_file_events()?;
            self.maybe_start_periodic_fetch();

            let poll_ms = if self.state.generation.is_some()
                || self.state.push_job.is_some()
                || self.state.checkout_job.is_some()
                || self.state.operation_job.is_some()
                || self.state.fetch_job.is_some()
                || self.state.refresh_job.is_some()
                || self.state.release_status_job.is_some()
                || self.state.commit_log_job.is_some()
                || self.state.diff_job.is_some()
                || self.state.review_job.is_some()
                || self.state.review_assist_job.is_some()
                || self.state.review_flag_job.is_some()
                || self.state.review_chat_job.is_some()
                || self.state.workflow_job.is_some()
            {
                80
            } else {
                TICK_MS
            };
            if event::poll(Duration::from_millis(poll_ms))? {
                match event::read()? {
                    Event::Key(k) => self.handle_key(k)?,
                    Event::Mouse(m) => self.handle_mouse(m)?,
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }

            // Dispatch pending IO action.
            if let Some(action) = self.state.pending_action.take() {
                self.dispatch_pending(action);
            }

            // Expire stale status messages.
            if let Some(ref s) = self.state.status.clone() {
                if (Utc::now() - s.at).num_seconds() >= STATUS_MSG_LIFETIME_SECS {
                    self.state.status = None;
                }
            }
        }
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        restore_terminal(self.terminal.backend_mut());
        self.join_background_jobs();
    }
}
