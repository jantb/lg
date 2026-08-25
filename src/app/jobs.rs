use anyhow::Result;
use std::collections::HashSet;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{
    config::{BACKGROUND_FETCH_INTERVAL_SECS, COMMIT_LIST_LIMIT, is_protected_branch_name},
    state::{
        BackgroundJob, CheckoutMsg, CommitLogJob, CommitLogMsg, DiffJob, DiffMsg, DiffSource,
        FetchJob, FetchMsg, GenMsg, Modal, OperationKind, OperationMsg, Pane, PushMsg, RefreshJob,
        RefreshMsg, ReleaseStatusJob, ReleaseStatusMsg, ReviewAssistJob, ReviewFlagMsg, ReviewMsg,
        SettingsSuggestMsg, WorkflowMsg,
    },
};

use super::{
    App, build_refresh_snapshot, git_job_running, load_diff_text, selected_commit_ref,
    selected_diff_source, should_refresh_for_fs_event, spawn_push,
};

/// Take a finished single-shot job out of `slot`, with the last message it sent.
/// `None` while it is still running, in which case its spinner advances.
fn take_finished<J: BackgroundJob>(slot: &mut Option<J>) -> Option<(J, J::Msg)> {
    let job = slot.as_mut()?;
    let mut finished = None;
    while let Ok(msg) = job.rx().try_recv() {
        finished = Some(msg);
    }
    match finished {
        Some(msg) => Some((slot.take()?, msg)),
        None => {
            tick_spinner(slot);
            None
        }
    }
}

/// Everything a streaming job has sent since the last check. The job stays put:
/// it reports many times before it is done.
fn drain_messages<J: BackgroundJob>(slot: &Option<J>) -> Vec<J::Msg> {
    let Some(job) = slot.as_ref() else {
        return Vec::new();
    };
    let mut drained = Vec::new();
    while let Ok(msg) = job.rx().try_recv() {
        drained.push(msg);
    }
    drained
}

fn tick_spinner<J: BackgroundJob>(slot: &mut Option<J>) {
    if let Some(job) = slot.as_mut() {
        let spinner = job.spinner_mut();
        *spinner = spinner.wrapping_add(1);
    }
}

/// Stream an LLM answer for a review node into the pane. Review explanations and
/// PR text differ only in what they are called when they finish. Returns the
/// status to show, if the stream reached an end.
fn drain_review_stream(
    slot: &mut Option<ReviewAssistJob>,
    assists: &mut std::collections::HashMap<String, String>,
    ready: &'static str,
) -> Option<(String, bool)> {
    let mut status = None;
    let mut handle = None;
    for msg in drain_messages(slot) {
        match msg {
            GenMsg::Thinking(_) => {}
            GenMsg::Output(output) => {
                if let Some(job) = slot.as_mut() {
                    job.output.push_str(&output);
                    assists.insert(job.node_id.clone(), job.output.clone());
                }
            }
            GenMsg::Reset => {
                if let Some(job) = slot.as_mut() {
                    job.output.clear();
                    assists.insert(job.node_id.clone(), String::new());
                }
            }
            GenMsg::Done(final_msg) => {
                if let Some(mut job) = slot.take() {
                    handle = job.handle.take();
                    assists.insert(job.node_id, final_msg);
                }
                status = Some((ready.to_string(), false));
            }
            GenMsg::Error(error) => {
                if let Some(mut job) = slot.take() {
                    handle = job.handle.take();
                    assists.insert(job.node_id, format!("llm error: {error}"));
                }
                status = Some((error, true));
            }
        }
    }
    join_worker(handle);
    tick_spinner(slot);
    status
}

fn join_worker(handle: Option<JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
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

fn open_conflict_modal_if_needed(state: &mut crate::state::AppState, log: String) -> bool {
    let conflicts = crate::git::conflicted_files().unwrap_or_default();
    if conflicts.is_empty() {
        return false;
    }
    state.conflicts = conflicts;
    state.conflict_idx = 0;
    state.conflict_log = log;
    state.modal = Modal::Conflict;
    state.set_status("conflicts detected", true);
    true
}

impl App {
    /// Point lg at `dir`: git commands, the file watcher and every per-checkout
    /// piece of state follow it. Used for nested repositories and worktrees
    /// alike, so switching between them is one code path.
    pub(super) fn switch_to_repository(&mut self, dir: &std::path::Path, label: &str) {
        crate::git::set_active_repo(dir);

        // A failed watcher only costs automatic refreshes, so the switch still
        // goes through — but say so, because staleness is otherwise silent.
        let watch_error = match super::refresh::watch_repo(dir) {
            Ok((watcher, events)) => {
                self.file_watcher = watcher;
                self.file_events = events;
                None
            }
            Err(err) => Some(err.to_string()),
        };

        self.state.repo_root = Some(dir.to_string_lossy().into_owned());
        self.clear_release_status(None);
        self.state.nested_repo_detail_path = None;
        self.state.nested_repo_branches.clear();
        self.state.nested_repo_remote_branches.clear();
        match watch_error {
            Some(err) => self
                .state
                .set_status(format!("selected {label}; file watch failed: {err}"), true),
            None => self.state.set_status(format!("selected {label}"), false),
        }
        self.start_refresh(true);
    }

    pub(super) fn start_refresh(&mut self, refresh_diff: bool) {
        self.start_refresh_with_status(refresh_diff, true);
    }

    pub(super) fn start_refresh_with_status(&mut self, refresh_diff: bool, show_status: bool) {
        if let Some(job) = self.state.refresh_job.as_mut() {
            job.refresh_diff |= refresh_diff;
            self.state.refresh_pending = true;
            self.state.refresh_pending_diff |= refresh_diff;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let workspace_root = self.state.workspace_root.clone();
        let handle = crate::git::spawn_pinned(move || {
            let _ = tx.send(RefreshMsg::Done(Box::new(build_refresh_snapshot(
                workspace_root,
            ))));
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

    pub(super) fn start_fetch(&mut self) {
        if git_job_running(&self.state) {
            return;
        }
        self.last_fetch_started = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = crate::git::spawn_pinned(move || match crate::git::fetch_updates() {
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

    pub(super) fn maybe_start_periodic_fetch(&mut self) {
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

    pub(super) fn defer_release_status_job(&mut self) {
        if let Some(mut job) = self.state.release_status_job.take() {
            self.state.defer_thread_join(job.handle.take());
        }
    }

    pub(super) fn start_diff_job(&mut self, force: bool) {
        if self.state.focus == Pane::Main && matches!(self.state.diff_source, DiffSource::Review) {
            return;
        }
        let source = selected_diff_source(&self.state);
        let same_source = source == self.state.diff_source;
        if !force && same_source {
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
        // Reloading what is already on screen leaves it there until the new
        // text arrives. The pane holds the last answer for this selection, and
        // blanking it would flash a placeholder and throw the scroll position
        // away on every refresh — which is once per file event, and a busy
        // worktree produces those all day.
        if !same_source {
            self.state.diff_offset = 0;
            self.state
                .set_diff_text(if matches!(source, DiffSource::None) {
                    String::new()
                } else if matches!(source, DiffSource::Branch(_)) {
                    "loading log...".to_string()
                } else {
                    "loading diff...".to_string()
                });
        }
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

    pub(super) fn sync_commit_log_to_selection(&mut self) {
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
        let handle = crate::git::spawn_pinned(move || {
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

    /// Drop any deployment status and stop the job that was producing it.
    /// `checked` is the branch the now-empty status stands for, so a later sync
    /// can tell "nothing to report for this branch" from "not looked at yet".
    fn clear_release_status(&mut self, checked: Option<String>) {
        self.state.current_branch_releases = Default::default();
        self.state.current_branch_releases_ref = checked;
        self.defer_release_status_job();
    }

    fn sync_release_status_to_branch(&mut self) {
        let Some(branch) = self.state.branch.clone() else {
            self.clear_release_status(None);
            return;
        };
        if !self.state.flow_available() {
            self.clear_release_status(None);
            return;
        }
        if is_protected_branch_name(&branch) {
            self.clear_release_status(Some(branch));
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

        self.state.current_branch_releases = Default::default();
        self.state.current_branch_releases_ref = None;
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_branch = branch.clone();
        let handle = crate::git::spawn_pinned(move || {
            match crate::git::branch_release_status(&thread_branch) {
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
            }
        });
        self.state.release_status_job = Some(ReleaseStatusJob {
            rx,
            handle: Some(handle),
            spinner: 0,
            branch,
        });
    }

    pub(super) fn drain_file_events(&mut self) -> Result<()> {
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
        let repo_before = self.state.repo_root.clone();
        self.state.repo_root = snapshot.repo_root;
        let repo_changed = self.state.repo_root != repo_before;
        self.state.workspace_root = snapshot.workspace_root;
        if let Some(files) = snapshot.files {
            self.state.files = files;
        }
        if let Some(branches) = snapshot.branches {
            self.state.branches = branches;
        }
        if let Some(branches) = snapshot.remote_branches {
            self.state.remote_branches = branches;
        }
        if let Some(repositories) = snapshot.nested_repositories {
            self.state.nested_repositories = repositories;
        }
        if let Some(worktrees) = snapshot.worktrees {
            self.state.worktrees = worktrees;
        }
        self.state.release_branches = snapshot.release_branches;
        if let Some(shas) = snapshot.unpushed_shas {
            self.state.unpushed_shas = shas;
        }
        let branch_before = self.state.branch.clone();
        self.state.branch = snapshot.branch;
        if self.state.branch != branch_before || repo_changed {
            self.clear_release_status(None);
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

    pub(super) fn drain_refresh_job(&mut self) {
        let Some((mut job, RefreshMsg::Done(snapshot))) =
            take_finished(&mut self.state.refresh_job)
        else {
            return;
        };
        let pending_refresh = self.state.refresh_pending;
        let pending_diff = self.state.refresh_pending_diff;
        join_worker(job.handle.take());
        self.state.refresh_pending = false;
        self.state.refresh_pending_diff = false;
        self.apply_refresh_snapshot(*snapshot, job.refresh_diff);
        if pending_refresh {
            self.start_refresh(pending_diff);
        }
    }

    pub(super) fn drain_diff_job(&mut self) {
        let Some((mut job, DiffMsg::Done { source, text })) =
            take_finished(&mut self.state.diff_job)
        else {
            return;
        };
        join_worker(job.handle.take());
        if source == self.state.diff_source {
            self.state.set_diff_text(text);
        } else {
            // Worker finished a stale selection. Kick off the right one.
            self.start_diff_job(true);
        }
    }

    pub(super) fn drain_release_status_job(&mut self) {
        let Some((mut job, msg)) = take_finished(&mut self.state.release_status_job) else {
            return;
        };
        join_worker(job.handle.take());
        {
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

    /// Applies derived conventions only to rows the user has not touched, and
    /// only while the settings modal is still open — a suggestion must never
    /// overwrite something typed in the meantime.
    pub(super) fn drain_settings_suggest_job(&mut self) {
        let Some((mut job, msg)) = take_finished(&mut self.state.settings_suggest_job) else {
            return;
        };
        join_worker(job.handle.take());
        match msg {
            SettingsSuggestMsg::Done { language, shapes } => {
                if self.state.modal != Modal::Model || crate::settings::is_configured() {
                    return;
                }
                let mut applied = Vec::new();
                if let Some(language) = language.filter(|language| !language.trim().is_empty()) {
                    self.state.settings_pr_language_input = language;
                    self.state.settings_derived_language = true;
                    applied.push("language".to_string());
                }
                let shapes: Vec<String> = shapes
                    .into_iter()
                    .filter(|shape| !shape.trim().is_empty())
                    .collect();
                if !shapes.is_empty() {
                    // The first shape is the model's best reading; the rest stay
                    // available as the row's choice list to step through.
                    if self.state.settings_comment_style_input.trim().is_empty() {
                        self.state.settings_comment_style_input = shapes[0].clone();
                    }
                    self.state.settings_comment_style_choices = shapes.clone();
                    self.state.settings_derived_shape = true;
                    applied.push(format!("{} message shapes", shapes.len()));
                }
                if applied.is_empty() {
                    self.state
                        .set_status("could not derive conventions from history", false);
                } else {
                    self.state.set_status(
                        format!(
                            "suggested {} from history; Up/Down to compare, Enter to save",
                            applied.join(" and ")
                        ),
                        false,
                    );
                }
            }
            SettingsSuggestMsg::Error(message) => {
                self.state
                    .set_status(format!("convention scan failed: {message}"), false);
            }
        }
    }

    pub(super) fn drain_commit_log_job(&mut self) {
        let Some((mut job, msg)) = take_finished(&mut self.state.commit_log_job) else {
            return;
        };
        join_worker(job.handle.take());
        {
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

    pub(super) fn drain_review_job(&mut self) {
        let Some((mut job, msg)) = take_finished(&mut self.state.review_job) else {
            return;
        };
        join_worker(job.handle.take());
        {
            match msg {
                ReviewMsg::Done(review) => {
                    let report = review.report.clone();
                    self.state.review = Some(*review);
                    self.state.review_collapsed.clear();
                    self.state.review_context_open.clear();
                    self.state.review_context_restore_collapsed.clear();
                    if let Some(review) = &self.state.review {
                        self.state.review_collapsed = default_review_collapsed_nodes(review);
                        self.state.review_idx = initial_review_index(review);
                    }
                    self.state.diff_source = DiffSource::Review;
                    self.state.set_diff_text(report);
                    self.state.diff_offset = 0;
                    self.state.set_status("review ready", false);
                }
                ReviewMsg::Error(err) => {
                    self.state
                        .set_diff_text(format!("error building assisted review: {err}"));
                    self.state.set_status(first_status_line(&err), true);
                }
            }
        }
    }

    pub(super) fn drain_review_flag_job(&mut self) {
        let mut handle = None;
        for msg in drain_messages(&self.state.review_flag_job) {
            match msg {
                ReviewFlagMsg::Started { path, index, total } => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        job.active_path = Some(path.clone());
                    }
                    let reveal_ids = self
                        .state
                        .review
                        .as_ref()
                        .map(|review| review_path_ancestor_ids(review, &path))
                        .unwrap_or_default();
                    for id in reveal_ids {
                        self.state.review_collapsed.remove(&id);
                    }
                    self.state.review_flag_active_path = Some(path.clone());
                    self.state
                        .set_status(format!("analyzing style {index}/{total}: {path}"), false);
                }
                ReviewFlagMsg::Done { path, finding } => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        job.completed = job.completed.saturating_add(1);
                    }
                    if self.state.review_flag_active_path.as_deref() == Some(path.as_str()) {
                        self.state.review_flag_active_path = None;
                    }
                    let severity = finding.severity;
                    let is_error = !matches!(severity, crate::state::ReviewStyleSeverity::Ok);
                    self.state
                        .review_style_findings
                        .insert(path.clone(), finding);
                    self.state.set_status(
                        format!("style {}: {path}", severity.label().to_ascii_lowercase()),
                        is_error,
                    );
                }
                ReviewFlagMsg::Error { path, message } => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        job.completed = job.completed.saturating_add(1);
                    }
                    if self.state.review_flag_active_path.as_deref() == Some(path.as_str()) {
                        self.state.review_flag_active_path = None;
                    }
                    self.state
                        .set_status(format!("style check failed for {path}: {message}"), true);
                }
                ReviewFlagMsg::Finished => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        handle = job.handle.take();
                    }
                    self.state.review_flag_job = None;
                    self.state.review_flag_active_path = None;
                    let warn_count = self
                        .state
                        .review_style_findings
                        .values()
                        .filter(|finding| {
                            matches!(finding.severity, crate::state::ReviewStyleSeverity::Warn)
                        })
                        .count();
                    let fail_count = self
                        .state
                        .review_style_findings
                        .values()
                        .filter(|finding| {
                            matches!(finding.severity, crate::state::ReviewStyleSeverity::Fail)
                        })
                        .count();
                    self.state.set_status(
                        format!("style pass complete: {warn_count} warn, {fail_count} fail"),
                        fail_count > 0,
                    );
                }
            }
        }
        join_worker(handle);
        tick_spinner(&mut self.state.review_flag_job);
    }

    pub(super) fn drain_fetch_job(&mut self) {
        let Some((mut job, msg)) = take_finished(&mut self.state.fetch_job) else {
            return;
        };
        join_worker(job.handle.take());
        self.state.current_branch_releases_ref = None;
        match msg {
            FetchMsg::Done(s) if s != "no remotes configured" => self.state.set_status(s, false),
            FetchMsg::Done(_) => {}
            FetchMsg::Error(e) => self.state.set_status(first_status_line(&e), true),
        }
        self.start_refresh_with_status(false, false);
    }

    pub(super) fn drain_push_job(&mut self) -> Result<()> {
        let Some((mut job, msg)) = take_finished(&mut self.state.push_job) else {
            return Ok(());
        };
        join_worker(job.handle.take());
        self.state.modal = Modal::None;
        self.state.current_branch_releases_ref = None;
        match msg {
            PushMsg::Done(s) => self.state.set_status(s, false),
            PushMsg::Error(e) => self.state.set_status(e, true),
        }
        crate::panel::environments::reload_nested_repo_detail(&mut self.state);
        self.start_refresh(true);
        Ok(())
    }

    pub(super) fn drain_checkout_job(&mut self) -> Result<()> {
        let Some((mut job, msg)) = take_finished(&mut self.state.checkout_job) else {
            return Ok(());
        };
        join_worker(job.handle.take());
        self.state.current_branch_releases_ref = None;
        match msg {
            CheckoutMsg::Done(s) => self.state.set_status(s, false),
            CheckoutMsg::Error(e) => {
                if !open_conflict_modal_if_needed(&mut self.state, e.clone()) {
                    self.state.set_status(e, true);
                }
            }
        }
        self.start_refresh(true);
        Ok(())
    }

    pub(super) fn drain_operation_job(&mut self) -> Result<()> {
        // Progress reports arrive before the one message that ends the job, so
        // this drains rather than taking whichever arrived last.
        let mut finished = None;
        for msg in drain_messages(&self.state.operation_job) {
            match msg {
                OperationMsg::Progress(step) => {
                    if let Some(job) = self.state.operation_job.as_mut() {
                        job.step = Some(step);
                    }
                }
                ended => finished = Some(ended),
            }
        }
        let Some(msg) = finished else {
            tick_spinner(&mut self.state.operation_job);
            return Ok(());
        };
        let Some(mut job) = self.state.operation_job.take() else {
            return Ok(());
        };
        let kind = job.kind;
        join_worker(job.handle.take());
        self.state.current_branch_releases_ref = None;
        match msg {
            OperationMsg::Done(s) => {
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
                } else if kind == OperationKind::MergeUpstream {
                    self.state.modal = Modal::None;
                }
            }
            OperationMsg::Error(e) => {
                if matches!(
                    kind,
                    OperationKind::Commit | OperationKind::StageAllAndCommit
                ) {
                    self.state.push_after_commit = false;
                }
                if !open_conflict_modal_if_needed(&mut self.state, e.clone()) {
                    self.state.set_status(e, true);
                }
            }
            // Filtered out above; only Done and Error reach here.
            OperationMsg::Progress(_) => {}
        }
        self.start_refresh(true);
        Ok(())
    }

    pub(super) fn drain_workflow_job(&mut self) -> Result<()> {
        // Progress reports arrive before the one message that ends the job, so
        // this drains rather than taking the last message.
        let mut finished = None;
        for msg in drain_messages(&self.state.workflow_job) {
            match msg {
                WorkflowMsg::Progress(step) => {
                    if let Some(job) = self.state.workflow_job.as_mut() {
                        job.current_step = Some(step);
                    }
                }
                done_or_error => finished = Some(done_or_error),
            }
        }
        let Some(res) = finished else {
            tick_spinner(&mut self.state.workflow_job);
            return Ok(());
        };
        let finished_label = self
            .state
            .workflow_job
            .as_ref()
            .map(|job| job.label.clone());
        if let Some(mut job) = self.state.workflow_job.take() {
            join_worker(job.handle.take());
        }
        {
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
                    let conflicts = crate::git::conflicted_files().unwrap_or_default();
                    self.state.conflicts = conflicts;
                    self.state.conflict_idx = 0;
                    if !self.state.conflicts.is_empty() {
                        self.state.conflict_log = e.clone();
                        self.state.modal = Modal::Conflict;
                        self.state.set_status("merge conflicts detected", true);
                        self.start_refresh(true);
                        return Ok(());
                    }
                    if matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict_log = e.clone();
                        self.state.modal = Modal::None;
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

    pub(super) fn drain_generation(&mut self) {
        let mut handle = None;
        for msg in drain_messages(&self.state.generation) {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(o) => {
                    if let Some(g) = self.state.generation.as_mut() {
                        g.output.push_str(&o);
                    }
                }
                GenMsg::Reset => {
                    if let Some(g) = self.state.generation.as_mut() {
                        g.output.clear();
                    }
                }
                GenMsg::Done(final_msg) => {
                    if let Some(mut g) = self.state.generation.take() {
                        handle = g.handle.take();
                    }
                    self.state.commit_message = final_msg;
                    self.state.commit_cursor = self.state.commit_message.chars().count();
                    self.state.set_status("message generated", false);
                }
                GenMsg::Error(e) => {
                    if let Some(mut g) = self.state.generation.take() {
                        handle = g.handle.take();
                    }
                    self.state.set_status(e, true);
                }
            }
        }
        join_worker(handle);
        tick_spinner(&mut self.state.generation);
    }

    pub(super) fn drain_review_assist(&mut self) {
        let status = drain_review_stream(
            &mut self.state.review_assist_job,
            &mut self.state.review_assists,
            "review explanation ready",
        );
        if let Some((text, is_error)) = status {
            self.state.set_status(text, is_error);
        }
    }

    pub(super) fn drain_review_pr_text(&mut self) {
        let status = drain_review_stream(
            &mut self.state.review_pr_job,
            &mut self.state.review_assists,
            "PR text ready",
        );
        if let Some((text, is_error)) = status {
            self.state.set_status(text, is_error);
        }
    }

    pub(super) fn drain_review_chat(&mut self) {
        let mut handle = None;
        for msg in drain_messages(&self.state.review_chat_job) {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(output) => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        job.output.push_str(&output);
                    }
                }
                GenMsg::Reset => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        job.output.clear();
                    }
                }
                GenMsg::Done(final_msg) => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        handle = job.handle.take();
                    }
                    self.state.review_chat_job = None;
                    self.state
                        .review_chat_messages
                        .push(crate::state::ReviewChatMessage {
                            role: crate::state::ReviewChatRole::Assistant,
                            content: final_msg,
                        });
                    self.state.review_chat_scroll = u16::MAX;
                    self.state.set_status("review chat ready", false);
                }
                GenMsg::Error(error) => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        handle = job.handle.take();
                    }
                    self.state.review_chat_job = None;
                    self.state
                        .review_chat_messages
                        .push(crate::state::ReviewChatMessage {
                            role: crate::state::ReviewChatRole::Assistant,
                            content: format!("llm error: {error}"),
                        });
                    self.state.review_chat_scroll = u16::MAX;
                    self.state.set_status(error, true);
                }
            }
        }
        join_worker(handle);
        if self.state.review_chat_job.is_some() {
            tick_spinner(&mut self.state.review_chat_job);
            self.state.review_chat_scroll = u16::MAX;
        }
    }

    pub(super) fn join_background_jobs(&mut self) {
        let mut handles = Vec::new();
        handles.extend(self.state.take_deferred_threads());
        handles.extend(self.state.take_job_handles());

        if !handles.is_empty() {
            // The terminal has already been restored at this point, so tell the user
            // why the process has not exited yet instead of appearing to hang.
            eprintln!(
                "lg: waiting for {} background job(s) to finish\u{2026}",
                handles.len()
            );
        }
        for handle in handles {
            join_worker(Some(handle));
        }
    }
}

fn default_review_collapsed_nodes(review: &crate::git::AssistedReview) -> HashSet<String> {
    review
        .nodes
        .iter()
        .filter(|node| should_start_collapsed(review, node))
        .map(|node| node.id.clone())
        .collect()
}

fn should_start_collapsed(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> bool {
    if node.id == "branch" || node.id.starts_with("branch:category:") {
        return false;
    }
    if node.id == "checklist" {
        return false;
    }
    if node.id.contains(":file:") {
        return true;
    }
    let has_child = review
        .nodes
        .iter()
        .any(|candidate| candidate.parent.as_deref() == Some(node.id.as_str()));
    (node.parent.is_none() && !node.body.is_empty()) || (node.id.contains(":entry:") && has_child)
}

fn initial_review_index(review: &crate::git::AssistedReview) -> usize {
    review
        .nodes
        .iter()
        .position(|node| node.id.starts_with("branch:file:"))
        .or_else(|| review.nodes.iter().position(|node| node.id == "branch"))
        .unwrap_or(0)
}

fn review_path_ancestor_ids(review: &crate::git::AssistedReview, path: &str) -> Vec<String> {
    let Some(node_id) = review_node_id_for_path(review, path) else {
        return Vec::new();
    };
    let mut ancestors = Vec::new();
    let mut parent = review
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .and_then(|node| node.parent.as_deref());
    while let Some(parent_id) = parent {
        ancestors.push(parent_id.to_string());
        parent = review
            .nodes
            .iter()
            .find(|node| node.id == parent_id)
            .and_then(|node| node.parent.as_deref());
    }
    ancestors
}

fn review_node_id_for_path<'a>(
    review: &'a crate::git::AssistedReview,
    path: &str,
) -> Option<&'a str> {
    review
        .nodes
        .iter()
        .find(|node| node.id.contains(":file:") && review_title_path(&node.title) == Some(path))
        .or_else(|| {
            review
                .nodes
                .iter()
                .find(|node| review_title_path(&node.title) == Some(path))
        })
        .map(|node| node.id.as_str())
}

fn review_title_path(title: &str) -> Option<&str> {
    let location = title
        .split_once(" in ")
        .map(|(path, _)| path)
        .or_else(|| title.split_once(" - ").map(|(location, _)| location))
        .unwrap_or(title);
    let path = location
        .rsplit_once(':')
        .filter(|(_, line)| line.chars().all(|ch| ch.is_ascii_digit()))
        .map(|(path, _)| path)
        .unwrap_or(location)
        .trim();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{AssistedReview, ReviewNode};

    #[test]
    fn review_defaults_show_file_rows_but_keep_file_children_collapsed() {
        let review = AssistedReview {
            report: String::new(),
            nodes: vec![
                ReviewNode {
                    id: "branch".into(),
                    parent: None,
                    depth: 0,
                    title: "Full diff against main".into(),
                    body: Vec::new(),
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "branch:category:production".into(),
                    parent: Some("branch".into()),
                    depth: 1,
                    title: "Production (1 file, 1 entry point, +1 -1)".into(),
                    body: Vec::new(),
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "branch:file:0".into(),
                    parent: Some("branch:category:production".into()),
                    depth: 2,
                    title: "src/lib.rs - 1 entry point (+1 -1)".into(),
                    body: vec!["@@ -1 +1 @@".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "branch:entry:0".into(),
                    parent: Some("branch:file:0".into()),
                    depth: 3,
                    title: "src/lib.rs:1 in fn greet - updates greet (+1 -1)".into(),
                    body: vec!["@@ -1 +1 @@".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "summary".into(),
                    parent: None,
                    depth: 0,
                    title: "Summary".into(),
                    body: vec!["details".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "checklist".into(),
                    parent: None,
                    depth: 0,
                    title: "Review checklist".into(),
                    body: vec!["- Check this".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: crate::git::REVIEW_PR_TEXT_NODE_ID.into(),
                    parent: Some("checklist".into()),
                    depth: 1,
                    title: "PR text - generated by LLM (y copy)".into(),
                    body: Vec::new(),
                    context: Vec::new(),
                },
            ],
        };

        let collapsed = default_review_collapsed_nodes(&review);

        assert!(!collapsed.contains("branch"));
        assert!(!collapsed.contains("branch:category:production"));
        assert!(collapsed.contains("branch:file:0"));
        assert!(collapsed.contains("summary"));
        assert!(!collapsed.contains("checklist"));
        assert!(!collapsed.contains(crate::git::REVIEW_PR_TEXT_NODE_ID));
        assert_eq!(initial_review_index(&review), 2);

        let reveal_ids = review_path_ancestor_ids(&review, "src/lib.rs");
        assert_eq!(
            reveal_ids,
            vec![
                "branch:category:production".to_string(),
                "branch".to_string()
            ]
        );
    }
}
