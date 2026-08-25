//! Taking in what a finished job sent, and applying it to the state.

use anyhow::Result;

use crate::state::{
    CheckoutMsg, CommitLogMsg, DiffMsg, FetchMsg, GenMsg, Modal, OperationKind, OperationMsg,
    PushMsg, RefreshMsg, ReleaseStatusMsg, SettingsSuggestMsg, WorkflowMsg,
};

use super::super::{App, selected_commit_ref, should_refresh_for_fs_event, spawn_push};
use super::{
    drain_messages, first_status_line, join_worker, open_conflict_modal_if_needed, take_finished,
    tick_spinner,
};

impl App {
    pub(in crate::app) fn drain_file_events(&mut self) -> Result<()> {
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

    pub(in crate::app) fn drain_refresh_job(&mut self) {
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

    pub(in crate::app) fn drain_diff_job(&mut self) {
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

    pub(in crate::app) fn drain_release_status_job(&mut self) {
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
    pub(in crate::app) fn drain_settings_suggest_job(&mut self) {
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

    pub(in crate::app) fn drain_commit_log_job(&mut self) {
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

    pub(in crate::app) fn drain_fetch_job(&mut self) {
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

    pub(in crate::app) fn drain_push_job(&mut self) -> Result<()> {
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

    pub(in crate::app) fn drain_checkout_job(&mut self) -> Result<()> {
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

    pub(in crate::app) fn drain_operation_job(&mut self) -> Result<()> {
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

    pub(in crate::app) fn drain_workflow_job(&mut self) -> Result<()> {
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

    pub(in crate::app) fn drain_generation(&mut self) {
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

    pub(in crate::app) fn join_background_jobs(&mut self) {
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
