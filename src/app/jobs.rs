use anyhow::Result;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{
    config::{
        BACKGROUND_FETCH_INTERVAL_SECS, BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST, COMMIT_LIST_LIMIT,
    },
    state::{
        CheckoutMsg, CommitLogJob, CommitLogMsg, DiffJob, DiffMsg, DiffSource, FetchJob, FetchMsg,
        GenMsg, Modal, OperationKind, OperationMsg, Pane, PushMsg, RefreshJob, RefreshMsg,
        ReleaseStatusJob, ReleaseStatusMsg, ReviewMsg, WorkflowMsg,
    },
};

use super::{
    App, build_refresh_snapshot, git_job_running, load_diff_text, selected_commit_ref,
    selected_diff_source, should_refresh_for_fs_event, spawn_push,
};

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

impl App {
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

    pub(super) fn start_fetch(&mut self) {
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

    fn defer_release_status_job(&mut self) {
        if let Some(mut job) = self.state.release_status_job.take() {
            self.state.defer_thread_join(job.handle.take());
        }
    }

    pub(super) fn start_diff_job(&mut self, force: bool) {
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

    pub(super) fn drain_refresh_job(&mut self) {
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

    pub(super) fn drain_diff_job(&mut self) {
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

    pub(super) fn drain_release_status_job(&mut self) {
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

    pub(super) fn drain_commit_log_job(&mut self) {
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

    pub(super) fn drain_review_job(&mut self) {
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
                    self.state.review_context_restore_collapsed.clear();
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

    pub(super) fn drain_fetch_job(&mut self) {
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

    pub(super) fn drain_push_job(&mut self) -> Result<()> {
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

    pub(super) fn drain_checkout_job(&mut self) -> Result<()> {
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

    pub(super) fn drain_operation_job(&mut self) -> Result<()> {
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
                    } else if kind == OperationKind::MergeUpstream {
                        self.state.modal = Modal::None;
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

    pub(super) fn drain_workflow_job(&mut self) -> Result<()> {
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

    pub(super) fn drain_review_assist(&mut self) {
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

    pub(super) fn drain_review_chat(&mut self) {
        let mut drained: Vec<GenMsg> = Vec::new();
        let mut handle = None;
        if let Some(job) = self.state.review_chat_job.as_ref() {
            while let Ok(msg) = job.rx.try_recv() {
                drained.push(msg);
            }
        }
        for msg in drained {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(output) => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        job.output.push_str(&output);
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
                            content: format!("ollama error: {error}"),
                        });
                    self.state.review_chat_scroll = u16::MAX;
                    self.state.set_status(error, true);
                }
            }
        }
        join_worker(handle);
        if let Some(job) = self.state.review_chat_job.as_mut() {
            job.spinner = job.spinner.wrapping_add(1);
            self.state.review_chat_scroll = u16::MAX;
        }
    }

    pub(super) fn join_background_jobs(&mut self) {
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
        if let Some(job) = self.state.review_chat_job.as_mut() {
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
