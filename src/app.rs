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
    layout::Rect,
};
use std::{
    io::{Stdout, Write},
    sync::mpsc::Receiver,
    thread::JoinHandle,
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
mod header;
mod input;
mod mouse;
mod refresh;
mod review_assist;
mod spawn;
mod workflow;

pub(crate) use spawn::{checkout_branch_async, checkout_remote_branch_async};
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

fn join_worker(handle: Option<JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

/// Headless app backed by a generic [`Backend`]; used by tests and the harness.
pub struct HeadlessApp<B: Backend> {
    pub state: AppState,
    pub terminal: Terminal<B>,
}

fn first_status_line(s: &str) -> String {
    s.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(s)
        .chars()
        .take(120)
        .collect()
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

            header::draw(frame, rects.header, state);
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
                Modal::StageAllBeforeCommit => panel::stage_all::render(state, area, frame),
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

    fn dispatch_pending(&mut self, action: PendingAction) {
        match action {
            PendingAction::GenerateMessage => match crate::git::staged_diff() {
                Ok(diff) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let handle = std::thread::spawn(move || {
                        crate::ollama::stream_commit_message(diff, tx);
                    });
                    self.state.start_generation(rx, handle);
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
            PendingAction::StageAllAndCommit => {
                spawn_operation(
                    &mut self.state,
                    "staging",
                    OperationKind::StageAllAndCommit,
                    || {
                        crate::git::stage_all()?;
                        Ok("staged all".to_string())
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
        let handle = std::thread::spawn(move || {
            let _ = tx.send(RefreshMsg::Done(Box::new(build_refresh_snapshot())));
        });
        self.state.refresh_job = Some(RefreshJob {
            rx,
            handle: Some(handle),
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
        let handle = std::thread::spawn(move || match crate::git::fetch_updates() {
            Ok(s) => {
                let _ = tx.send(FetchMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(FetchMsg::Error(e.to_string()));
            }
        });
        self.state.fetch_job = Some(FetchJob {
            rx,
            handle: Some(handle),
            spinner: 0,
        });
    }

    fn maybe_start_periodic_fetch(&mut self) {
        if self.last_fetch_started.elapsed() >= Duration::from_secs(BACKGROUND_FETCH_INTERVAL_SECS)
        {
            self.start_fetch();
        }
    }

    fn defer_diff_job(&mut self) {
        if let Some(mut job) = self.state.diff_job.take() {
            self.state.defer_thread_join(job.handle.take());
        }
    }

    fn defer_release_status_job(&mut self) {
        if let Some(mut job) = self.state.release_status_job.take() {
            self.state.defer_thread_join(job.handle.take());
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
            self.defer_diff_job();
            return;
        }
        // Cap in-flight diff workers to one. When the running job finishes,
        // drain_diff_job re-triggers for the latest selection. Without this
        // bound, fast scrolling spawns one OS thread + git subprocess per key
        // press; if scrolling outpaces git show, threads pile up and an
        // eventual std::thread::spawn failure aborts the process.
        if self.state.diff_job.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_source = source.clone();
        let spawn_result = std::thread::Builder::new()
            .name("lg-diff".into())
            .spawn(move || {
                let text = load_diff_text(&thread_source);
                let _ = tx.send(DiffMsg::Done {
                    source: thread_source,
                    text,
                });
            });
        match spawn_result {
            Ok(handle) => {
                self.state.diff_job = Some(DiffJob {
                    rx,
                    handle: Some(handle),
                    spinner: 0,
                    source,
                });
            }
            Err(err) => {
                self.state
                    .set_status(format!("diff worker spawn failed: {err}"), true);
            }
        }
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
        let handle = std::thread::spawn(move || {
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
            handle: Some(handle),
            spinner: 0,
            branch,
        });
    }

    fn sync_release_status_to_branch(&mut self) {
        let Some(branch) = self.state.branch.clone() else {
            self.state.current_branch_releases = Default::default();
            self.state.current_branch_releases_ref = None;
            self.defer_release_status_job();
            return;
        };
        if !self.state.flow_available() {
            self.state.current_branch_releases = Default::default();
            self.state.current_branch_releases_ref = None;
            self.defer_release_status_job();
            return;
        }
        if matches!(branch.as_str(), BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST) {
            self.state.current_branch_releases = Default::default();
            self.state.current_branch_releases_ref = Some(branch);
            self.defer_release_status_job();
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
        let handle =
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
            handle: Some(handle),
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
        self.state.repo_root = snapshot.repo_root;
        if let Some(files) = snapshot.files {
            self.state.files = files;
        }
        if let Some(branches) = snapshot.branches {
            self.state.branches = branches;
        }
        if let Some(branches) = snapshot.remote_branches {
            self.state.remote_branches = branches;
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
            self.defer_release_status_job();
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
        let mut handle = None;
        if let Some(job) = self.state.refresh_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                let RefreshMsg::Done(snapshot) = msg;
                finished = Some((*snapshot, job.refresh_diff));
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some((snapshot, refresh_diff)) = finished {
            let pending_refresh = self.state.refresh_pending;
            let pending_diff = self.state.refresh_pending_diff;
            self.state.refresh_job = None;
            join_worker(handle);
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
        let mut handle = None;
        if let Some(job) = self.state.diff_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                let DiffMsg::Done { source, text } = msg;
                finished = Some((source, text));
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some((source, text)) = finished {
            self.state.diff_job = None;
            join_worker(handle);
            if source == self.state.diff_source {
                self.state.diff_text = text;
                self.state.diff_line_count =
                    self.state.diff_text.lines().count().min(u16::MAX as usize) as u16;
            } else {
                // Worker finished a stale selection. Kick off the right one.
                self.start_diff_job(true);
            }
        }
    }

    fn drain_release_status_job(&mut self) {
        let mut finished = None;
        let mut handle = None;
        if let Some(job) = self.state.release_status_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                finished = Some(msg);
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(msg) = finished {
            self.state.release_status_job = None;
            join_worker(handle);
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
        let mut handle = None;
        if let Some(job) = self.state.commit_log_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                finished = Some(msg);
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(msg) = finished {
            self.state.commit_log_job = None;
            join_worker(handle);
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
        let mut handle = None;
        if let Some(job) = self.state.review_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    ReviewMsg::Done(review) => finished = Some(Ok(review)),
                    ReviewMsg::Error(err) => finished = Some(Err(err)),
                }
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(result) = finished {
            self.state.review_job = None;
            join_worker(handle);
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
        let mut handle = None;
        if let Some(job) = self.state.fetch_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    FetchMsg::Done(s) => finished = Some(Ok(s)),
                    FetchMsg::Error(s) => finished = Some(Err(s)),
                }
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.fetch_job = None;
            join_worker(handle);
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
        let mut handle = None;
        if let Some(job) = self.state.push_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    PushMsg::Done(s) => finished = Some(Ok(s)),
                    PushMsg::Error(s) => finished = Some(Err(s)),
                }
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.push_job = None;
            join_worker(handle);
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
        let mut handle = None;
        if let Some(job) = self.state.checkout_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    CheckoutMsg::Done(s) => finished = Some(Ok(s)),
                    CheckoutMsg::Error(s) => finished = Some(Err(s)),
                }
                handle = job.handle.take();
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.checkout_job = None;
            join_worker(handle);
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
        let mut handle = None;
        if let Some(job) = self.state.operation_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    OperationMsg::Done(s) => finished = Some(Ok(s)),
                    OperationMsg::Error(s) => finished = Some(Err(s)),
                }
                handle = job.handle.take();
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
            join_worker(handle);
            self.state.current_branch_releases_ref = None;
            match res {
                Ok(s) => {
                    self.state.set_status(s, false);
                    if kind == OperationKind::Commit {
                        self.state.modal = Modal::None;
                        self.state.commit_message.clear();
                        self.state.commit_cursor = 0;
                        if self.state.push_after_commit {
                            self.state.push_after_commit = false;
                            spawn_push(&mut self.state);
                        }
                    } else if kind == OperationKind::StageAllAndCommit {
                        self.state.open_commit_modal();
                    }
                }
                Err(e) => {
                    if matches!(
                        kind,
                        OperationKind::Commit | OperationKind::StageAllAndCommit
                    ) {
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
        let mut handle = None;
        if let Some(job) = self.state.workflow_job.as_mut() {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    WorkflowMsg::Progress(step) => job.current_step = Some(step),
                    WorkflowMsg::Done(_) | WorkflowMsg::Error(_) => {
                        finished_label = Some(job.label.clone());
                        finished = Some(msg)
                    }
                }
                if finished.is_some() {
                    handle = job.handle.take();
                }
            }
            job.spinner = job.spinner.wrapping_add(1);
        }
        if let Some(res) = finished {
            self.state.workflow_job = None;
            join_worker(handle);
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
        let mut handle = None;
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
                    if let Some(g) = self.state.generation.as_mut() {
                        handle = g.handle.take();
                    }
                    self.state.commit_message = final_msg;
                    self.state.commit_cursor = self.state.commit_message.chars().count();
                    self.state.generation = None;
                    self.state.set_status("message generated", false);
                }
                GenMsg::Error(e) => {
                    if let Some(g) = self.state.generation.as_mut() {
                        handle = g.handle.take();
                    }
                    self.state.generation = None;
                    self.state.set_status(e, true);
                }
            }
        }
        join_worker(handle);
        if let Some(g) = self.state.generation.as_mut() {
            g.spinner = g.spinner.wrapping_add(1);
        }
    }

    fn drain_review_assist(&mut self) {
        let mut drained: Vec<GenMsg> = Vec::new();
        let mut handle = None;
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
                    if let Some(job) = self.state.review_assist_job.as_mut() {
                        handle = job.handle.take();
                    }
                    if let Some(job) = self.state.review_assist_job.take() {
                        self.state.review_assists.insert(job.node_id, final_msg);
                    }
                    self.state.set_status("review explanation ready", false);
                }
                GenMsg::Error(error) => {
                    if let Some(job) = self.state.review_assist_job.as_mut() {
                        handle = job.handle.take();
                    }
                    if let Some(job) = self.state.review_assist_job.take() {
                        self.state
                            .review_assists
                            .insert(job.node_id, format!("ollama error: {error}"));
                    }
                    self.state.set_status(error, true);
                }
            }
        }
        join_worker(handle);
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

            header::draw(frame, rects.header, state);
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
                Modal::StageAllBeforeCommit => panel::stage_all::render(state, area, frame),
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

    fn join_background_jobs(&mut self) {
        let mut handles = Vec::new();
        handles.extend(self.state.take_deferred_threads());

        if let Some(job) = self.state.generation.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.push_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.checkout_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.operation_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.fetch_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.refresh_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.release_status_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.commit_log_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.diff_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.review_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.review_assist_job.as_mut() {
            handles.extend(job.handle.take());
        }
        if let Some(job) = self.state.workflow_job.as_mut() {
            handles.extend(job.handle.take());
        }

        for handle in handles {
            join_worker(Some(handle));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        restore_terminal(self.terminal.backend_mut());
        self.join_background_jobs();
    }
}
