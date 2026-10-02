use anyhow::{Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::state::{AppState, Modal, OperationKind, PendingAction};

use super::{
    App, spawn_force_push, spawn_operation, spawn_operation_with_progress, spawn_pull, spawn_push,
    spawn_review_assist, spawn_review_chat, spawn_review_pr_text, spawn_review_style_flags,
};

/// Saves the LLM choice and this checkout's settings together, so the settings
/// modal's single save either lands fully or reports why it did not.
fn save_settings(
    model: &str,
    provider: crate::llm::LlmProvider,
    pr_language: &str,
    comment_style: &str,
    commit_subject_max_chars: Option<usize>,
    commit_body_max_lines: Option<usize>,
) -> Result<()> {
    // With claude answering, the field shows the Claude model, which is
    // chosen in Settings; saving it here would overwrite the local model.
    if provider != crate::llm::LlmProvider::Claude {
        crate::llm::save_llm_settings(model, provider)?;
    }
    // The footer names the model that last answered over the one configured.
    // That was right until now; the next answer will come from this one.
    crate::llm::forget_last_stats();
    let current = crate::settings::load();
    crate::settings::save(&crate::settings::RepoSettings {
        pr_language: pr_language.trim().to_string(),
        comment_style: comment_style.trim().to_string(),
        commit_subject_max_chars: commit_subject_max_chars
            .unwrap_or(current.commit_subject_max_chars),
        commit_body_max_lines: commit_body_max_lines.unwrap_or(current.commit_body_max_lines),
        commit_prompt: current.commit_prompt,
        review_style: current.review_style,
    })
}

fn refresh_llm_settings_state(state: &mut AppState) {
    state.llm_model = crate::llm::current_model();
    state.llm_provider = crate::llm::current_provider();
    state.llm_provider_idx = crate::llm::LlmProvider::ALL
        .iter()
        .position(|provider| *provider == state.llm_provider)
        .unwrap_or(0);
    state.llm_config_path = crate::llm::config_file_display();
    // The server may be somewhere else now; let the commit modal ask it.
    state.model_server_unreachable = false;
}

/// The actions that send a request to the model server.
fn asks_the_model(action: &PendingAction) -> bool {
    matches!(
        action,
        PendingAction::GenerateMessage
            | PendingAction::ReviewAssist(_)
            | PendingAction::ReviewPrText
            | PendingAction::ReviewStyleFlags
            | PendingAction::ReviewChat(_)
    )
}

impl App {
    pub(super) fn dispatch_pending(&mut self, action: PendingAction) {
        if !self.state.ai_assist && asks_the_model(&action) {
            self.state
                .set_status(crate::panel::commit::AI_OFF_NOTICE, false);
            return;
        }
        match action {
            PendingAction::GenerateMessage => self.generate_message(),
            PendingAction::ReviewAssist(node_id) => spawn_review_assist(&mut self.state, node_id),
            PendingAction::ReviewPrText => spawn_review_pr_text(&mut self.state),
            PendingAction::ReviewStyleFlags => spawn_review_style_flags(&mut self.state),
            PendingAction::ReviewAgent => self.start_review_agent(),
            PendingAction::ReviewChat(prompt) => spawn_review_chat(&mut self.state, prompt),

            PendingAction::Commit => self.commit(),
            PendingAction::AmendCommit => self.amend(),
            PendingAction::ApplyHunk { op, hunk } => self.apply_hunk(op, *hunk),
            PendingAction::RevertCommit { sha } => self.replay_commit("reverting", move || {
                Ok(last_line_or(&crate::git::revert_commit(&sha)?, "reverted"))
            }),
            PendingAction::CherryPick { sha } => self.replay_commit("cherry-picking", move || {
                Ok(last_line_or(
                    &crate::git::cherry_pick_commit(&sha)?,
                    "cherry-picked",
                ))
            }),
            PendingAction::StashPush { message } => {
                self.stash_operation("stashing", move || crate::git::stash_push(&message))
            }
            PendingAction::StashApply { sha } => {
                self.stash_operation("applying stash", move || crate::git::stash_apply(&sha))
            }
            PendingAction::StashPop { sha } => {
                self.stash_operation("popping stash", move || crate::git::stash_pop(&sha))
            }
            PendingAction::StashDrop { sha } => {
                // Dropping is confirmed from the stash list, and the list is
                // where the reader goes on from.
                self.state.modal = Modal::Stash;
                self.stash_operation("dropping stash", move || crate::git::stash_drop(&sha))
            }
            PendingAction::StageAllAndCommit => spawn_operation(
                &mut self.state,
                "staging",
                OperationKind::StageAllAndCommit,
                || {
                    crate::git::stage_all()?;
                    Ok("staged all".to_string())
                },
            ),
            PendingAction::StageAll => self.index_operation("staging", || {
                crate::git::stage_all()?;
                Ok("staged all".to_string())
            }),
            PendingAction::UnstageAll => self.index_operation("unstaging", || {
                crate::git::unstage_all()?;
                Ok("unstaged all".to_string())
            }),
            PendingAction::StagePath(path) => self.index_operation("staging", move || {
                crate::git::stage(&path)?;
                Ok(format!("staged {path}"))
            }),
            PendingAction::UnstagePath(path) => self.index_operation("unstaging", move || {
                crate::git::unstage(&path)?;
                Ok(format!("unstaged {path}"))
            }),
            PendingAction::RollbackPath { path, is_dir } => spawn_operation(
                &mut self.state,
                "rolling back",
                OperationKind::FileSystem,
                move || {
                    crate::git::rollback_worktree_path(&path)?;
                    let label = if is_dir { "folder" } else { "file" };
                    Ok(format!("rolled back {label} {path}"))
                },
            ),
            PendingAction::DeletePath { path, is_dir } => spawn_operation(
                &mut self.state,
                "deleting",
                OperationKind::FileSystem,
                move || {
                    crate::git::delete_worktree_path(&path, is_dir)?;
                    Ok(format!("deleted {path}"))
                },
            ),
            PendingAction::IgnorePath { path, is_dir } => self.settle(
                crate::git::add_to_gitignore(&path, is_dir),
                "gitignore update failed",
                |app, status| {
                    app.state.set_status(status, false);
                    app.start_refresh_with_status(false, false);
                },
            ),

            PendingAction::Push => spawn_push(&mut self.state),
            PendingAction::ForcePushWithLease(plan) => spawn_force_push(&mut self.state, *plan),
            PendingAction::Pull => spawn_pull(&mut self.state),
            PendingAction::MergeUpstream => self.merge_upstream(),
            PendingAction::MergeMainAllBranches => self.merge_main_into_all_branches(),
            PendingAction::Flow(action) => super::run_flow_action(&mut self.state, action, None),
            PendingAction::DeleteBranch {
                name,
                delete_local,
                delete_remote,
                force,
            } => self.delete_branch(name, delete_local, delete_remote, force),
            PendingAction::SetBranchUpstream { branch, upstream } => spawn_operation(
                &mut self.state,
                "setting upstream",
                OperationKind::WorkingTree,
                move || crate::git::set_branch_upstream(&branch, &upstream),
            ),
            PendingAction::Promote(preview) => spawn_operation(
                &mut self.state,
                "promoting",
                OperationKind::WorkingTree,
                move || crate::git::environments::promote(&preview),
            ),

            PendingAction::SaveAuthor { name, email } => self.change_author(
                crate::git::set_local_author(&name, &email),
                "saved repo author",
                "author save failed",
                |state| state.author.has_local_override = true,
            ),
            PendingAction::ClearAuthor => self.change_author(
                crate::git::clear_local_author(),
                "cleared repo author",
                "author clear failed",
                |state| state.author.has_local_override = false,
            ),
            PendingAction::SaveSubtreeAuthor { path, name, email } => self.change_author(
                crate::git::set_subtree_author(&path, &name, &email),
                "saved subtree author",
                "author save failed",
                |state| state.author.has_subtree_rule = true,
            ),
            PendingAction::ClearSubtreeAuthor { path } => self.change_author(
                crate::git::clear_subtree_author(&path),
                "cleared subtree author",
                "author clear failed",
                |state| state.author.has_subtree_rule = false,
            ),

            PendingAction::SaveSettings {
                model,
                provider,
                pr_language,
                comment_style,
                commit_subject_max_chars,
                commit_body_max_lines,
            } => self.settle(
                save_settings(
                    &model,
                    provider,
                    &pr_language,
                    &comment_style,
                    commit_subject_max_chars,
                    commit_body_max_lines,
                ),
                "settings save failed",
                |app, ()| {
                    app.reload_settings_state();
                    app.state.modal = Modal::None;
                    if crate::llm::env_model_active() || crate::llm::env_provider_active() {
                        app.state
                            .set_status("saved settings; env override is active", false);
                    } else {
                        app.state.set_status("saved settings", false);
                    }
                },
            ),
            PendingAction::ClearSettings => self.settle(
                crate::llm::clear_saved_llm_settings().and_then(|()| crate::settings::clear()),
                "settings reset failed",
                |app, ()| {
                    app.reload_settings_state();
                    // The modal stays open so the re-detected conventions land
                    // in front of the user instead of behind a closed dialog.
                    super::spawn::suggest_repo_settings_if_unset(&mut app.state);
                    app.state.set_status("reset settings to defaults", false);
                },
            ),
            PendingAction::EditCommitPrompt => self.edit_settings_file(
                "commit prompt",
                crate::settings::ensure_commit_prompt_file(),
            ),
            PendingAction::EditReviewStyle => {
                self.edit_settings_file("review style", crate::settings::ensure_review_style_file())
            }

            PendingAction::OpenProject => self.report_open(crate::git::open_project_in_ide()),
            PendingAction::OpenProjectAt(path) => {
                self.report_open(crate::git::open_project_path_in_ide(&PathBuf::from(path)))
            }
            PendingAction::OpenFile(path) => self.report_open(crate::git::open_file_in_ide(&path)),
            PendingAction::EditFile { path, line } => self.edit_file(&path, line),
            PendingAction::CopyToClipboard { label, text } => {
                self.settle(copy_to_clipboard(&text), "copy failed", |app, ()| {
                    app.state
                        .set_status(format!("copied {label} to clipboard"), false)
                })
            }

            PendingAction::CreateWorktree { path, branch, base } => {
                self.worktree_operation("adding worktree", "worktree added", move || {
                    crate::git::worktree_add(Path::new(&path), &branch, &base)
                })
            }
            PendingAction::RemoveWorktree { path, force } => {
                self.leave_checkout_before_removal(&path);
                self.worktree_operation("removing worktree", "worktree removed", move || {
                    crate::git::worktree_remove(Path::new(&path), force)
                });
            }
            PendingAction::LandWorktree { path, branch } => {
                self.leave_checkout_before_removal(&path);
                spawn_operation_with_progress(
                    &mut self.state,
                    "landing worktree",
                    OperationKind::WorkingTree,
                    move |progress| {
                        crate::git::worktree_land_with_progress(Path::new(&path), &branch, progress)
                    },
                );
            }
            PendingAction::SyncWorktree { path, branch } => {
                // A conflict is resolved where it happened, and the conflict
                // modal reads whichever checkout lg is pointed at, so the
                // worktree has to be that checkout before the merge starts.
                self.enter_checkout_before_merge(&path);
                spawn_operation_with_progress(
                    &mut self.state,
                    "syncing worktree",
                    OperationKind::WorkingTree,
                    move |progress| {
                        crate::git::worktree_sync_main_with_progress(
                            Path::new(&path),
                            &branch,
                            progress,
                        )
                    },
                );
            }
            PendingAction::BringWorktreeHome { path, branch } => {
                self.leave_checkout_before_removal(&path);
                spawn_operation(
                    &mut self.state,
                    "moving branch home",
                    OperationKind::WorkingTree,
                    move || crate::git::worktree_bring_home(Path::new(&path), &branch),
                );
            }
            PendingAction::PruneWorktrees => {
                self.worktree_operation("pruning worktrees", "pruned", crate::git::worktree_prune)
            }

            PendingAction::StartAgent {
                path,
                label,
                profile,
            } => self.start_agent(&path, &label, &profile),
            PendingAction::StartSession {
                path,
                label,
                sandboxed,
                kind,
                prompt,
            } => self.start_session(&path, label, sandboxed, kind, prompt),

            PendingAction::InitRepository { path } => self.init_repository(&path),
            PendingAction::SwitchRepository { target } => self.switch_repository(&target),
            PendingAction::GitHub(action) => self.run_github_action(action),
            PendingAction::Quit => self.state.should_quit = true,
            PendingAction::CloseSession { id, follow } => {
                crate::panel::environments::close_session(&mut self.state, id, follow)
            }
        }
    }

    /// What became of an action that ran here and now: `done` with its
    /// result, or a status line saying it failed and why.
    fn settle<T>(&mut self, result: Result<T>, failed: &str, done: impl FnOnce(&mut Self, T)) {
        match result {
            Ok(value) => done(self, value),
            Err(err) => self.state.set_status(format!("{failed}: {err}"), true),
        }
    }

    fn report_open(&mut self, result: Result<String>) {
        self.settle(result, "open failed", |app, status| {
            app.state.set_status(status, false)
        });
    }
}

/// The model.
impl App {
    fn generate_message(&mut self) {
        match crate::git::staged_diff() {
            Ok(diff) => {
                let (tx, rx) = std::sync::mpsc::channel();
                // The scene shows the very diff that went to the model,
                // streaming into the network a character at a time.
                let feed = crate::panel::commit_art::Feed::from_diff(&diff);
                let handle = std::thread::spawn(move || {
                    crate::llm::stream_commit_message(diff, tx);
                });
                self.state.start_generation(rx, handle, feed);
                self.state.set_status("generating\u{2026}", false);
            }
            Err(e) => {
                self.state.set_status(e.to_string(), true);
            }
        }
    }
}

/// The index and the working tree.
impl App {
    fn commit(&mut self) {
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

    fn amend(&mut self) {
        let msg = self.state.commit_message.clone();
        spawn_operation(
            &mut self.state,
            "amending",
            OperationKind::Commit,
            move || {
                let out = crate::git::amend_commit(&msg)?;
                Ok(out.lines().next().unwrap_or("amended").to_owned())
            },
        );
    }

    /// Stage, unstage or discard one hunk. Discarding writes the file, so it
    /// waits for the working tree like a rollback does; the other two only
    /// touch the index.
    fn apply_hunk(&mut self, op: crate::git::hunk::HunkOp, hunk: crate::git::hunk::ShownHunk) {
        use crate::git::hunk::HunkOp;
        let (label, kind) = match op {
            HunkOp::Stage => ("staging hunk", OperationKind::Index),
            HunkOp::Unstage => ("unstaging hunk", OperationKind::Index),
            HunkOp::Discard => ("discarding hunk", OperationKind::FileSystem),
        };
        spawn_operation(&mut self.state, label, kind, move || {
            crate::git::hunk::apply_hunk(op, &hunk)
        });
    }

    /// Revert or cherry-pick one commit. A conflict opens the conflict modal,
    /// as a merge's does, with the operation left in progress for `v` to
    /// continue or `a` to abort.
    fn replay_commit<F>(&mut self, label: &'static str, work: F)
    where
        F: FnOnce() -> Result<String> + Send + 'static,
    {
        spawn_operation(&mut self.state, label, OperationKind::WorkingTree, work);
    }

    fn stash_operation<F>(&mut self, label: &'static str, work: F)
    where
        F: FnOnce() -> Result<String> + Send + 'static,
    {
        spawn_operation(&mut self.state, label, OperationKind::Stash, work);
    }

    fn index_operation<F>(&mut self, label: &'static str, work: F)
    where
        F: FnOnce() -> Result<String> + Send + 'static,
    {
        spawn_operation(&mut self.state, label, OperationKind::Index, work);
    }
}

/// Branches, the remote and promotions.
impl App {
    fn merge_upstream(&mut self) {
        spawn_operation(
            &mut self.state,
            "merging",
            OperationKind::MergeUpstream,
            || {
                Ok(last_line_or(
                    &crate::git::merge_upstream()?,
                    "merged upstream",
                ))
            },
        );
    }

    fn merge_main_into_all_branches(&mut self) {
        // Where the sync starts is read from git rather than the state,
        // because a resumed sync queues itself before the refresh that would
        // notice the validation checked another branch out.
        self.state.conflict.followup = Some(crate::state::ConflictFollowup::for_branch_sync(
            crate::git::head_branch().ok(),
        ));
        spawn_operation(
            &mut self.state,
            "syncing branches",
            OperationKind::WorkingTree,
            crate::git::flow_merge_main_into_all_local_branches,
        );
    }

    fn delete_branch(&mut self, name: String, local: bool, remote: bool, force: bool) {
        self.state.modal = Modal::None;
        spawn_operation(
            &mut self.state,
            "deleting branch",
            OperationKind::WorkingTree,
            move || {
                let mut report = Vec::new();
                if local {
                    let line = crate::git::delete_local_branch(&name, force)?;
                    report.push(format!("local: {line}"));
                }
                if remote {
                    let line = crate::git::delete_remote_branch(&name)?;
                    report.push(format!("remote: {line}"));
                }
                Ok(report.join(" | "))
            },
        );
    }
}

/// The commit author.
impl App {
    /// Report an author change made here and now, and on success record what
    /// it left in place with `record` and close the author modal.
    fn change_author(
        &mut self,
        result: Result<()>,
        done: &str,
        failed: &str,
        record: impl FnOnce(&mut AppState),
    ) {
        self.settle(result, failed, |app, ()| {
            record(&mut app.state);
            app.state.modal = Modal::None;
            app.state.set_status(done, false);
        });
    }
}

/// Settings.
impl App {
    /// Read back everything the settings modal shows after it was saved or
    /// reset.
    fn reload_settings_state(&mut self) {
        refresh_llm_settings_state(&mut self.state);
        super::spawn::load_repo_settings_into_state(&mut self.state);
        self.state.llm_model_input = self.state.llm_model.clone();
    }
}

/// Opening and editing files.
impl App {
    fn edit_file(&mut self, path: &str, line: usize) {
        match self.edit_in_terminal(path, line) {
            Ok(()) => self.state.set_status(format!("edited {path}"), false),
            Err(err) => self.state.set_status(format!("edit failed: {err:#}"), true),
        }
        // Whatever the editor did is part of the change now.
        crate::panel::guided::reload(&mut self.state);
        self.start_refresh(false);
    }
}

/// Worktrees.
impl App {
    /// Run a worktree command in the background and report the last line it
    /// printed, or `fallback` when it printed nothing.
    fn worktree_operation<F>(&mut self, label: &'static str, fallback: &'static str, work: F)
    where
        F: FnOnce() -> Result<String> + Send + 'static,
    {
        spawn_operation(
            &mut self.state,
            label,
            OperationKind::WorkingTree,
            move || Ok(last_line_or(&work()?, fallback)),
        );
    }
}

/// Sessions and repositories.
impl App {
    fn start_agent(&mut self, path: &str, label: &str, profile: &crate::preferences::Agent) {
        let cwd = PathBuf::from(path);
        let sandboxed = profile.sandboxed();
        let result = (|| {
            if sandboxed {
                prepare_sandbox(&cwd)?;
            }
            let spec = crate::session::SessionSpec {
                cwd,
                label: format!("{label} · {}", profile.name),
                sandboxed,
                kind: crate::agents::kind(profile),
                prompt: None,
            };
            self.state
                .sessions
                .start_profile(spec, profile, crate::session::default_size())
        })();
        match result {
            Ok(id) => {
                self.state.show_session(id);
                self.set_session_capture(true);
            }
            Err(e) => self
                .state
                .set_status(format!("start agent failed: {e:#}"), true),
        }
    }

    fn start_session(
        &mut self,
        path: &str,
        label: String,
        sandboxed: bool,
        kind: crate::session::SessionKind,
        prompt: Option<String>,
    ) {
        let cwd = PathBuf::from(path);
        // A terminal is the user's own shell and is never confined.
        let sandboxed = sandboxed && kind != crate::session::SessionKind::Terminal;
        if sandboxed {
            match prepare_sandbox(&cwd) {
                Ok(Some(note)) => self.state.set_status(note, false),
                Ok(None) => {}
                Err(err) => {
                    // Running unsandboxed instead would quietly hand the
                    // session the whole filesystem, which is the opposite of
                    // what was asked for.
                    self.state
                        .set_status(format!("sandbox setup failed: {err:#}"), true);
                    return;
                }
            }
        }
        let spec = crate::session::SessionSpec {
            label,
            cwd,
            sandboxed,
            kind,
            prompt,
        };
        let size = crate::session::default_size();
        match self.state.sessions.start(spec, size) {
            Ok(id) => {
                self.state.show_session(id);
                self.set_session_capture(true);
                let label = self
                    .state
                    .sessions
                    .get(id)
                    .map(|session| session.label.clone())
                    .unwrap_or_default();
                self.state
                    .set_status(format!("{} for {label}", kind.label()), false);
            }
            Err(err) => self
                .state
                .set_status(format!("start session failed: {err}"), true),
        }
    }

    fn init_repository(&mut self, path: &str) {
        let dir = PathBuf::from(path);
        let label = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        match crate::git::init_repository(&dir) {
            Ok(message) => {
                // Point lg at what it just created: the folder is a checkout
                // now, and every panel was showing it as a directory with no
                // history a moment ago.
                self.switch_to_repository(&dir, &label);
                self.state.set_status(message, false);
            }
            Err(err) => self.state.set_status(err.to_string(), true),
        }
    }

    fn switch_repository(&mut self, target: &crate::state::RepoTarget) {
        let root = self
            .state
            .workspace_root
            .clone()
            .or_else(|| self.state.repo_root.clone())
            .unwrap_or_default();
        if root.is_empty() {
            self.state.set_status("workspace root is unknown", true);
            return;
        }
        let dir = target.resolve(Path::new(&root));
        if !dir.is_dir() {
            self.state
                .set_status(format!("{} is not a directory", dir.display()), true);
            return;
        }
        self.switch_to_repository(&dir, &target.label());
    }
}

/// The last line of `out` with anything in it, or `fallback`.
fn last_line_or(out: &str, fallback: &str) -> String {
    out.lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

impl App {
    /// Open one of this checkout's editable settings files in the IDE.
    ///
    /// The editor opens beside the modal, which stays up so the rest of the
    /// settings are still there to save afterwards; the file is re-read on the
    /// way so the modal's "custom" markers reflect what was just written.
    fn edit_settings_file(&mut self, what: &str, path: anyhow::Result<std::path::PathBuf>) {
        let opened = path.and_then(|path| {
            let path = path.to_string_lossy().into_owned();
            crate::git::open_file_in_ide(&path)
        });
        match opened {
            Ok(status) => {
                super::spawn::load_repo_settings_into_state(&mut self.state);
                self.state.set_status(status, false);
            }
            Err(err) => self
                .state
                .set_status(format!("{what} open failed: {err}"), true),
        }
    }

    /// Move lg off a checkout that is about to be removed, so the panels are
    /// not left pointing at a directory that no longer exists. The main
    /// worktree is where every repository has one.
    /// Point lg at the checkout a merge is about to run in, so the conflict
    /// modal and everything it drives — the file list, staging, validation —
    /// act on the checkout holding the conflict.
    fn enter_checkout_before_merge(&mut self, path: &str) {
        let already_there = self
            .state
            .repo_root
            .as_deref()
            .is_some_and(|root| crate::git::same_dir(Path::new(root), Path::new(path)));
        if already_there {
            return;
        }
        let dir = PathBuf::from(path);
        let label = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        self.switch_to_repository(&dir, &label);
    }

    fn leave_checkout_before_removal(&mut self, path: &str) {
        let showing_it = self
            .state
            .repo_root
            .as_deref()
            .is_some_and(|root| crate::git::same_dir(Path::new(root), Path::new(path)));
        if !showing_it {
            return;
        }
        let Some(main) = self
            .state
            .worktrees
            .iter()
            .find(|worktree| worktree.is_main)
            .map(|worktree| PathBuf::from(&worktree.path))
        else {
            return;
        };
        let label = main
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| main.to_string_lossy().into_owned());
        self.switch_to_repository(&main, &label);
    }
}

/// Give `cwd` a terrarium profile confined to it, deriving one from the
/// repository's own profile when the checkout is a worktree. Returns a note
/// worth showing when something was written.
pub(super) fn prepare_sandbox(cwd: &Path) -> Result<Option<String>> {
    let (main_worktree, git_dir) = crate::git::with_repo(cwd, || {
        Ok::<_, anyhow::Error>((crate::git::main_worktree()?, crate::git::common_git_dir()?))
    })?;
    crate::terrarium::ensure_profile(cwd, &main_worktree, &git_dir)
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        copy_with_command("pbcopy", &[], text)
    }

    #[cfg(target_os = "windows")]
    {
        copy_with_command("clip", &[], text)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let attempts: &[(&str, &[&str])] = &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];
        let mut errors = Vec::new();
        for (program, args) in attempts {
            match copy_with_command(program, args, text) {
                Ok(()) => return Ok(()),
                Err(err) => errors.push(format!("{program}: {err:#}")),
            }
        }
        anyhow::bail!("no clipboard command succeeded ({})", errors.join("; "))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    {
        let _ = text;
        anyhow::bail!("clipboard copy is not supported on this platform")
    }
}

fn copy_with_command(program: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to launch {program}"))?;
    let mut stdin = child
        .stdin
        .take()
        .with_context(|| format!("{program} did not open stdin"))?;
    stdin
        .write_all(text.as_bytes())
        .with_context(|| format!("failed writing to {program}"))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for {program}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr.trim();
    if message.is_empty() {
        anyhow::bail!("{program} exited with {}", output.status)
    } else {
        anyhow::bail!("{program} exited with {}: {message}", output.status)
    }
}
