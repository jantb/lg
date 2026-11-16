//! Running what the GitHub modal asks for.
//!
//! Each action is an ordinary operation: it runs off the UI thread, shows in
//! the footer while it does, and reports how it went in the status line. The
//! ones that only change something on GitHub leave the modal open, and the
//! list it shows is read again once they finish.

use std::path::{Path, PathBuf};

use crate::state::{AppState, GitHubAction, OperationKind};

use super::{App, spawn_operation};

impl App {
    pub(super) fn run_github_action(&mut self, action: GitHubAction) {
        let state = &mut self.state;
        match action {
            GitHubAction::Checkout { number } => spawn_operation(
                state,
                "checking out pull request",
                OperationKind::WorkingTree,
                move || crate::github::checkout(number),
            ),
            GitHubAction::CheckoutWorktree { number, path } => spawn_operation(
                state,
                "checking out pull request",
                OperationKind::WorkingTree,
                move || crate::github::checkout_in_worktree(number, Path::new(&path)),
            ),
            GitHubAction::Review {
                number,
                verdict,
                body,
            } => {
                let label = match verdict {
                    crate::github::Verdict::Approve => "approving",
                    crate::github::Verdict::RequestChanges => "requesting changes",
                };
                spawn_operation(state, label, OperationKind::GitHub, move || {
                    crate::github::review(number, verdict, &body)
                });
            }
            GitHubAction::SubmitReview {
                number,
                commit,
                event,
                body,
                comments,
            } => {
                spawn_operation(
                    state,
                    "submitting review",
                    OperationKind::SubmitReview,
                    move || crate::github::submit_review(number, &commit, event, &body, &comments),
                );
            }
            GitHubAction::Comment { number, body } => {
                spawn_operation(state, "commenting", OperationKind::GitHub, move || {
                    crate::github::comment(number, &body)
                });
            }
            GitHubAction::Merge {
                number,
                method,
                delete_branch,
                auto,
            } => spawn_operation(
                state,
                "merging pull request",
                OperationKind::GitHub,
                move || crate::github::merge(number, method, delete_branch, auto),
            ),
            GitHubAction::SetDraft { number, draft } => {
                let label = if draft {
                    "converting to draft"
                } else {
                    "marking ready for review"
                };
                spawn_operation(state, label, OperationKind::GitHub, move || {
                    crate::github::set_draft(number, draft)
                });
            }
            GitHubAction::Close { number } => spawn_operation(
                state,
                "closing pull request",
                OperationKind::GitHub,
                move || crate::github::close(number),
            ),
            GitHubAction::Reopen { number } => spawn_operation(
                state,
                "reopening pull request",
                OperationKind::GitHub,
                move || crate::github::reopen(number),
            ),
            GitHubAction::Create(pr) => {
                let remote = crate::preferences::remote();
                spawn_operation(
                    state,
                    "opening pull request",
                    OperationKind::OpenPullRequest,
                    move || crate::github::create(&remote, &pr),
                );
            }
            GitHubAction::Clone { repo, dir } => start_clone(state, repo, dir),
            GitHubAction::OpenInBrowser { url } => match crate::github::open_in_browser(&url) {
                Ok(()) => state.set_status(format!("opened {url}"), false),
                Err(err) => state.set_status(format!("{err:#}"), true),
            },
            GitHubAction::ReviewInCheckout { dir, number } => {
                if crate::git::active_repo().as_deref() != Some(dir.as_path()) {
                    let label = dir
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| dir.to_string_lossy().into_owned());
                    self.switch_to_repository(&dir, &label);
                }
                crate::panel::github::inbox::start_review(&mut self.state, number);
            }
        }
    }

    /// Follow up on a GitHub operation that has just ended.
    pub(super) fn after_github_operation(&mut self, kind: OperationKind, succeeded: bool) {
        match kind {
            OperationKind::GitHub | OperationKind::OpenPullRequest => {
                crate::panel::github::after_action(&mut self.state)
            }
            OperationKind::SubmitReview => {
                crate::panel::guided::after_submit(&mut self.state, succeeded);
                crate::panel::github::after_action(&mut self.state);
            }
            OperationKind::Clone => {
                let Some(dir) = self.state.github.clone_target.take() else {
                    return;
                };
                if !succeeded {
                    return;
                }
                let status = self.state.status.clone();
                let label = dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dir.to_string_lossy().into_owned());
                self.switch_to_repository(&dir, &label);
                // The clone's own report says more than "selected <name>".
                if let Some(status) = status.filter(|status| !status.is_error) {
                    self.state.set_status(status.text, false);
                }
            }
            _ => {}
        }
    }
}

/// Start cloning `repo` into `dir`, which lg switches to once the clone lands.
fn start_clone(state: &mut AppState, repo: String, dir: String) {
    let target = PathBuf::from(&dir);
    let operation_was_running = state.operation_job.is_some();
    spawn_operation(state, "cloning", OperationKind::Clone, move || {
        crate::github::clone(&repo, Path::new(&dir))
    });
    // Only remembered once this clone is actually under way; a blocked one
    // must not send lg somewhere later, nor redirect a clone already running.
    if !operation_was_running && state.operation_job.is_some() {
        state.github.clone_target = Some(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::OperationJob;

    #[test]
    fn a_clone_asked_for_while_another_runs_leaves_the_running_one_where_it_lands() {
        let mut state = AppState::new();
        let (_tx, rx) = std::sync::mpsc::channel();
        state.operation_job = Some(OperationJob {
            rx,
            handle: None,
            spinner: 0,
            label: "cloning",
            kind: OperationKind::Clone,
            step: None,
        });
        let first = PathBuf::from("/tmp/lg-test-first-clone");
        state.github.clone_target = Some(first.clone());

        start_clone(
            &mut state,
            "owner/second".into(),
            "/tmp/lg-test-second-clone".into(),
        );

        assert_eq!(state.github.clone_target, Some(first));
    }
}
