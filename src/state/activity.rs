//! What the app is doing right now: running jobs, spawned threads, the status line.

use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use chrono::{DateTime, Utc};

use super::{AppState, BackgroundJob, CommitDraft, GenMsg, Generation, Modal, PendingAction};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatusMsg {
    pub text: String,
    pub is_error: bool,
    pub at: DateTime<Utc>,
}

impl StatusMsg {
    /// How long ago the message was set, in milliseconds, never negative.
    pub fn age_ms(&self) -> i64 {
        (Utc::now() - self.at).num_milliseconds().max(0)
    }
}

impl AppState {
    /// Read the animation clock. Called once per frame; the clock follows wall
    /// time rather than counting frames, which is what keeps a spinner at one
    /// speed whether lg is idle or redrawing at `ANIMATION_FRAME_MS`.
    pub fn advance_animation(&mut self) {
        self.animation_ms = self.animation_started.elapsed().as_millis() as u64;
        self.animation_tick = (self.animation_ms / crate::config::ANIMATION_STEP_MS) as usize;
    }

    /// Move the animation clock forward by `by`, as though that much time had
    /// passed. Lets a test look at a later frame without waiting for it.
    pub fn skip_animation(&mut self, by: std::time::Duration) {
        self.animation_started -= by;
        self.advance_animation();
    }

    /// Hand over every running job's worker handle so the caller can wait for
    /// them. Every job field is listed here: one left out is a worker the
    /// process can exit from under.
    pub fn take_job_handles(&mut self) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();
        macro_rules! take {
            ($($job:ident),+ $(,)?) => { $(
                if let Some(job) = self.$job.as_mut() {
                    handles.extend(job.handle_mut().take());
                }
            )+ };
        }
        // Every checkout's generation, then the single-slot jobs.
        for draft in &mut self.commit_drafts {
            if let Some(generation) = draft.generation.as_mut() {
                handles.extend(generation.handle_mut().take());
            }
        }
        take!(
            push_job,
            checkout_job,
            operation_job,
            fetch_job,
            refresh_job,
            release_status_job,
            settings_suggest_job,
            commit_log_job,
            diff_job,
            review_job,
            review_assist_job,
            review_pr_job,
            review_flag_job,
            review_chat_job,
            conflict_resolve_job,
            workflow_job,
        );
        handles
    }

    /// Whether any background job is in flight. The event loop polls faster
    /// while one is, so its result lands without waiting out a full tick.
    pub fn any_job_running(&self) -> bool {
        self.any_generating()
            || self.push_job.is_some()
            || self.checkout_job.is_some()
            || self.operation_job.is_some()
            || self.fetch_job.is_some()
            || self.refresh_job.is_some()
            || self.release_status_job.is_some()
            || self.settings_suggest_job.is_some()
            || self.commit_log_job.is_some()
            || self.diff_job.is_some()
            || self.review_job.is_some()
            || self.review_assist_job.is_some()
            || self.review_pr_job.is_some()
            || self.review_flag_job.is_some()
            || self.review_chat_job.is_some()
            || self.conflict_resolve_job.is_some()
            || self.workflow_job.is_some()
            || self.github.loading()
    }

    /// What the running operation is doing right now, for the ones that report
    /// their steps. The label alone says a land is running; this says whether
    /// it is still fetching or already deleting.
    pub fn activity_detail(&self) -> Option<&str> {
        self.operation_job.as_ref()?.step.as_deref()
    }

    /// Whether the activity the footer names is a request to the model server,
    /// as opposed to git work that happens to overlap one. The server's phase
    /// readout is only meaningful next to the former.
    pub fn activity_is_llm(&self) -> bool {
        self.activity_label().is_some_and(|label| {
            matches!(
                label,
                "generating"
                    | "reading conventions"
                    | "reviewing"
                    | "explaining"
                    | "flagging style"
                    | "writing PR text"
                    | "chatting"
                    | "resolving conflicts"
            )
        })
    }

    pub fn activity_label(&self) -> Option<&'static str> {
        if self.any_generating() {
            Some("generating")
        } else if self.push_job.is_some() {
            Some("pushing")
        } else if self.checkout_job.is_some() {
            Some("checking out")
        } else if let Some(job) = &self.operation_job {
            Some(job.label)
        } else if self.fetch_job.is_some() {
            Some("fetching")
        } else if self.refresh_job.is_some() {
            Some("refreshing")
        } else if self.release_status_job.is_some() {
            Some("checking deployments")
        } else if self.commit_log_job.is_some() {
            Some("loading commits")
        } else if self.diff_job.is_some() {
            Some("loading diff")
        } else if self.settings_suggest_job.is_some() {
            Some("reading conventions")
        } else if self.review_job.is_some() {
            Some("reviewing")
        } else if self.review_assist_job.is_some() {
            Some("explaining")
        } else if self.review_flag_job.is_some() {
            Some("flagging style")
        } else if self.review_pr_job.is_some() {
            Some("writing PR text")
        } else if self.review_chat_job.is_some() {
            Some("chatting")
        } else if self.conflict_resolve_job.is_some() {
            Some("resolving conflicts")
        } else if self.workflow_job.is_some() {
            Some("running branch action")
        } else if self.github.loading() {
            Some("reading GitHub")
        } else {
            match &self.pending_action {
                Some(PendingAction::GenerateMessage) => Some("starting generator"),
                Some(PendingAction::ReviewAssist(_)) => Some("starting explanation"),
                Some(PendingAction::ReviewPrText) => Some("starting PR text"),
                Some(PendingAction::ReviewStyleFlags) => Some("starting style flag pass"),
                Some(PendingAction::ReviewAgent) => Some("starting agent review"),
                Some(PendingAction::ReviewChat(_)) => Some("starting chat"),
                Some(PendingAction::CopyToClipboard { .. }) => Some("copying"),
                Some(PendingAction::Commit) => Some("committing"),
                Some(PendingAction::StageAllAndCommit) => Some("staging"),
                Some(PendingAction::Push) => Some("starting push"),
                Some(PendingAction::Pull) => Some("starting pull"),
                Some(PendingAction::MergeUpstream) => Some("starting merge"),
                Some(PendingAction::MergeMainAllBranches) => Some("starting branch sync"),
                Some(PendingAction::Promote(_)) => Some("promoting"),
                Some(PendingAction::Flow(_)) => Some("starting branch action"),
                Some(
                    PendingAction::SaveAuthor { .. }
                    | PendingAction::ClearAuthor
                    | PendingAction::SaveSubtreeAuthor { .. }
                    | PendingAction::ClearSubtreeAuthor { .. },
                ) => Some("saving author"),
                Some(PendingAction::SaveSettings { .. } | PendingAction::ClearSettings) => {
                    Some("saving settings")
                }
                Some(PendingAction::EditCommitPrompt) => Some("opening commit prompt"),
                Some(PendingAction::EditReviewStyle) => Some("opening review style"),
                Some(PendingAction::StageAll | PendingAction::StagePath(_)) => Some("staging"),
                Some(PendingAction::UnstageAll | PendingAction::UnstagePath(_)) => {
                    Some("unstaging")
                }
                Some(PendingAction::RollbackPath { .. }) => Some("rolling back"),
                Some(PendingAction::DeletePath { .. }) => Some("deleting"),
                Some(PendingAction::IgnorePath { .. }) => Some("updating gitignore"),
                Some(PendingAction::OpenProject | PendingAction::OpenProjectAt(_)) => {
                    Some("opening project")
                }
                Some(PendingAction::OpenFile(_)) => Some("opening file"),
                Some(PendingAction::DeleteBranch { .. }) => Some("deleting branch"),
                Some(PendingAction::SetBranchUpstream { .. }) => Some("setting upstream"),
                Some(PendingAction::SwitchRepository { .. }) => Some("switching repo"),
                Some(PendingAction::InitRepository { .. }) => Some("initializing repository"),
                Some(PendingAction::CreateWorktree { .. }) => Some("adding worktree"),
                Some(PendingAction::RemoveWorktree { .. }) => Some("removing worktree"),
                Some(PendingAction::LandWorktree { .. }) => Some("landing worktree"),
                Some(PendingAction::SyncWorktree { .. }) => Some("syncing worktree"),
                Some(PendingAction::BringWorktreeHome { .. }) => Some("moving branch home"),
                Some(PendingAction::PruneWorktrees) => Some("pruning worktrees"),
                Some(PendingAction::StartAgent { .. } | PendingAction::StartSession { .. }) => {
                    Some("starting session")
                }
                Some(PendingAction::GitHub(_)) => Some("starting GitHub action"),
                Some(PendingAction::Quit) => Some("quitting"),
                None => None,
            }
        }
    }

    /// The checkout a commit message is written for: the repository on
    /// screen. With no checkout the draft has no row in the tree; the modal
    /// itself is still the way back to it.
    pub fn commit_dir(&self) -> String {
        self.repo_root
            .clone()
            .or_else(|| self.workspace_root.clone())
            .unwrap_or_default()
    }

    /// Where the draft for `dir` sits, matched the way sessions are: git and
    /// the workspace scan can name the same checkout differently, and a
    /// symlink between them must not lose the draft.
    pub fn draft_idx(&self, dir: &str) -> Option<usize> {
        self.commit_drafts.iter().position(|draft| {
            crate::session::same_dir(std::path::Path::new(&draft.dir), std::path::Path::new(dir))
        })
    }

    /// The draft for the repository on screen, which is the one the commit
    /// modal is about.
    pub fn current_draft(&self) -> Option<&CommitDraft> {
        self.commit_drafts.get(self.draft_idx(&self.commit_dir())?)
    }

    /// The generation the commit modal shows: the one writing this
    /// checkout's message. A message being written for another checkout is
    /// that checkout's business and stays out of this modal.
    pub fn generation(&self) -> Option<&Generation> {
        self.current_draft()?.generation.as_ref()
    }

    /// Whether the repository on screen is having its message written.
    pub fn generating(&self) -> bool {
        self.generation().is_some()
    }

    /// Whether any checkout is.
    pub fn any_generating(&self) -> bool {
        self.commit_drafts.iter().any(CommitDraft::generating)
    }

    pub fn start_generation(
        &mut self,
        rx: Receiver<GenMsg>,
        handle: JoinHandle<()>,
        feed: crate::panel::commit_art::Feed,
    ) {
        let dir = self.commit_dir();
        // One draft to a checkout: asking again replaces what was there,
        // whatever state it had reached.
        self.cancel_generation_at(&dir);
        let generation = Generation {
            rx,
            handle: Some(handle),
            output: String::new(),
            spinner: 0,
            scene: crate::panel::commit_art::fresh_seed(),
            arrivals: Vec::new(),
            first_output_ms: None,
            feed,
        };
        self.commit_drafts.push(CommitDraft {
            dir,
            generation: Some(generation),
            text: String::new(),
            ready: false,
        });
    }

    pub fn defer_thread_join(&mut self, handle: Option<JoinHandle<()>>) {
        if let Some(handle) = handle {
            self.deferred_threads.push(handle);
        }
    }

    pub fn reap_deferred_threads(&mut self) {
        let mut i = 0;
        while i < self.deferred_threads.len() {
            if self.deferred_threads[i].is_finished() {
                let handle = self.deferred_threads.swap_remove(i);
                let _ = handle.join();
            } else {
                i += 1;
            }
        }
    }

    pub fn take_deferred_threads(&mut self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.deferred_threads)
    }

    /// Cancel any in-flight LLM work and report what was stopped.
    ///
    /// Dropping a job drops its receiver; the streaming loop in `llm` bails out
    /// as soon as a send fails, so this really does stop the work rather than
    /// just hiding it. The assisted-review builder is not a stream, so it is
    /// detached and its result discarded.
    pub fn cancel_llm_jobs(&mut self) -> Option<&'static str> {
        let mut cancelled = None;

        if let Some(mut job) = self.conflict_resolve_job.take() {
            self.defer_thread_join(job.handle.take());
            cancelled = Some("local conflict resolution cancelled");
        }
        if let Some(mut job) = self.review_chat_job.take() {
            self.defer_thread_join(job.handle.take());
            cancelled = Some("review chat cancelled");
        }
        if let Some(mut job) = self.review_flag_job.take() {
            self.defer_thread_join(job.handle.take());
            self.review_flag_active_path = None;
            cancelled = Some("style flag pass cancelled");
        }
        if let Some(mut job) = self.review_pr_job.take() {
            self.defer_thread_join(job.handle.take());
            cancelled = Some("PR text cancelled");
        }
        if let Some(mut job) = self.review_assist_job.take() {
            self.defer_thread_join(job.handle.take());
            cancelled = Some("explanation cancelled");
        }
        if let Some(message) = self.cancel_settings_suggest() {
            cancelled = Some(message);
        }
        if let Some(mut job) = self.review_job.take() {
            self.defer_thread_join(job.handle.take());
            // Otherwise the pane keeps claiming it is still building the review.
            self.set_diff_text("review cancelled".to_string());
            cancelled = Some("review cancelled");
        }
        if self.any_generating() {
            self.cancel_all_generations();
            cancelled = Some("generation cancelled");
        }

        cancelled
    }

    /// Stop the convention scan alone. Closing the settings modal must not take
    /// a review or a commit message down with it.
    pub fn cancel_settings_suggest(&mut self) -> Option<&'static str> {
        let mut job = self.settings_suggest_job.take()?;
        self.defer_thread_join(job.handle.take());
        Some("convention scan cancelled")
    }

    /// True when [`cancel_llm_jobs`] would stop something.
    pub fn llm_job_running(&self) -> bool {
        self.settings_suggest_job.is_some()
            || self.review_job.is_some()
            || self.review_assist_job.is_some()
            || self.review_pr_job.is_some()
            || self.review_flag_job.is_some()
            || self.review_chat_job.is_some()
            || self.conflict_resolve_job.is_some()
            || self.any_generating()
    }

    /// Drop the draft for the repository on screen, stopping the model if it
    /// is still writing.
    pub fn cancel_generation(&mut self) {
        let dir = self.commit_dir();
        self.cancel_generation_at(&dir);
    }

    /// The same for a named checkout: the workspace tree can set aside a
    /// draft belonging to a checkout other than the one on screen.
    pub fn cancel_generation_at(&mut self, dir: &str) {
        let Some(i) = self.draft_idx(dir) else { return };
        self.drop_draft(i);
    }

    /// Take the draft out and let go of the thread behind it. Dropping the
    /// receiver is what stops the model: the streaming loop bails out as
    /// soon as a send fails.
    pub(crate) fn drop_draft(&mut self, i: usize) -> Option<CommitDraft> {
        if i >= self.commit_drafts.len() {
            return None;
        }
        let mut draft = self.commit_drafts.remove(i);
        if let Some(generation) = draft.generation.as_mut() {
            let handle = generation.handle.take();
            self.defer_thread_join(handle);
        }
        Some(draft)
    }

    /// Stop every checkout's generation.
    pub fn cancel_all_generations(&mut self) {
        while !self.commit_drafts.is_empty() {
            self.drop_draft(0);
        }
    }

    /// The draft at `i` has been written. With its own checkout's modal
    /// open the message goes straight into the editor and the sub-line has
    /// done its job; otherwise the sub-line keeps standing, marked ready and
    /// holding the message, until the checkout it belongs to opens it.
    pub fn finish_commit_draft(&mut self, i: usize, text: String) {
        let on_screen = self
            .draft_idx(&self.commit_dir())
            .is_some_and(|current| current == i);
        if on_screen && self.modal == Modal::Commit {
            self.drop_draft(i);
            self.commit_message = text;
            self.commit_cursor = self.commit_message.chars().count();
            return;
        }
        if let Some(draft) = self.commit_drafts.get_mut(i) {
            draft.generation = None;
            draft.text = text;
            draft.ready = true;
        }
    }

    /// Bring a finished draft into the editor, if the checkout on screen has
    /// one waiting. Called as its modal opens: until then the message sits
    /// in the draft, so that opening one checkout's message cannot put
    /// another checkout's into the editor.
    pub fn take_ready_draft(&mut self) {
        let Some(i) = self.draft_idx(&self.commit_dir()) else {
            return;
        };
        if self.commit_drafts[i].generating() {
            return;
        }
        if let Some(draft) = self.drop_draft(i) {
            self.commit_message = draft.text;
            self.commit_cursor = self.commit_message.chars().count();
        }
    }

    pub fn enable_history(&mut self) {
        self.history_file = crate::preferences::scope_path(crate::preferences::Scope::Repository)
            .ok()
            .map(|p| p.with_file_name("activity.json"));
        if let Some(path) = &self.history_file {
            self.status_history = std::fs::read(path)
                .ok()
                .and_then(|data| serde_json::from_slice::<Vec<StatusMsg>>(&data).ok())
                .unwrap_or_default();
            if self.status_history.len() > 500 {
                self.status_history.drain(..self.status_history.len() - 500);
            }
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        let status = StatusMsg {
            text: text.into(),
            is_error,
            at: Utc::now(),
        };
        if self.status_history.len() >= 500 {
            self.status_history.remove(0);
        }
        self.status_history.push(status.clone());
        if let Some(path) = &self.history_file
            && let Ok(data) = serde_json::to_vec(&self.status_history)
        {
            let _ = crate::preferences::atomic_write(path, &data);
        }
        self.status = Some(status);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::ReviewJob;
    use super::*;

    #[test]
    fn the_animation_clock_ignores_extra_frames() {
        let mut state = AppState::new();
        let start = state.animation_tick;
        // A session on screen redraws at SESSION_TICK_MS; a burst of those
        // frames is still well inside one animation step.
        for _ in 0..200 {
            state.advance_animation();
        }
        assert_eq!(
            state.animation_tick, start,
            "animation speed must not follow the frame rate"
        );
    }

    /// Start a generation for `dir` and hand back the sender, so the test
    /// can drive it the way the model would.
    fn generate_in(state: &mut AppState, dir: &str) -> std::sync::mpsc::Sender<GenMsg> {
        state.repo_root = Some(dir.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        state.start_generation(rx, std::thread::spawn(|| {}), Default::default());
        tx
    }

    /// Two checkouts can have their messages written at once, and each
    /// modal shows its own: the whole point of leaving one in the
    /// background is to get on with the next repository meanwhile.
    #[test]
    fn each_checkout_writes_its_own_message_at_the_same_time() {
        let mut state = AppState::new();
        let _one = generate_in(&mut state, "/one");
        let _two = generate_in(&mut state, "/two");

        assert_eq!(state.commit_drafts.len(), 2, "one draft to a checkout");
        assert!(state.generating(), "the checkout on screen is writing");
        assert_eq!(state.current_draft().map(|d| d.dir.as_str()), Some("/two"));

        state.repo_root = Some("/one".into());
        assert_eq!(state.current_draft().map(|d| d.dir.as_str()), Some("/one"));

        // A checkout with no draft of its own sees no generation, however
        // much is being written elsewhere.
        state.repo_root = Some("/three".into());
        assert!(
            state.generation().is_none(),
            "another checkout's is not ours"
        );
        assert!(state.any_generating(), "though work is still in flight");
    }

    /// A message that finishes while its checkout is off screen waits in
    /// its own draft rather than landing in the editor, which is pointed at
    /// another repository.
    #[test]
    fn a_message_finished_off_screen_waits_for_its_own_checkout() {
        let mut state = AppState::new();
        let _one = generate_in(&mut state, "/one");
        let _two = generate_in(&mut state, "/two");
        state.commit_message = "written by hand for two".into();

        let one = state.draft_idx("/one").expect("the first draft");
        state.finish_commit_draft(one, "feat: one".into());

        assert_eq!(
            state.commit_message, "written by hand for two",
            "the editor still holds the checkout it is pointed at"
        );
        let draft = &state.commit_drafts[state.draft_idx("/one").expect("still listed")];
        assert!(draft.ready && !draft.generating(), "it is done and waiting");

        // Only opening that checkout's modal brings it in, and it does not
        // start another generation over the top of it.
        state.repo_root = Some("/one".into());
        state.commit_message.clear();
        state.open_commit_modal();
        assert_eq!(state.commit_message, "feat: one");
        assert_eq!(state.pending_action, None, "the message is already written");
        assert!(
            state.draft_idx("/one").is_none(),
            "looked at, so no longer listed"
        );
    }

    /// Setting one checkout's message aside leaves the others alone.
    #[test]
    fn cancelling_one_checkout_leaves_the_others_writing() {
        let mut state = AppState::new();
        let _one = generate_in(&mut state, "/one");
        let _two = generate_in(&mut state, "/two");

        state.cancel_generation();
        assert!(
            state.draft_idx("/two").is_none(),
            "the one on screen is gone"
        );
        assert!(state.draft_idx("/one").is_some(), "the other keeps writing");

        state.cancel_all_generations();
        assert!(state.commit_drafts.is_empty());
    }

    #[test]
    fn cancelling_a_review_resizes_the_pane_to_its_notice() {
        let mut state = AppState::new();
        state.set_diff_text("a long review\n".repeat(50));
        let (_tx, rx) = std::sync::mpsc::channel();
        state.review_job = Some(ReviewJob {
            rx,
            handle: None,
            spinner: 0,
        });

        assert_eq!(state.cancel_llm_jobs(), Some("review cancelled"));
        assert_eq!(
            state.diff_line_count, 1,
            "the notice is one line, so scrolling must stop there"
        );
    }

    #[test]
    fn the_animation_clock_advances_once_a_step_has_passed() {
        let mut state = AppState::new();
        state.advance_animation();
        let start = state.animation_tick;
        state.skip_animation(Duration::from_millis(crate::config::ANIMATION_STEP_MS));
        assert_eq!(state.animation_tick, start + 1);
    }

    #[test]
    fn the_animation_clock_runs_in_milliseconds_too() {
        let mut state = AppState::new();
        state.skip_animation(Duration::from_millis(1_500));
        assert!(state.animation_ms >= 1_500);
        assert_eq!(
            state.animation_tick,
            (state.animation_ms / crate::config::ANIMATION_STEP_MS) as usize,
            "the two readings of the clock must agree"
        );
    }
}
