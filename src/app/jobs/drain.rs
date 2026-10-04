//! Taking in what a finished job sent, and applying it to the state.

use anyhow::Result;

use crate::state::{
    CheckoutMsg, CommitLogMsg, DiffMsg, FetchMsg, GenMsg, Modal, OperationKind, OperationMsg,
    PushMsg, RefreshMsg, RefreshScope, ReleaseStatusMsg, SettingsSuggestMsg, WorkflowMsg,
};

use super::super::{App, selected_commit_ref, spawn_push};
use super::{
    drain_job, drain_messages, first_status_line, join_worker, open_conflict_modal_if_needed,
    stopped_status, take_finished, tick_spinner,
};

impl App {
    /// Gather what the watcher reported, and start the refresh it asks for
    /// once the burst it came in has settled. Returns whether one started.
    pub(in crate::app) fn drain_file_events(&mut self) -> Result<bool> {
        let now = std::time::Instant::now();
        let mut nested = None;
        while let Ok(event) = self.file_events.try_recv() {
            match event {
                Ok(event) => {
                    let nested = nested.get_or_insert_with(|| self.nested_in_checkout());
                    self.file_batch.add(&event, &self.watch_roots, nested, now);
                }
                Err(err) => {
                    self.state
                        .set_status(format!("file watch failed: {err}"), true);
                }
            }
        }
        let Some(due) = self.file_batch.take_due(now) else {
            return Ok(false);
        };
        // A refresh the watcher started is no news worth a status line; the
        // footer already shows one is running.
        self.start_scoped_refresh(due.scope, due.changed, true, false);
        Ok(true)
    }

    /// The repositories nested in the checkout being watched, relative to it.
    fn nested_in_checkout(&self) -> Vec<std::path::PathBuf> {
        let Some(workspace) = self.state.workspace_root.as_deref() else {
            return Vec::new();
        };
        let workspace = std::path::Path::new(workspace);
        self.state
            .nested_repositories
            .iter()
            .filter_map(|repo| {
                workspace
                    .join(&repo.path)
                    .strip_prefix(&self.watch_roots.root)
                    .ok()
                    .filter(|rel| !rel.as_os_str().is_empty())
                    .map(std::path::Path::to_path_buf)
            })
            .collect()
    }

    /// What a [`RefreshScope::Files`] refresh read: the file list, and the
    /// diff beside it. A refresh that found every changed file ignored read
    /// nothing, and leaves everything as it was.
    fn apply_files_snapshot(
        &mut self,
        snapshot: crate::state::RefreshSnapshot,
        refresh_diff: bool,
    ) {
        if let Some(error) = snapshot.errors.into_iter().next() {
            self.state.set_status(error, true);
        }
        let Some(files) = snapshot.files else {
            return;
        };
        // The worktree list's "has changes" for this checkout comes from the
        // same status, so it is kept in step without asking git again.
        let has_changes = !files.is_empty();
        if let Some(root) = self.state.repo_root.as_deref() {
            let root = std::path::Path::new(root);
            for worktree in &mut self.state.worktrees {
                if crate::git::same_dir(std::path::Path::new(&worktree.path), root) {
                    worktree.has_changes = has_changes;
                }
            }
        }
        self.state.files = files;
        self.state.files_generation = self.state.files_generation.wrapping_add(1);
        self.state.clamp();
        if refresh_diff {
            self.start_diff_job(true);
        }
    }

    fn apply_refresh_snapshot(
        &mut self,
        snapshot: crate::state::RefreshSnapshot,
        refresh_diff: bool,
    ) {
        if snapshot.scope == RefreshScope::Files {
            self.apply_files_snapshot(snapshot, refresh_diff);
            return;
        }
        // The checkouts were listed again; resolve their directories afresh.
        crate::git::forget_resolved_dirs();
        self.state.decorative_animations = snapshot.decorative_animations;
        if let Some(author) = snapshot.commit_author {
            self.state.commit_author = author;
        }
        let repo_before = self.state.repo_root.clone();
        self.state.repo_root = snapshot.repo_root;
        let repo_changed = self.state.repo_root != repo_before;
        if repo_changed {
            // A folder that just became a checkout keeps its directory, so the
            // preferences read for it before would otherwise stand.
            crate::preferences::invalidate();
        }
        if repo_changed && self.state.history_file.is_some() {
            self.state.enable_history();
        }
        self.state.workspace_root = snapshot.workspace_root;
        if let Some(files) = snapshot.files {
            self.state.files = files;
            self.state.files_generation = self.state.files_generation.wrapping_add(1);
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
        if let Some(commits) = snapshot.commits
            && selected_ref.as_deref() == self.state.branch.as_deref()
        {
            self.state.commits = commits;
            self.state.commits_ref = selected_ref.clone();
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
        let Some((mut job, msg)) = take_finished(&mut self.state.refresh_job) else {
            return;
        };
        let pending_refresh = self.state.refresh_pending.take();
        let pending_diff = self.state.refresh_pending_diff;
        join_worker(job.handle.take());
        self.state.refresh_pending_diff = false;
        match msg {
            Ok(RefreshMsg::Done(snapshot)) => {
                self.apply_refresh_snapshot(*snapshot, job.refresh_diff);
            }
            Err(stopped) => self.state.set_status(stopped, true),
        }
        if let Some(scope) = pending_refresh {
            self.start_scoped_refresh(scope, Vec::new(), pending_diff, scope > RefreshScope::Files);
        }
    }

    pub(in crate::app) fn drain_diff_job(&mut self) {
        let Some((mut job, msg)) = take_finished(&mut self.state.diff_job) else {
            return;
        };
        join_worker(job.handle.take());
        let DiffMsg::Done { source, text } = match msg {
            Ok(msg) => msg,
            Err(stopped) => {
                self.state.set_status(stopped, true);
                return;
            }
        };
        if source == self.state.diff_source {
            self.state.set_diff_text(text);
            crate::panel::main::settle_cursor(&mut self.state);
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
                Err(stopped) => self.state.set_status(stopped, true),
                Ok(ReleaseStatusMsg::Done { branch, status }) => {
                    if self.state.branch.as_deref() == Some(branch.as_str()) {
                        self.state.current_branch_releases = status;
                        self.state.current_branch_releases_ref = Some(branch);
                    }
                }
                Ok(ReleaseStatusMsg::Error { branch, message }) => {
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
            Err(stopped) => self.state.set_status(stopped, true),
            Ok(SettingsSuggestMsg::Done { language, shapes }) => {
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
            Ok(SettingsSuggestMsg::Error(message)) => {
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
                Err(stopped) => self.state.set_status(stopped, true),
                Ok(CommitLogMsg::Done { branch, commits }) => {
                    if self.state.commits_ref.as_deref() == Some(branch.as_str()) {
                        self.state.commits = commits;
                        self.state.commits_list.idx = 0;
                        self.state.clamp();
                    }
                }
                Ok(CommitLogMsg::Error { branch, message }) => {
                    if self.state.commits_ref.as_deref() == Some(branch.as_str()) {
                        self.state.commits.clear();
                        self.state.commits_list.idx = 0;
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
            Ok(FetchMsg::Done(s)) => self.state.set_status(s, false),
            Ok(FetchMsg::NoRemotes) => {}
            Ok(FetchMsg::Error(e)) => self.state.set_status(first_status_line(&e), true),
            Err(stopped) => self.state.set_status(stopped, true),
        }
        self.start_refresh_with_status(false, false);
        super::super::spawn::release_queued_after_fetch(&mut self.state);
    }

    pub(in crate::app) fn drain_push_job(&mut self) -> Result<()> {
        let Some((mut job, msg)) = take_finished(&mut self.state.push_job) else {
            return Ok(());
        };
        join_worker(job.handle.take());
        // A push started with P runs behind no modal; whatever the user opened
        // in the meantime (Shift-F's flow modal, say) is theirs to close.
        if matches!(self.state.modal, Modal::Push) {
            self.state.modal = Modal::None;
        }
        self.state.current_branch_releases_ref = None;
        match msg {
            Ok(PushMsg::Done(s)) => self.state.set_status(s, false),
            Ok(PushMsg::Error(e)) | Err(e) => self.state.set_status(e, true),
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
            Ok(CheckoutMsg::Done(s)) => self.state.set_status(s, false),
            Ok(CheckoutMsg::Error(e)) | Err(e) => {
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
        let drained = drain_messages(&self.state.operation_job);
        for msg in drained.messages {
            match msg {
                OperationMsg::Progress(step) => {
                    if let Some(job) = self.state.operation_job.as_mut() {
                        job.step = Some(step);
                    }
                }
                ended => finished = Some(ended),
            }
        }
        // A worker gone without the message that ends the job failed it: one
        // left running would hold the git job slot for good.
        if finished.is_none()
            && drained.disconnected
            && let Some(job) = self.state.operation_job.as_mut()
        {
            finished = Some(OperationMsg::Error(stopped_status(
                job.label,
                job.handle.take(),
            )));
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
        let succeeded = matches!(msg, OperationMsg::Done(_));
        match msg {
            OperationMsg::Done(s) => {
                if !matches!(self.state.modal, Modal::Conflict) {
                    self.state.conflict.followup = None;
                }
                self.state.set_status(s, false);
                if kind == OperationKind::Commit {
                    let dir = self.state.commit_dir();
                    self.state.forget_set_aside_message(&dir);
                    self.state.modal = Modal::None;
                    self.state.cancel_generation();
                    self.state.commit_message.clear();
                    self.state.commit_cursor = 0;
                    self.state.commit_amend = false;
                    self.state.commit_amend_draft = None;
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
                    self.state.conflict.followup = None;
                    self.state.set_status(e, true);
                }
            }
            // Filtered out above; only Done and Error reach here.
            OperationMsg::Progress(_) => {}
        }
        self.after_github_operation(kind, succeeded);
        if kind == OperationKind::Stash && self.state.modal == Modal::Stash {
            self.state.stash.reload();
        }
        self.start_refresh(true);
        Ok(())
    }

    pub(in crate::app) fn drain_workflow_job(&mut self) -> Result<()> {
        // Progress reports arrive before the one message that ends the job, so
        // this drains rather than taking the last message.
        let mut finished = None;
        let drained = drain_messages(&self.state.workflow_job);
        for msg in drained.messages {
            match msg {
                WorkflowMsg::Progress(step) => {
                    if let Some(job) = self.state.workflow_job.as_mut() {
                        job.current_step = Some(step);
                    }
                }
                done_or_error => finished = Some(done_or_error),
            }
        }
        if finished.is_none()
            && drained.disconnected
            && let Some(job) = self.state.workflow_job.as_mut()
        {
            finished = Some(WorkflowMsg::Error(stopped_status(
                &job.label,
                job.handle.take(),
            )));
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
        let finished_action = self
            .state
            .workflow_job
            .as_ref()
            .and_then(|job| job.flow.as_ref())
            .map(|flow| flow.action);
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
                        // Continuing means the rest of the flow, not just the
                        // merge that was in the way.
                        self.state.settle_conflict(
                            finished_label.as_deref() == Some("validate conflict resolution"),
                        );
                    } else if !matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict.followup = None;
                    }
                    if matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict.log = s.clone();
                    } else {
                        self.state.modal = Modal::None;
                    }
                    if finished_action == Some(crate::state::FlowAction::CleanOrphans) {
                        // One line per branch, so the history says which were
                        // deleted and why any were kept; the bar shows the sum.
                        for line in s.lines().skip(1).filter(|line| !line.trim().is_empty()) {
                            self.state.set_status(line, false);
                        }
                    }
                    self.state.set_status(first_status_line(&s), false);
                }
                WorkflowMsg::Error(e) => {
                    let conflicts = crate::git::conflicted_files().unwrap_or_default();
                    self.state.set_conflicts(conflicts);
                    if !self.state.conflict.files.is_empty() {
                        self.state.conflict.log = e.clone();
                        self.state.modal = Modal::Conflict;
                        self.state.set_status("merge conflicts detected", true);
                        self.start_refresh(true);
                        return Ok(());
                    }
                    if matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict.log = e.clone();
                        self.state.modal = Modal::None;
                    }
                    if !matches!(self.state.modal, Modal::Conflict) {
                        self.state.conflict.followup = None;
                    }
                    self.state.set_status(first_status_line(&e), true);
                }
            }
            self.start_refresh(true);
        }
        Ok(())
    }

    /// Take in what every checkout's generation has sent. Each draft is its
    /// own stream: one checkout's message arriving must not land in another
    /// checkout's draft, and a message that finishes while its checkout is
    /// off screen waits in its draft rather than jumping into the editor.
    pub(in crate::app) fn drain_generation(&mut self) {
        let mut i = 0;
        while i < self.state.commit_drafts.len() {
            if self.drain_draft(i) {
                i += 1;
            }
        }
        for draft in &mut self.state.commit_drafts {
            if let Some(generation) = draft.generation.as_mut() {
                generation.spinner = generation.spinner.wrapping_add(1);
            }
        }
    }

    /// Take in what the draft at `i` has sent. False when the draft is gone
    /// from the list, which leaves the next one at the same index.
    fn drain_draft(&mut self, i: usize) -> bool {
        let Some(generation) = self.state.commit_drafts[i].generation.as_ref() else {
            return true;
        };
        let mut handle = None;
        let mut kept = true;
        let drained = drain_job(generation);
        for msg in drained.messages {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(o) => {
                    if let Some(g) = self.state.commit_drafts[i].generation.as_mut() {
                        g.receive(&o);
                    }
                }
                GenMsg::Reset => {
                    if let Some(g) = self.state.commit_drafts[i].generation.as_mut() {
                        g.restart();
                    }
                }
                GenMsg::Done { text, stats } => {
                    self.state.model_server_unreachable = false;
                    // Held until the words still queued or in flight have
                    // landed; the editor takes the text over after that.
                    if let Some(g) = self.state.commit_drafts[i].generation.as_mut() {
                        handle = g.handle.take();
                        g.finished = Some((text, stats));
                    }
                }
                GenMsg::Error(e) => {
                    if let Some(g) = self.state.commit_drafts[i].generation.as_mut() {
                        handle = g.handle.take();
                    }
                    self.state.drop_draft(i);
                    kept = false;
                    self.state.report_generation_error(e);
                }
            }
            if !kept {
                break;
            }
        }
        if kept {
            let now = self.state.animation_ms;
            let animate = self.state.decorative_animations;
            let finished = self.state.commit_drafts[i]
                .generation
                .as_mut()
                .and_then(|g| {
                    g.release(now, animate);
                    if g.settled(now) || !animate {
                        g.finished.take()
                    } else {
                        None
                    }
                });
            if let Some((text, stats)) = finished {
                kept = self.finish_draft(i, text, stats);
            }
        }
        // The draft is dropped as for an error, rather than left generating
        // with nothing left to generate it. A finished message waiting for
        // its words to land has nothing left to generate.
        if kept
            && drained.disconnected
            && self.state.commit_drafts[i]
                .generation
                .as_ref()
                .is_some_and(|g| g.finished.is_none())
            && let Some(mut generation) = self.state.commit_drafts[i].generation.take()
        {
            let stopped = super::stopped(&mut generation);
            self.state.drop_draft(i);
            kept = false;
            self.state.set_status(stopped, true);
        }
        join_worker(handle);
        kept
    }

    /// Hand the finished message at `i` over to its draft or the editor.
    /// False when the draft is gone from the list.
    fn finish_draft(&mut self, i: usize, text: String, stats: crate::llm::GenStats) -> bool {
        let was = self.state.commit_drafts.len();
        self.state.finish_commit_draft(i, text);
        // A message that ran out of budget goes in the editable field either
        // way — it is a draft, and half a draft is still a starting point. It
        // is reported as an error so it lingers, because the one way to commit
        // a truncated message is not to notice it was truncated.
        if stats.truncated {
            self.state.set_status(
                "message generated but cut off at the token budget \u{2014} finish it before committing",
                true,
            );
        } else {
            let status = match stats.summary() {
                Some(summary) => format!("message generated \u{b7} {summary}"),
                None => "message generated".to_string(),
            };
            self.state.set_status(status, false);
        }
        self.state.commit_drafts.len() == was
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
