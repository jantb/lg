//! Starting a background job, and deferring the ones that can wait.

use std::time::{Duration, Instant};

use crate::{
    config::{BACKGROUND_FETCH_INTERVAL_SECS, COMMIT_LIST_LIMIT, is_protected_branch_name},
    state::{
        CommitLogJob, CommitLogMsg, DiffJob, DiffMsg, DiffSource, FetchJob, FetchMsg, Pane,
        RefreshJob, RefreshMsg, ReleaseStatusJob, ReleaseStatusMsg,
    },
};

use super::super::{
    App, build_refresh_snapshot, git_job_running, load_diff_text, selected_commit_ref,
    selected_diff_source,
};

impl App {
    /// Point lg at `dir`: git commands, the file watcher and every per-checkout
    /// piece of state follow it. Used for nested repositories and worktrees
    /// alike, so switching between them is one code path.
    pub(in crate::app) fn switch_to_repository(&mut self, dir: &std::path::Path, label: &str) {
        crate::git::set_active_repo(dir);

        // A failed watcher only costs automatic refreshes, so the switch still
        // goes through — but say so, because staleness is otherwise silent.
        let watch_error = match crate::app::refresh::watch_repo(dir) {
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

    pub(in crate::app) fn start_refresh(&mut self, refresh_diff: bool) {
        self.start_refresh_with_status(refresh_diff, true);
    }

    pub(in crate::app) fn start_refresh_with_status(
        &mut self,
        refresh_diff: bool,
        show_status: bool,
    ) {
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

    pub(in crate::app) fn start_fetch(&mut self) {
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

    pub(in crate::app) fn maybe_start_periodic_fetch(&mut self) {
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

    pub(in crate::app) fn defer_release_status_job(&mut self) {
        if let Some(mut job) = self.state.release_status_job.take() {
            self.state.defer_thread_join(job.handle.take());
        }
    }

    pub(in crate::app) fn start_diff_job(&mut self, force: bool) {
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

    pub(in crate::app) fn sync_commit_log_to_selection(&mut self) {
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
    pub(in crate::app) fn clear_release_status(&mut self, checked: Option<String>) {
        self.state.current_branch_releases = Default::default();
        self.state.current_branch_releases_ref = checked;
        self.defer_release_status_job();
    }

    pub(in crate::app) fn sync_release_status_to_branch(&mut self) {
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
}
