use anyhow::{Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::state::{AppState, Modal, OperationKind, PendingAction};

use super::{
    App, spawn_operation, spawn_pull, spawn_push, spawn_review_assist, spawn_review_chat,
    spawn_review_pr_text, spawn_review_style_flags,
};

/// Saves the LLM choice and this checkout's settings together, so the settings
/// modal's single save either lands fully or reports why it did not.
fn save_settings(
    model: &str,
    provider: crate::llm::LlmProvider,
    pr_language: &str,
    comment_style: &str,
    commit_subject_max_chars: &str,
    commit_body_max_lines: &str,
) -> Result<()> {
    crate::llm::save_llm_settings(model, provider)?;
    let current = crate::settings::load();
    crate::settings::save(&crate::settings::RepoSettings {
        pr_language: pr_language.trim().to_string(),
        comment_style: comment_style.trim().to_string(),
        commit_subject_max_chars: parse_limit(
            commit_subject_max_chars,
            current.commit_subject_max_chars,
        ),
        commit_body_max_lines: parse_limit(commit_body_max_lines, current.commit_body_max_lines),
        commit_prompt: current.commit_prompt,
    })
}

/// An empty limit field means "unlimited"; anything unparsable keeps the value
/// already stored rather than silently resetting it.
fn parse_limit(value: &str, fallback: usize) -> usize {
    let value = value.trim();
    if value.is_empty() {
        return 0;
    }
    value.parse::<usize>().unwrap_or(fallback)
}

fn refresh_llm_settings_state(state: &mut AppState) {
    state.llm_model = crate::llm::current_model();
    state.llm_provider = crate::llm::current_provider();
    state.llm_provider_idx = crate::llm::LlmProvider::ALL
        .iter()
        .position(|provider| *provider == state.llm_provider)
        .unwrap_or(0);
    state.llm_config_path = crate::llm::config_file_display();
}

impl App {
    pub(super) fn dispatch_pending(&mut self, action: PendingAction) {
        match action {
            PendingAction::GenerateMessage => match crate::git::staged_diff() {
                Ok(diff) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let handle = std::thread::spawn(move || {
                        crate::llm::stream_commit_message(diff, tx);
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
            PendingAction::ReviewPrText => {
                spawn_review_pr_text(&mut self.state);
            }
            PendingAction::ReviewStyleFlags => {
                spawn_review_style_flags(&mut self.state);
            }
            PendingAction::ReviewChat(prompt) => {
                spawn_review_chat(&mut self.state, prompt);
            }
            PendingAction::CopyToClipboard { label, text } => match copy_to_clipboard(&text) {
                Ok(()) => self
                    .state
                    .set_status(format!("copied {label} to clipboard"), false),
                Err(err) => self.state.set_status(format!("copy failed: {err}"), true),
            },
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
            PendingAction::MergeUpstream => {
                spawn_operation(
                    &mut self.state,
                    "merging",
                    OperationKind::MergeUpstream,
                    || {
                        let out = crate::git::merge_upstream()?;
                        Ok(out
                            .lines()
                            .rfind(|line| !line.trim().is_empty())
                            .unwrap_or("merged upstream")
                            .to_owned())
                    },
                );
            }
            PendingAction::MergeMainAllBranches => {
                spawn_operation(
                    &mut self.state,
                    "syncing branches",
                    OperationKind::WorkingTree,
                    crate::git::flow_merge_main_into_all_local_branches,
                );
            }
            PendingAction::Flow(action) => {
                super::run_flow_action(&mut self.state, action, None);
            }
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
            PendingAction::SaveSettings {
                model,
                provider,
                pr_language,
                comment_style,
                commit_subject_max_chars,
                commit_body_max_lines,
            } => match save_settings(
                &model,
                provider,
                &pr_language,
                &comment_style,
                &commit_subject_max_chars,
                &commit_body_max_lines,
            ) {
                Ok(()) => {
                    refresh_llm_settings_state(&mut self.state);
                    super::spawn::load_repo_settings_into_state(&mut self.state);
                    self.state.llm_model_input = self.state.llm_model.clone();
                    self.state.modal = Modal::None;
                    if crate::llm::env_model_active() || crate::llm::env_provider_active() {
                        self.state
                            .set_status("saved settings; env override is active", false);
                    } else {
                        self.state.set_status("saved settings", false);
                    }
                }
                Err(err) => self
                    .state
                    .set_status(format!("settings save failed: {err}"), true),
            },
            PendingAction::ClearSettings => {
                match crate::llm::clear_saved_llm_settings().and_then(|()| crate::settings::clear())
                {
                    Ok(()) => {
                        refresh_llm_settings_state(&mut self.state);
                        super::spawn::load_repo_settings_into_state(&mut self.state);
                        self.state.llm_model_input = self.state.llm_model.clone();
                        // The modal stays open so the re-detected conventions land
                        // in front of the user instead of behind a closed dialog.
                        super::spawn::suggest_repo_settings_if_unset(&mut self.state);
                        self.state.set_status("reset settings to defaults", false);
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("settings reset failed: {err}"), true),
                }
            }
            PendingAction::EditCommitPrompt => match crate::settings::ensure_commit_prompt_file() {
                Ok(path) => {
                    let path = path.to_string_lossy().into_owned();
                    match crate::git::open_file_in_ide(&path) {
                        Ok(status) => {
                            // The editor opens beside the modal, which stays up so the
                            // rest of the settings are still there to save afterwards.
                            super::spawn::load_repo_settings_into_state(&mut self.state);
                            self.state.set_status(status, false);
                        }
                        Err(err) => self
                            .state
                            .set_status(format!("commit prompt open failed: {err}"), true),
                    }
                }
                Err(err) => self
                    .state
                    .set_status(format!("commit prompt open failed: {err}"), true),
            },
            PendingAction::StageAll => {
                spawn_operation(&mut self.state, "staging", OperationKind::Index, || {
                    crate::git::stage_all()?;
                    Ok("staged all".to_string())
                });
            }
            PendingAction::UnstageAll => {
                spawn_operation(&mut self.state, "unstaging", OperationKind::Index, || {
                    crate::git::unstage_all()?;
                    Ok("unstaged all".to_string())
                });
            }
            PendingAction::StagePath(path) => {
                spawn_operation(
                    &mut self.state,
                    "staging",
                    OperationKind::Index,
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
                    OperationKind::Index,
                    move || {
                        crate::git::unstage(&path)?;
                        Ok(format!("unstaged {path}"))
                    },
                );
            }
            PendingAction::RollbackPath { path, is_dir } => {
                spawn_operation(
                    &mut self.state,
                    "rolling back",
                    OperationKind::FileSystem,
                    move || {
                        crate::git::rollback_worktree_path(&path)?;
                        let label = if is_dir { "folder" } else { "file" };
                        Ok(format!("rolled back {label} {path}"))
                    },
                );
            }
            PendingAction::DeletePath { path, is_dir } => {
                spawn_operation(
                    &mut self.state,
                    "deleting",
                    OperationKind::FileSystem,
                    move || {
                        crate::git::delete_worktree_path(&path, is_dir)?;
                        Ok(format!("deleted {path}"))
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
            PendingAction::OpenProjectAt(path) => {
                match crate::git::open_project_path_in_ide(&PathBuf::from(path)) {
                    Ok(status) => self.state.set_status(status, false),
                    Err(err) => self.state.set_status(format!("open failed: {err}"), true),
                }
            }
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
                    OperationKind::WorkingTree,
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
            PendingAction::SetBranchUpstream { branch, upstream } => {
                spawn_operation(
                    &mut self.state,
                    "setting upstream",
                    OperationKind::WorkingTree,
                    move || crate::git::set_branch_upstream(&branch, &upstream),
                );
            }
            PendingAction::CreateWorktree { path, branch, base } => {
                spawn_operation(
                    &mut self.state,
                    "adding worktree",
                    OperationKind::WorkingTree,
                    move || {
                        let out = crate::git::worktree_add(Path::new(&path), &branch, &base)?;
                        Ok(out
                            .lines()
                            .rfind(|line| !line.trim().is_empty())
                            .unwrap_or("worktree added")
                            .to_owned())
                    },
                );
            }
            PendingAction::RemoveWorktree { path, force } => {
                spawn_operation(
                    &mut self.state,
                    "removing worktree",
                    OperationKind::WorkingTree,
                    move || {
                        let out = crate::git::worktree_remove(Path::new(&path), force)?;
                        Ok(out
                            .lines()
                            .rfind(|line| !line.trim().is_empty())
                            .unwrap_or("worktree removed")
                            .to_owned())
                    },
                );
            }
            PendingAction::LandWorktree { path, branch } => {
                self.leave_checkout_before_removal(&path);
                spawn_operation(
                    &mut self.state,
                    "landing worktree",
                    OperationKind::WorkingTree,
                    move || crate::git::worktree_land(Path::new(&path), &branch),
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
                spawn_operation(
                    &mut self.state,
                    "pruning worktrees",
                    OperationKind::WorkingTree,
                    || {
                        let out = crate::git::worktree_prune()?;
                        Ok(out
                            .lines()
                            .rfind(|line| !line.trim().is_empty())
                            .unwrap_or("pruned")
                            .to_owned())
                    },
                );
            }
            PendingAction::Quit => self.state.should_quit = true,
            PendingAction::StartSession {
                path,
                label,
                sandboxed,
            } => {
                let cwd = PathBuf::from(&path);
                if sandboxed {
                    match prepare_sandbox(&cwd) {
                        Ok(Some(note)) => self.state.set_status(note, false),
                        Ok(None) => {}
                        Err(err) => {
                            // Running unsandboxed instead would quietly hand the
                            // session the whole filesystem, which is the
                            // opposite of what was asked for.
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
                        self.state.set_status(format!("session for {label}"), false);
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("start session failed: {err}"), true),
                }
            }
            PendingAction::SwitchRepository { target } => {
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
    }
}

impl App {
    /// Move lg off a checkout that is about to be removed, so the panels are
    /// not left pointing at a directory that no longer exists. The main
    /// worktree is where every repository has one.
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
fn prepare_sandbox(cwd: &Path) -> Result<Option<String>> {
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
