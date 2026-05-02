use anyhow::{Context, Result};
use chrono::Utc;
use notify::RecommendedWatcher;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::{
    event::{
        self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::Rect,
};
use std::{
    backtrace::Backtrace,
    fs::OpenOptions,
    io::{Stdout, Write},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use crate::{
    config::{
        BACKGROUND_FETCH_INTERVAL_SECS, BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST, COMMIT_LIST_LIMIT,
        STATUS_MSG_LIFETIME_SECS, TICK_MS,
    },
    panel,
    state::{
        AppState, CheckoutMsg, CommitLogJob, CommitLogMsg, DiffJob, DiffMsg, DiffSource, FetchJob,
        FetchMsg, GenMsg, Modal, OperationKind, OperationMsg, Pane, PendingAction, PushMsg,
        RefreshJob, RefreshMsg, ReleaseStatusJob, ReleaseStatusMsg, ReviewMsg, WorkflowMsg,
    },
    ui,
};

mod footer;
mod mouse;
mod refresh;
mod review_assist;
mod spawn;
mod workflow;

pub(crate) use spawn::checkout_branch_async;
pub(crate) use workflow::{
    abort_conflict_operation, run_flow_action, validate_conflict_resolution,
};

use refresh::{
    build_refresh_snapshot, prime_branches, should_refresh_for_fs_event, watch_current_dir,
};
use review_assist::{spawn_assisted_review, spawn_review_assist};
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

/// Headless app backed by a generic [`Backend`]; used by tests and the harness.
pub struct HeadlessApp<B: Backend> {
    pub state: AppState,
    pub terminal: Terminal<B>,
}

fn next_pane(p: Pane) -> Pane {
    match p {
        Pane::Status => Pane::Files,
        Pane::Files => Pane::Branches,
        Pane::Branches => Pane::Commits,
        Pane::Commits => Pane::Main,
        Pane::Main => Pane::Status,
    }
}

fn prev_pane(p: Pane) -> Pane {
    match p {
        Pane::Status => Pane::Main,
        Pane::Files => Pane::Status,
        Pane::Branches => Pane::Files,
        Pane::Commits => Pane::Branches,
        Pane::Main => Pane::Commits,
    }
}

fn first_status_line(s: &str) -> String {
    s.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(s)
        .chars()
        .take(120)
        .collect()
}

pub fn trace_event(kind: &str, message: impl AsRef<str>) {
    let Some(path) = std::env::var_os("LG_TRACE") else {
        return;
    };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(f, "{kind} {}", message.as_ref());
}

fn trace_scroll(message: impl AsRef<str>) {
    trace_event("SCROLL", message);
}

fn trace_lifecycle(message: impl AsRef<str>) {
    trace_event("LIFECYCLE", message);
}

fn trace_state_summary(state: &AppState) -> String {
    format!(
        "focus={:?} modal={:?} should_quit={} diff_offset={} max_offset={} viewport={} width={} source={:?} pending_action={} jobs={{gen:{} push:{} checkout:{} op:{} fetch:{} refresh:{} release:{} commit_log:{} diff:{} review:{} assist:{} workflow:{}}}",
        state.focus,
        state.modal,
        state.should_quit,
        state.diff_offset,
        panel::main::max_scroll_offset(state),
        state.diff_viewport_height,
        state.diff_viewport_width,
        state.diff_source,
        state.pending_action.is_some(),
        state.generation.is_some(),
        state.push_job.is_some(),
        state.checkout_job.is_some(),
        state.operation_job.is_some(),
        state.fetch_job.is_some(),
        state.refresh_job.is_some(),
        state.release_status_job.is_some(),
        state.commit_log_job.is_some(),
        state.diff_job.is_some(),
        state.review_job.is_some(),
        state.review_assist_job.is_some(),
        state.workflow_job.is_some(),
    )
}

fn trace_panic(info: &std::panic::PanicHookInfo<'_>) {
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");
    let location = info
        .location()
        .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
        .unwrap_or_else(|| "<unknown location>".into());
    trace_event("PANIC", format!("location={location} payload={payload:?}"));
    trace_event("BACKTRACE", format!("{}", Backtrace::force_capture()));
}

fn mouse_scroll_snapshot(state: &AppState, pane: Pane) -> (usize, usize) {
    match pane {
        Pane::Files => (state.files_idx, state.tree_rows().len()),
        Pane::Branches => (state.branches_idx, state.branches.len()),
        Pane::Commits => (state.commits_idx, state.commits.len()),
        Pane::Status | Pane::Main => (0, 0),
    }
}

fn drain_pending_terminal_events() {
    for _ in 0..1024 {
        match event::poll(Duration::from_millis(0)) {
            Ok(true) => {
                let _ = event::read();
            }
            _ => break,
        }
    }
}

fn restore_terminal<W: Write>(output: &mut W) {
    trace_lifecycle("restore_terminal begin");
    let _ = execute!(output, DisableMouseCapture);
    trace_lifecycle("restore_terminal mouse_disabled");
    drain_pending_terminal_events();
    trace_lifecycle("restore_terminal events_drained");
    let _ = execute!(output, LeaveAlternateScreen);
    trace_lifecycle("restore_terminal left_alternate_screen");
    let _ = disable_raw_mode();
    trace_lifecycle("restore_terminal raw_mode_disabled");
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

    pub fn render(&mut self) -> Result<()> {
        self.state.advance_animation();

        // Compute viewport height before the draw closure so we can update state.
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects_pre = ui::split_layout_with_sizes(
            area,
            self.state.flow_available(),
            self.state.left_column_width,
            self.state.left_panel_heights,
        );
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);
        self.state.diff_viewport_width = rects_pre.main.width.saturating_sub(2);
        self.clamp_main_scroll_offset();

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout_with_sizes(
                area,
                state.flow_available(),
                state.left_column_width,
                state.left_panel_heights,
            );
            let focused_pane = state.focus;

            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
            panel::environments::render(state, rects.environments, frame);
            panel::files::render(state, rects.files, frame, focused_pane == Pane::Files);
            panel::branches::render(state, rects.branches, frame, focused_pane == Pane::Branches);
            panel::commits::render(state, rects.commits, frame, focused_pane == Pane::Commits);
            panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);

            footer::draw(frame, rects.footer, state);

            match state.modal {
                Modal::None => {}
                Modal::Commit => panel::commit::render(state, area, frame),
                Modal::Push => panel::push::render(state, area, frame),
                Modal::Author => panel::author::render(state, area, frame),
                Modal::Help => panel::help::render(state, area, frame),
                Modal::Flow => panel::flow::render(state, area, frame),
                Modal::Conflict => panel::conflict::render(state, area, frame),
                Modal::DeleteBranch => panel::delete_branch::render(state, area, frame),
            }
        })?;
        Ok(())
    }

    fn clamp_main_scroll_offset(&mut self) {
        self.state.diff_offset = self
            .state
            .diff_offset
            .min(panel::main::max_scroll_offset(&self.state));
    }

    pub fn send_key(&mut self, k: KeyEvent) -> Result<()> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return self.render();
        }
        match self.state.modal {
            Modal::Help => {
                panel::help::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Commit => {
                panel::commit::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Push => {
                panel::push::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Author => {
                panel::author::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Flow => {
                panel::flow::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::Conflict => {
                panel::conflict::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::DeleteBranch => {
                panel::delete_branch::handle_key(&mut self.state, k)?;
                return self.render();
            }
            Modal::None => {}
        }
        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
            }
            KeyCode::Char('F') => {
                if self.state.flow_available() {
                    self.state.modal = Modal::Flow;
                }
            }
            KeyCode::Char('q') => {
                self.state.should_quit = true;
            }
            KeyCode::Esc => {}
            KeyCode::Char('1') => {
                self.state.focus = Pane::Status;
            }
            KeyCode::Char('2') => {
                self.state.focus = Pane::Files;
            }
            KeyCode::Char('3') => {
                self.state.focus = Pane::Branches;
            }
            KeyCode::Char('4') => {
                self.state.focus = Pane::Commits;
            }
            KeyCode::Char('0') => {
                self.state.focus = Pane::Main;
            }
            KeyCode::Tab => {
                self.state.focus = next_pane(self.state.focus);
            }
            KeyCode::BackTab => {
                self.state.focus = prev_pane(self.state.focus);
            }
            KeyCode::Char('c') => {
                self.state.open_commit_modal();
            }
            KeyCode::Char('a') => {
                open_author_modal(&mut self.state);
            }
            KeyCode::Char('p') => {
                if self.state.pull_available() {
                    self.state.pending_action = Some(PendingAction::Pull);
                } else {
                    self.state.set_status("nothing to pull", false);
                }
            }
            KeyCode::Char('f') => {
                self.state
                    .set_status("fetch unavailable in headless", false);
            }
            KeyCode::Char('P') => {
                if self.state.unpushed_shas.is_empty() {
                    self.state.set_status("nothing to push", false);
                } else {
                    spawn_push(&mut self.state);
                }
            }
            KeyCode::Char('R') => {
                spawn_assisted_review(&mut self.state);
            }
            _ => match self.state.focus {
                Pane::Status => {}
                Pane::Files => panel::files::handle_key(&mut self.state, k)?,
                Pane::Branches => panel::branches::handle_key(&mut self.state, k)?,
                Pane::Commits => panel::commits::handle_key(&mut self.state, k)?,
                Pane::Main => panel::main::handle_key(&mut self.state, k)?,
            },
        }
        self.render()
    }
}

// ─── App ─────────────────────────────────────────────────────────────────────

impl App {
    pub fn new() -> Result<Self> {
        trace_lifecycle("app_new begin");
        if !crate::git::is_repo() {
            trace_lifecycle("app_new not_git_repo");
            anyhow::bail!("not a git repository (or any parent up to mount point)");
        }

        trace_lifecycle("app_new watch_current_dir begin");
        let (_file_watcher, file_events) = watch_current_dir()?;
        trace_lifecycle("app_new watch_current_dir end");

        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            trace_panic(info);
            let mut stdout = std::io::stdout();
            restore_terminal(&mut stdout);
            prev_hook(info);
        }));
        trace_lifecycle("app_new panic_hook_installed");

        trace_lifecycle("app_new enable_raw_mode begin");
        enable_raw_mode().context("enable raw mode")?;
        trace_lifecycle("app_new enable_raw_mode end");
        let mut stdout = std::io::stdout();
        trace_lifecycle("app_new enter_alt_mouse begin");
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context("enter alt screen")?;
        trace_lifecycle("app_new enter_alt_mouse end");

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("create terminal")?;
        trace_lifecycle("app_new terminal_created");

        let mut app = Self {
            state: AppState::new(),
            terminal,
            file_events,
            _file_watcher,
            last_fetch_started: Instant::now()
                - Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS),
        };
        prime_branches(&mut app.state);
        trace_lifecycle(format!(
            "app_new prime_branches end {}",
            trace_state_summary(&app.state)
        ));
        app.start_refresh(true);
        trace_lifecycle(format!(
            "app_new start_refresh end {}",
            trace_state_summary(&app.state)
        ));
        app.start_fetch();
        trace_lifecycle(format!(
            "app_new start_fetch end {}",
            trace_state_summary(&app.state)
        ));
        Ok(app)
    }

    pub fn run(&mut self) -> Result<()> {
        trace_lifecycle(format!("run enter {}", trace_state_summary(&self.state)));
        let mut loop_iter: u64 = 0;
        loop {
            loop_iter = loop_iter.saturating_add(1);
            if self.state.should_quit {
                trace_lifecycle(format!(
                    "run loop={loop_iter} quit_flag_seen {}",
                    trace_state_summary(&self.state)
                ));
                break;
            }

            trace_lifecycle(format!(
                "run loop={loop_iter} render_begin {}",
                trace_state_summary(&self.state)
            ));
            self.render()?;
            trace_lifecycle(format!(
                "run loop={loop_iter} render_end {}",
                trace_state_summary(&self.state)
            ));

            self.drain_generation();
            self.drain_review_assist();
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
            self.drain_file_events()?;
            self.maybe_start_periodic_fetch();
            trace_lifecycle(format!(
                "run loop={loop_iter} drains_end {}",
                trace_state_summary(&self.state)
            ));

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
                || self.state.workflow_job.is_some()
            {
                80
            } else {
                TICK_MS
            };
            trace_lifecycle(format!(
                "run loop={loop_iter} poll_begin poll_ms={poll_ms} {}",
                trace_state_summary(&self.state)
            ));
            if event::poll(Duration::from_millis(poll_ms))? {
                trace_lifecycle(format!("run loop={loop_iter} poll_ready"));
                let input_event = event::read()?;
                trace_lifecycle(format!("run loop={loop_iter} event={input_event:?}"));
                match input_event {
                    Event::Key(k) => self.handle_key(k)?,
                    Event::Mouse(m) => self.handle_mouse(m)?,
                    Event::Resize(width, height) => {
                        trace_lifecycle(format!(
                            "run loop={loop_iter} resize width={width} height={height}"
                        ));
                    }
                    other => {
                        trace_lifecycle(format!("run loop={loop_iter} ignored_event={other:?}"));
                    }
                }
                trace_lifecycle(format!(
                    "run loop={loop_iter} event_handled {}",
                    trace_state_summary(&self.state)
                ));
            } else {
                trace_lifecycle(format!("run loop={loop_iter} poll_timeout"));
            }

            // Dispatch pending IO action.
            if let Some(action) = self.state.pending_action.take() {
                trace_lifecycle(format!(
                    "run loop={loop_iter} dispatch_pending action={action:?}"
                ));
                self.dispatch_pending(action);
                trace_lifecycle(format!(
                    "run loop={loop_iter} dispatch_pending_end {}",
                    trace_state_summary(&self.state)
                ));
            }

            // Expire stale status messages.
            if let Some(ref s) = self.state.status.clone() {
                if (Utc::now() - s.at).num_seconds() >= STATUS_MSG_LIFETIME_SECS {
                    self.state.status = None;
                }
            }
        }
        trace_lifecycle(format!("run exit {}", trace_state_summary(&self.state)));
        Ok(())
    }

    fn dispatch_pending(&mut self, action: PendingAction) {
        match action {
            PendingAction::GenerateMessage => match crate::git::staged_diff() {
                Ok(diff) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        crate::ollama::stream_commit_message(diff, tx);
                    });
                    self.state.start_generation(rx);
                    self.state.set_status("generating\u{2026}", false);
                }
                Err(e) => {
                    self.state.set_status(e.to_string(), true);
                }
            },
            PendingAction::ReviewAssist(node_id) => {
                spawn_review_assist(&mut self.state, node_id);
            }
            PendingAction::Commit => {
                let msg = self.state.commit_message.clone();
                spawn_operation(
                    &mut self.state,
                    "committing",
                    OperationKind::Commit,
                    move || {
                        let out = crate::git::commit(&msg)?;
                        Ok(out.lines().next().unwrap_or("committed").to_owned())
                    },
                );
            }
            PendingAction::Push => spawn_push(&mut self.state),
            PendingAction::Pull => spawn_pull(&mut self.state),
            PendingAction::SaveAuthor { name, email } => {
                match crate::git::set_local_author(&name, &email) {
                    Ok(()) => {
                        self.state.author_has_local_override = true;
                        self.state.modal = Modal::None;
                        self.state.set_status("saved repo author", false);
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("author save failed: {err}"), true),
                }
            }
            PendingAction::ClearAuthor => match crate::git::clear_local_author() {
                Ok(()) => {
                    self.state.author_has_local_override = false;
                    self.state.modal = Modal::None;
                    self.state.set_status("cleared repo author", false);
                }
                Err(err) => self
                    .state
                    .set_status(format!("author clear failed: {err}"), true),
            },
            PendingAction::SaveSubtreeAuthor { path, name, email } => {
                match crate::git::set_subtree_author(&path, &name, &email) {
                    Ok(()) => {
                        self.state.author_has_subtree_rule = true;
                        self.state.modal = Modal::None;
                        self.state.set_status("saved subtree author", false);
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("author save failed: {err}"), true),
                }
            }
            PendingAction::ClearSubtreeAuthor { path } => {
                match crate::git::clear_subtree_author(&path) {
                    Ok(()) => {
                        self.state.author_has_subtree_rule = false;
                        self.state.modal = Modal::None;
                        self.state.set_status("cleared subtree author", false);
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("author clear failed: {err}"), true),
                }
            }
            PendingAction::StageAll => {
                spawn_operation(&mut self.state, "staging", OperationKind::Worktree, || {
                    crate::git::stage_all()?;
                    Ok("staged all".to_string())
                });
            }
            PendingAction::UnstageAll => {
                spawn_operation(
                    &mut self.state,
                    "unstaging",
                    OperationKind::Worktree,
                    || {
                        crate::git::unstage_all()?;
                        Ok("unstaged all".to_string())
                    },
                );
            }
            PendingAction::StagePath(path) => {
                spawn_operation(
                    &mut self.state,
                    "staging",
                    OperationKind::Worktree,
                    move || {
                        crate::git::stage(&path)?;
                        Ok(format!("staged {path}"))
                    },
                );
            }
            PendingAction::UnstagePath(path) => {
                spawn_operation(
                    &mut self.state,
                    "unstaging",
                    OperationKind::Worktree,
                    move || {
                        crate::git::unstage(&path)?;
                        Ok(format!("unstaged {path}"))
                    },
                );
            }
            PendingAction::IgnorePath { path, is_dir } => {
                match crate::git::add_to_gitignore(&path, is_dir) {
                    Ok(status) => {
                        self.state.set_status(status, false);
                        self.start_refresh_with_status(false, false);
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("gitignore update failed: {err}"), true),
                }
            }
            PendingAction::OpenProject => match crate::git::open_project_in_ide() {
                Ok(status) => self.state.set_status(status, false),
                Err(err) => self.state.set_status(format!("open failed: {err}"), true),
            },
            PendingAction::OpenFile(path) => match crate::git::open_file_in_ide(&path) {
                Ok(status) => self.state.set_status(status, false),
                Err(err) => self.state.set_status(format!("open failed: {err}"), true),
            },
            PendingAction::DeleteBranch {
                name,
                delete_local,
                delete_remote,
                force,
            } => {
                self.state.modal = Modal::None;
                spawn_operation(
                    &mut self.state,
                    "deleting branch",
                    OperationKind::Worktree,
                    move || {
                        let mut report = Vec::new();
                        if delete_local {
                            let line = crate::git::delete_local_branch(&name, force)?;
                            report.push(format!("local: {line}"));
                        }
                        if delete_remote {
                            let line = crate::git::delete_remote_branch(&name)?;
                            report.push(format!("remote: {line}"));
                        }
                        Ok(report.join(" | "))
                    },
                );
            }
        }
    }

    fn start_refresh(&mut self, refresh_diff: bool) {
        self.start_refresh_with_status(refresh_diff, true);
    }

    fn start_refresh_with_status(&mut self, refresh_diff: bool, show_status: bool) {
        if let Some(job) = self.state.refresh_job.as_mut() {
            job.refresh_diff |= refresh_diff;
            self.state.refresh_pending = true;
            self.state.refresh_pending_diff |= refresh_diff;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(RefreshMsg::Done(Box::new(build_refresh_snapshot())));
        });
        self.state.refresh_job = Some(RefreshJob {
            rx,
            spinner: 0,
            refresh_diff,
        });
        if show_status {
            self.state.set_status("refreshing\u{2026}", false);
        }
    }

    fn start_fetch(&mut self) {
        if git_job_running(&self.state) {
            return;
        }
        self.last_fetch_started = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || match crate::git::fetch_updates() {
            Ok(s) => {
                let _ = tx.send(FetchMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(FetchMsg::Error(e.to_string()));
            }
        });
        self.state.fetch_job = Some(FetchJob { rx, spinner: 0 });
    }

    fn maybe_start_periodic_fetch(&mut self) {
        if self.last_fetch_started.elapsed() >= Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS)
        {
            self.start_fetch();
        }
    }

    fn start_diff_job(&mut self, force: bool) {
        if self.state.focus == Pane::Main && matches!(self.state.diff_source, DiffSource::Review) {
            return;
        }
        let source = selected_diff_source(&self.state);
        if !force && source == self.state.diff_source {
            return;
        }
        if self
            .state
            .diff_job
            .as_ref()
            .is_some_and(|job| job.source == source)
        {
            return;
        }
        self.state.diff_source = source.clone();
        self.state.diff_offset = 0;
        self.state.diff_text = if matches!(source, DiffSource::None) {
            String::new()
        } else if matches!(source, DiffSource::Branch(_)) {
            "loading log...".to_string()
        } else {
            "loading diff...".to_string()
        };
        self.state.diff_line_count =
            self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
        if matches!(source, DiffSource::None) {
            self.state.diff_job = None;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_source = source.clone();
        std::thread::spawn(move || {
            let text = load_diff_text(&thread_source);
            let _ = tx.send(DiffMsg::Done {
                source: thread_source,
                text,
            });
        });
        self.state.diff_job = Some(DiffJob {
            rx,
            spinner: 0,
            source,
        });
    }

    fn sync_commit_log_to_selection(&mut self) {
        let Some(branch) = selected_commit_ref(&self.state) else {
            return;
        };
        self.start_commit_log_job(branch);
    }

    fn start_commit_log_job(&mut self, branch: String) {
        if self.state.commits_ref.as_deref() == Some(branch.as_str()) {
            return;
        }
        if self
            .state
            .commit_log_job
            .as_ref()
            .is_some_and(|job| job.branch == branch)
        {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let thread_branch = branch.clone();
        std::thread::spawn(move || {
            match crate::git::list_commits_for_ref(&thread_branch, COMMIT_LIST_LIMIT) {
                Ok(commits) => {
                    let _ = tx.send(CommitLogMsg::Done {
                        branch: thread_branch,
                        commits,
                    });
                }
                Err(e) => {
                    let _ = tx.send(CommitLogMsg::Error {
                        branch: thread_branch,
                        message: e.to_string(),
                    });
                }
            }
        });

        self.state.commits_ref = Some(branch.clone());
        self.state.commits.clear();
        self.state.commits_idx = 0;
        self.state.commit_log_job = Some(CommitLogJob {
            rx,
            spinner: 0,
            branch,
        });
    }

    fn sync_release_status_to_branch(&mut self) {
        let Some(branch) = self.state.branch.clone() else {
            self.state.current_branch_releases = Default::default();
            self.state.current_branch_releases_ref = None;
            self.state.release_status_job = None;
            return;
        };
        if !self.state.flow_available() {
            self.state.current_branch_releases = Default::default();
            self.state.current_branch_releases_ref = None;
            self.state.release_status_job = None;
            return;
        }
        if matches!(branch.as_str(), BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST) {
            self.state.current_branch_releases = Default::default();
            self.state.current_branch_releases_ref = Some(branch);
            self.state.release_status_job = None;
            return;
        }
        if self.state.current_branch_releases_ref.as_deref() == Some(branch.as_str()) {
            return;
        }
        if self
            .state
            .release_status_job
            .as_ref()
            .is_some_and(|job| job.branch == branch)
        {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let thread_branch = branch.clone();
        std::thread::spawn(
            move || match crate::git::branch_release_status(&thread_branch) {
                Ok(status) => {
                    let _ = tx.send(ReleaseStatusMsg::Done {
                        branch: thread_branch,
                        status,
                    });
                }
                Err(e) => {
                    let _ = tx.send(ReleaseStatusMsg::Error {
                        branch: thread_branch,
                        message: e.to_string(),
                    });
                }
            },
        );
        self.state.release_status_job = Some(ReleaseStatusJob {
            rx,
            spinner: 0,
            branch,
        });
    }

    fn drain_file_events(&mut self) -> Result<()> {
        let mut should_refresh = false;
        while let Ok(event) = self.file_events.try_recv() {
            match event {
                Ok(event) => {
                    if should_refresh_for_fs_event(&event) {
                        should_refresh = true;
                    }
                }
                Err(err) => {
                    self.state
                        .set_status(format!("file watch failed: {err}"), true);
                }
            }
        }
        if should_refresh {
            self.start_refresh(true);
        }
        Ok(())
    }

    fn apply_refresh_snapshot(
        &mut self,
        snapshot: crate::state::RefreshSnapshot,
        refresh_diff: bool,
    ) {
        if let Some(files) = snapshot.files {
            self.state.files = files;
        }
        if let Some(branches) = snapshot.branches {
            self.state.branches = branches;
        }
        self.state.flow_branches_available = snapshot.flow_branches_available;
        if let Some(shas) = snapshot.unpushed_shas {
            self.state.unpushed_shas = shas;
        }
        let branch_before = self.state.branch.clone();
        self.state.branch = snapshot.branch;
        if self.state.branch != branch_before {
            self.state.current_branch_releases = Default::default();
            self.state.current_branch_releases_ref = None;
            self.state.release_status_job = None;
        }
        let selected_ref = selected_commit_ref(&self.state);
        if let Some(commits) = snapshot.commits {
            if selected_ref.as_deref() == self.state.branch.as_deref() {
                self.state.commits = commits;
                self.state.commits_ref = selected_ref.clone();
            }
        }
        self.state.remote_url = snapshot.remote_url;
        self.state.ahead_behind = snapshot.ahead_behind;
        if let Some(error) = snapshot.errors.into_iter().next() {
            self.state.set_status(error, true);
        }
        self.state.clamp();
        if selected_ref.as_deref() != self.state.commits_ref.as_deref() {
            self.sync_commit_log_to_selection();
        }
        self.sync_release_status_to_branch();
        if refresh_diff {
            self.start_diff_job(true);
        }
    }

    fn drain_refresh_job(&mut self) {
        let mut finished = None;
        if let Some(job) = self.state.refresh_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                let RefreshMsg::Done(snapshot) = msg;
                finished = Some((*snapshot, job.refresh_diff));
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some((snapshot, refresh_diff)) = finished {
            let pending_refresh = self.state.refresh_pending;
            let pending_diff = self.state.refresh_pending_diff;
            self.state.refresh_job = None;
            self.state.refresh_pending = false;
            self.state.refresh_pending_diff = false;
            self.apply_refresh_snapshot(snapshot, refresh_diff);
            if pending_refresh {
                self.start_refresh(pending_diff);
            }
        }
    }

    fn drain_diff_job(&mut self) {
        let mut finished = None;
        if let Some(job) = self.state.diff_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                let DiffMsg::Done { source, text } = msg;
                finished = Some((source, text));
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some((source, text)) = finished {
            self.state.diff_job = None;
            if source == self.state.diff_source {
                self.state.diff_text = text;
                self.state.diff_line_count =
                    self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
            }
        }
    }

    fn drain_release_status_job(&mut self) {
        let mut finished = None;
        if let Some(job) = self.state.release_status_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                finished = Some(msg);
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(msg) = finished {
            self.state.release_status_job = None;
            match msg {
                ReleaseStatusMsg::Done { branch, status } => {
                    if self.state.branch.as_deref() == Some(branch.as_str()) {
                        self.state.current_branch_releases = status;
                        self.state.current_branch_releases_ref = Some(branch);
                    }
                }
                ReleaseStatusMsg::Error { branch, message } => {
                    if self.state.branch.as_deref() == Some(branch.as_str()) {
                        self.state.current_branch_releases = Default::default();
                        self.state.current_branch_releases_ref = None;
                        self.state
                            .set_status(format!("deployment status failed: {message}"), true);
                    }
                }
            }
        }
    }

    fn drain_commit_log_job(&mut self) {
        let mut finished = None;
        if let Some(job) = self.state.commit_log_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                finished = Some(msg);
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(msg) = finished {
            self.state.commit_log_job = None;
            match msg {
                CommitLogMsg::Done { branch, commits } => {
                    if self.state.commits_ref.as_deref() == Some(branch.as_str()) {
                        self.state.commits = commits;
                        self.state.commits_idx = 0;
                        self.state.clamp();
                    }
                }
                CommitLogMsg::Error { branch, message } => {
                    if self.state.commits_ref.as_deref() == Some(branch.as_str()) {
                        self.state.commits.clear();
                        self.state.commits_idx = 0;
                    }
                    self.state
                        .set_status(format!("git log {branch} failed: {message}"), true);
                }
            }
        }
    }

    fn drain_review_job(&mut self) {
        let mut finished: Option<std::result::Result<Box<crate::git::AssistedReview>, String>> =
            None;
        if let Some(job) = self.state.review_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    ReviewMsg::Done(review) => finished = Some(Ok(review)),
                    ReviewMsg::Error(err) => finished = Some(Err(err)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(result) = finished {
            self.state.review_job = None;
            match result {
                Ok(review) => {
                    let report = review.report.clone();
                    self.state.review = Some(*review);
                    self.state.review_collapsed.clear();
                    self.state.review_context_open.clear();
                    if let Some(review) = &self.state.review {
                        for node in &review.nodes {
                            let expandable = !node.body.is_empty()
                                || review.nodes.iter().any(|candidate| {
                                    candidate
                                        .parent
                                        .as_ref()
                                        .is_some_and(|parent| parent == &node.id)
                                });
                            if expandable {
                                self.state.review_collapsed.insert(node.id.clone());
                            }
                        }
                        self.state.review_idx = review
                            .nodes
                            .iter()
                            .position(|node| {
                                node.id == "branch" || node.id.starts_with("branch:file:")
                            })
                            .unwrap_or(0);
                    }
                    self.state.diff_source = DiffSource::Review;
                    self.state.diff_text = report;
                    self.state.diff_offset = 0;
                    self.state.diff_line_count =
                        self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
                    self.state.set_status("review ready", false);
                }
                Err(err) => {
                    self.state.diff_text = format!("error building assisted review: {err}");
                    self.state.diff_line_count =
                        self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
                    self.state.set_status(first_status_line(&err), true);
                }
            }
        }
    }

    fn drain_fetch_job(&mut self) {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.fetch_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    FetchMsg::Done(s) => finished = Some(Ok(s)),
                    FetchMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.fetch_job = None;
            self.state.current_branch_releases_ref = None;
            match res {
                Ok(s) if s != "no remotes configured" => self.state.set_status(s, false),
                Ok(_) => {}
                Err(e) => self.state.set_status(first_status_line(&e), true),
            }
            self.start_refresh_with_status(false, false);
        }
    }

    fn drain_push_job(&mut self) -> Result<()> {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.push_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    PushMsg::Done(s) => finished = Some(Ok(s)),
                    PushMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.push_job = None;
            self.state.modal = Modal::None;
            self.state.current_branch_releases_ref = None;
            match res {
                Ok(s) => self.state.set_status(s, false),
                Err(e) => self.state.set_status(e, true),
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_checkout_job(&mut self) -> Result<()> {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.checkout_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    CheckoutMsg::Done(s) => finished = Some(Ok(s)),
                    CheckoutMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.checkout_job = None;
            self.state.current_branch_releases_ref = None;
            match res {
                Ok(s) => self.state.set_status(s, false),
                Err(e) => self.state.set_status(e, true),
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_operation_job(&mut self) -> Result<()> {
        let mut finished: Option<std::result::Result<String, String>> = None;
        if let Some(job) = self.state.operation_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    OperationMsg::Done(s) => finished = Some(Ok(s)),
                    OperationMsg::Error(s) => finished = Some(Err(s)),
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            let kind = self
                .state
                .operation_job
                .as_ref()
                .map(|job| job.kind)
                .unwrap_or(OperationKind::Worktree);
            self.state.operation_job = None;
            self.state.current_branch_releases_ref = None;
            match res {
                Ok(s) => {
                    self.state.set_status(s, false);
                    if kind == OperationKind::Commit {
                        self.state.modal = Modal::None;
                        self.state.commit_message.clear();
                        if self.state.push_after_commit {
                            self.state.push_after_commit = false;
                            spawn_push(&mut self.state);
                        }
                    }
                }
                Err(e) => {
                    if kind == OperationKind::Commit {
                        self.state.push_after_commit = false;
                    }
                    self.state.set_status(e, true);
                }
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_workflow_job(&mut self) -> Result<()> {
        let mut finished: Option<WorkflowMsg> = None;
        let mut finished_label: Option<String> = None;
        if let Some(job) = self.state.workflow_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    WorkflowMsg::Progress(step) => job.current_step = Some(step),
                    WorkflowMsg::Done(_) | WorkflowMsg::Error(_) => {
                        finished_label = Some(job.label.clone());
                        finished = Some(msg)
                    }
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.workflow_job = None;
            self.state.current_branch_releases_ref = None;
            match res {
                WorkflowMsg::Progress(_) => {}
                WorkflowMsg::Done(s) => {
                    if matches!(
                        finished_label.as_deref(),
                        Some("validate conflict resolution") | Some("abort merge")
                    ) {
                        self.state.conflict_followup = None;
                        self.state.conflicts.clear();
                        self.state.modal = Modal::None;
                    } else if !matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_followup = None;
                    }
                    if matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_log = s.clone();
                    } else {
                        self.state.modal = Modal::None;
                    }
                    self.state.set_status(first_status_line(&s), false);
                }
                WorkflowMsg::Error(e) => {
                    if let Ok(conflicts) = crate::git::conflicted_files() {
                        if !conflicts.is_empty() {
                            self.state.conflicts = conflicts;
                            self.state.conflict_idx = 0;
                            self.state.conflict_log = e.clone();
                            self.state.modal = Modal::Conflict;
                            self.state.set_status("merge conflicts detected", true);
                            self.start_refresh(true);
                            return Ok(());
                        }
                    }
                    if matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_log = e.clone();
                    }
                    if !matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_followup = None;
                    }
                    self.state.set_status(first_status_line(&e), true);
                }
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    fn drain_generation(&mut self) {
        let mut drained: Vec<GenMsg> = Vec::new();
        if let Some(g) = self.state.generation.as_ref() {
            while let Ok(msg) = g.rx.try_recv() {
                drained.push(msg);
            }
        }
        for msg in drained {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(o) => {
                    if let Some(g) = self.state.generation.as_mut() {
                        g.output.push_str(&o);
                    }
                }
                GenMsg::Done(final_msg) => {
                    self.state.commit_message = final_msg;
                    self.state.generation = None;
                    self.state.set_status("message generated", false);
                }
                GenMsg::Error(e) => {
                    self.state.generation = None;
                    self.state.set_status(e, true);
                }
            }
        }
        if let Some(g) = self.state.generation.as_mut() {
            g.spinner = g.spinner.wrapping_add(1);
        }
    }

    fn drain_review_assist(&mut self) {
        let mut drained: Vec<GenMsg> = Vec::new();
        if let Some(job) = self.state.review_assist_job.as_ref() {
            while let Ok(msg) = job.rx.try_recv() {
                drained.push(msg);
            }
        }
        for msg in drained {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(output) => {
                    if let Some(job) = self.state.review_assist_job.as_mut() {
                        job.output.push_str(&output);
                        self.state
                            .review_assists
                            .insert(job.node_id.clone(), job.output.clone());
                    }
                }
                GenMsg::Done(final_msg) => {
                    if let Some(job) = self.state.review_assist_job.take() {
                        self.state.review_assists.insert(job.node_id, final_msg);
                    }
                    self.state.set_status("review explanation ready", false);
                }
                GenMsg::Error(error) => {
                    if let Some(job) = self.state.review_assist_job.take() {
                        self.state
                            .review_assists
                            .insert(job.node_id, format!("ollama error: {error}"));
                    }
                    self.state.set_status(error, true);
                }
            }
        }
        if let Some(job) = self.state.review_assist_job.as_mut() {
            job.spinner = job.spinner.wrapping_add(1);
        }
    }

    fn render(&mut self) -> Result<()> {
        self.state.advance_animation();

        // Compute viewport height before the draw closure so we can update state.
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects_pre = ui::split_layout_with_sizes(
            area,
            self.state.flow_available(),
            self.state.left_column_width,
            self.state.left_panel_heights,
        );
        self.state.diff_viewport_height = rects_pre.main.height.saturating_sub(2);
        self.state.diff_viewport_width = rects_pre.main.width.saturating_sub(2);
        self.clamp_main_scroll_offset();

        let state = &self.state;
        self.terminal.draw(|frame| {
            let area = frame.area();
            let rects = ui::split_layout_with_sizes(
                area,
                state.flow_available(),
                state.left_column_width,
                state.left_panel_heights,
            );
            let focused_pane = state.focus;

            panel::status::render(state, rects.status, frame, focused_pane == Pane::Status);
            panel::environments::render(state, rects.environments, frame);
            panel::files::render(state, rects.files, frame, focused_pane == Pane::Files);
            panel::branches::render(state, rects.branches, frame, focused_pane == Pane::Branches);
            panel::commits::render(state, rects.commits, frame, focused_pane == Pane::Commits);
            panel::main::render(state, rects.main, frame, focused_pane == Pane::Main);

            footer::draw(frame, rects.footer, state);

            match state.modal {
                Modal::None => {}
                Modal::Commit => panel::commit::render(state, area, frame),
                Modal::Push => panel::push::render(state, area, frame),
                Modal::Author => panel::author::render(state, area, frame),
                Modal::Help => panel::help::render(state, area, frame),
                Modal::Flow => panel::flow::render(state, area, frame),
                Modal::Conflict => panel::conflict::render(state, area, frame),
                Modal::DeleteBranch => panel::delete_branch::render(state, area, frame),
            }
        })?;
        Ok(())
    }

    fn clamp_main_scroll_offset(&mut self) {
        self.state.diff_offset = self
            .state
            .diff_offset
            .min(panel::main::max_scroll_offset(&self.state));
    }

    pub fn handle_key(&mut self, k: KeyEvent) -> Result<()> {
        trace_lifecycle(format!(
            "handle_key begin key={k:?} {}",
            trace_state_summary(&self.state)
        ));
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            trace_lifecycle("handle_key quit_request reason=ctrl-c");
            self.state.should_quit = true;
            return Ok(());
        }

        match self.state.modal {
            Modal::Help => {
                panel::help::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Commit => {
                panel::commit::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Push => {
                panel::push::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Author => {
                panel::author::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Flow => {
                panel::flow::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::Conflict => {
                panel::conflict::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::DeleteBranch => {
                panel::delete_branch::handle_key(&mut self.state, k)?;
                return Ok(());
            }
            Modal::None => {}
        }

        match k.code {
            KeyCode::Char('?') => {
                self.state.prev_focus = self.state.focus;
                self.state.modal = Modal::Help;
                return Ok(());
            }
            KeyCode::Char('F') => {
                self.start_refresh(false);
                if self.state.flow_available() {
                    self.state.modal = Modal::Flow;
                }
                return Ok(());
            }
            KeyCode::Char('q') => {
                trace_lifecycle(format!("handle_key quit_request reason=key key={k:?}"));
                self.state.should_quit = true;
                return Ok(());
            }
            KeyCode::Esc => {
                trace_lifecycle(format!("handle_key ignored_top_level_esc key={k:?}"));
                return Ok(());
            }
            KeyCode::Char('1') => {
                self.state.focus = Pane::Status;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('2') => {
                self.state.focus = Pane::Files;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('3') => {
                self.state.focus = Pane::Branches;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('4') => {
                self.state.focus = Pane::Commits;
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('0') => {
                self.state.focus = Pane::Main;
                return Ok(());
            }
            KeyCode::Tab => {
                self.state.focus = next_pane(self.state.focus);
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::BackTab => {
                self.state.focus = prev_pane(self.state.focus);
                self.start_diff_job(false);
                self.sync_commit_log_to_selection();
                return Ok(());
            }
            KeyCode::Char('c') => {
                self.state.open_commit_modal();
                return Ok(());
            }
            KeyCode::Char('a') => {
                open_author_modal(&mut self.state);
                return Ok(());
            }
            KeyCode::Char('p') => {
                spawn_pull(&mut self.state);
                return Ok(());
            }
            KeyCode::Char('f') => {
                self.start_fetch();
                return Ok(());
            }
            KeyCode::Char('P') => {
                if self.state.unpushed_shas.is_empty() {
                    self.state.set_status("nothing to push", false);
                    return Ok(());
                }
                spawn_push(&mut self.state);
                return Ok(());
            }
            KeyCode::Char('R') => {
                spawn_assisted_review(&mut self.state);
                return Ok(());
            }
            _ => {}
        }

        let focus_before = self.state.focus;
        let commit_ref_before = selected_commit_ref(&self.state);

        match focus_before {
            Pane::Status => {}
            Pane::Files => panel::files::handle_key(&mut self.state, k)?,
            Pane::Branches => panel::branches::handle_key(&mut self.state, k)?,
            Pane::Commits => panel::commits::handle_key(&mut self.state, k)?,
            Pane::Main => panel::main::handle_key(&mut self.state, k)?,
        }

        if self.state.pending_action.is_none()
            && (matches!(focus_before, Pane::Files | Pane::Branches | Pane::Commits)
                || matches!(
                    self.state.focus,
                    Pane::Files | Pane::Branches | Pane::Commits
                ))
        {
            self.start_diff_job(false);
        }
        if selected_commit_ref(&self.state) != commit_ref_before {
            self.sync_commit_log_to_selection();
        }
        Ok(())
    }

    fn handle_mouse(&mut self, m: MouseEvent) -> Result<()> {
        let size = self.terminal.size()?;
        let area = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        let rects = ui::split_layout_with_sizes(
            area,
            self.state.flow_available(),
            self.state.left_column_width,
            self.state.left_panel_heights,
        );
        let divider_col = rects.main.x.saturating_sub(1);
        let on_divider = m.row >= rects.status.y
            && m.row < rects.footer.y
            && (m.column == divider_col || m.column == rects.main.x);

        match m.kind {
            MouseEventKind::Down(MouseButton::Left)
                if on_divider && !m.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.state.column_drag_active = true;
                self.state.row_drag_active = None;
                self.state.left_column_width = Some(ui::clamp_left_column_width(
                    rects.status.width.saturating_add(rects.main.width),
                    m.column.saturating_sub(area.x).saturating_add(1),
                ));
                return Ok(());
            }
            MouseEventKind::Drag(MouseButton::Left) if self.state.column_drag_active => {
                self.state.left_column_width = Some(ui::clamp_left_column_width(
                    rects.status.width.saturating_add(rects.main.width),
                    m.column.saturating_sub(area.x).saturating_add(1),
                ));
                return Ok(());
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.state.column_drag_active = false;
                self.state.row_drag_active = None;
                return Ok(());
            }
            _ => {}
        }

        let show_environments = self.state.flow_available();
        match m.kind {
            MouseEventKind::Down(MouseButton::Left)
                if !m.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                if let Some(pair) =
                    mouse::row_divider_pair_at(&rects, show_environments, m.column, m.row)
                {
                    self.state.column_drag_active = false;
                    self.state.row_drag_active = Some(pair);
                    self.state.left_panel_heights = Some(mouse::current_left_panel_heights(&rects));
                    mouse::resize_left_panel_pair(
                        &mut self.state,
                        &rects,
                        pair,
                        m.row,
                        show_environments,
                    );
                    return Ok(());
                }

                if let Some(pane) = mouse::pane_at(&rects, m.column, m.row) {
                    let commit_ref_before = selected_commit_ref(&self.state);
                    self.state.focus = pane;
                    mouse::select_mouse_row(&mut self.state, pane, &rects, m.row);
                    if !matches!(pane, Pane::Main) {
                        self.start_diff_job(false);
                    }
                    if selected_commit_ref(&self.state) != commit_ref_before {
                        self.sync_commit_log_to_selection();
                    }
                    return Ok(());
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(pair) = self.state.row_drag_active {
                    mouse::resize_left_panel_pair(
                        &mut self.state,
                        &rects,
                        pair,
                        m.row,
                        show_environments,
                    );
                    return Ok(());
                }
            }
            _ => {}
        }

        if matches!(
            m.kind,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        ) {
            if let Some(pane @ (Pane::Files | Pane::Branches | Pane::Commits)) =
                mouse::pane_at(&rects, m.column, m.row)
            {
                let focus_before = self.state.focus;
                let commit_ref_before = selected_commit_ref(&self.state);
                let (idx_before, len_before) = mouse_scroll_snapshot(&self.state, pane);
                self.state.focus = pane;
                let changed = mouse::scroll_list(
                    &mut self.state,
                    pane,
                    matches!(m.kind, MouseEventKind::ScrollDown),
                    3,
                );
                let (idx_after, len_after) = mouse_scroll_snapshot(&self.state, pane);
                trace_scroll(format!(
                    "mouse pane={pane:?} dir={} amount=3 idx_before={idx_before} idx_after={idx_after} len_before={len_before} len_after={len_after} changed={changed} focus_before={focus_before:?}",
                    if matches!(m.kind, MouseEventKind::ScrollDown) {
                        "down"
                    } else {
                        "up"
                    }
                ));
                if changed || focus_before != pane {
                    self.start_diff_job(false);
                }
                if selected_commit_ref(&self.state) != commit_ref_before {
                    self.sync_commit_log_to_selection();
                }
                return Ok(());
            }
        }

        let in_main = m.column >= rects.main.x
            && m.column < rects.main.x + rects.main.width
            && m.row >= rects.main.y
            && m.row < rects.main.y + rects.main.height;
        if !in_main {
            return Ok(());
        }
        let offset_before = self.state.diff_offset;
        let max_before = panel::main::max_scroll_offset(&self.state);
        match m.kind {
            MouseEventKind::ScrollDown => {
                panel::main::scroll(&mut self.state, true, 3);
            }
            MouseEventKind::ScrollUp => {
                panel::main::scroll(&mut self.state, false, 3);
            }
            _ => {}
        }
        if matches!(
            m.kind,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        ) {
            trace_scroll(format!(
                "mouse pane=Main dir={} amount=3 offset_before={offset_before} offset_after={} max_before={max_before} max_after={} viewport={} width={} source={:?} text_lines={} visual_lines={} diff_line_count={}",
                if matches!(m.kind, MouseEventKind::ScrollDown) {
                    "down"
                } else {
                    "up"
                },
                self.state.diff_offset,
                panel::main::max_scroll_offset(&self.state),
                self.state.diff_viewport_height,
                self.state.diff_viewport_width,
                self.state.diff_source,
                self.state.diff_text.lines().count(),
                panel::main::rendered_line_count(&self.state),
                self.state.diff_line_count,
            ));
        }
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        trace_lifecycle(format!(
            "app_drop begin {}",
            trace_state_summary(&self.state)
        ));
        restore_terminal(self.terminal.backend_mut());
        trace_lifecycle("app_drop end");
    }
}
