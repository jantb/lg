//! What the app is doing right now: running jobs, spawned threads, the status line.

use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use chrono::{DateTime, Utc};

use super::{AppState, BackgroundJob, GenMsg, Generation, PendingAction};

#[derive(Debug, Clone)]
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
        take!(
            generation,
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
        self.generation.is_some()
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
        if self.generation.is_some() {
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
        } else {
            match &self.pending_action {
                Some(PendingAction::GenerateMessage) => Some("starting generator"),
                Some(PendingAction::ReviewAssist(_)) => Some("starting explanation"),
                Some(PendingAction::ReviewPrText) => Some("starting PR text"),
                Some(PendingAction::ReviewStyleFlags) => Some("starting style flag pass"),
                Some(PendingAction::ReviewChat(_)) => Some("starting chat"),
                Some(PendingAction::CopyToClipboard { .. }) => Some("copying"),
                Some(PendingAction::Commit) => Some("committing"),
                Some(PendingAction::StageAllAndCommit) => Some("staging"),
                Some(PendingAction::Push) => Some("starting push"),
                Some(PendingAction::Pull) => Some("starting pull"),
                Some(PendingAction::MergeUpstream) => Some("starting merge"),
                Some(PendingAction::MergeMainAllBranches) => Some("starting branch sync"),
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
                Some(PendingAction::CreateWorktree { .. }) => Some("adding worktree"),
                Some(PendingAction::RemoveWorktree { .. }) => Some("removing worktree"),
                Some(PendingAction::LandWorktree { .. }) => Some("landing worktree"),
                Some(PendingAction::SyncWorktree { .. }) => Some("syncing worktree"),
                Some(PendingAction::BringWorktreeHome { .. }) => Some("moving branch home"),
                Some(PendingAction::PruneWorktrees) => Some("pruning worktrees"),
                Some(PendingAction::StartSession { .. }) => Some("starting session"),
                Some(PendingAction::Quit) => Some("quitting"),
                None => None,
            }
        }
    }

    pub fn start_generation(&mut self, rx: Receiver<GenMsg>, handle: JoinHandle<()>) {
        self.generation = Some(Generation {
            rx,
            handle: Some(handle),
            output: String::new(),
            spinner: 0,
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
        if self.generation.is_some() {
            self.cancel_generation();
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
            || self.generation.is_some()
    }

    pub fn cancel_generation(&mut self) {
        if let Some(mut generation) = self.generation.take() {
            self.defer_thread_join(generation.handle.take());
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status = Some(StatusMsg {
            text: text.into(),
            is_error,
            at: Utc::now(),
        });
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
