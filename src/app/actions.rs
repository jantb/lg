use anyhow::{Context, Result};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use crate::state::{AppState, Modal, OperationKind, PendingAction};

use super::{
    App, spawn_operation, spawn_pull, spawn_push, spawn_review_assist, spawn_review_chat,
    spawn_review_pr_text, spawn_review_style_flags,
};

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
                    OperationKind::Worktree,
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
            PendingAction::SaveLlmSettings { model, provider } => {
                match crate::llm::save_llm_settings(&model, provider) {
                    Ok(()) => {
                        refresh_llm_settings_state(&mut self.state);
                        self.state.llm_model_input = self.state.llm_model.clone();
                        self.state.modal = Modal::None;
                        if crate::llm::env_model_active() || crate::llm::env_provider_active() {
                            self.state
                                .set_status("saved LLM settings; env override is active", false);
                        } else {
                            self.state.set_status("saved LLM settings", false);
                        }
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("model save failed: {err}"), true),
                }
            }
            PendingAction::ClearLlmSettings => match crate::llm::clear_saved_llm_settings() {
                Ok(()) => {
                    refresh_llm_settings_state(&mut self.state);
                    self.state.llm_model_input = self.state.llm_model.clone();
                    self.state.modal = Modal::None;
                    self.state.set_status("cleared saved LLM settings", false);
                }
                Err(err) => self
                    .state
                    .set_status(format!("model clear failed: {err}"), true),
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
                    OperationKind::Worktree,
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
                    OperationKind::Worktree,
                    move || crate::git::set_branch_upstream(&branch, &upstream),
                );
            }
            PendingAction::SwitchRepository { path } => {
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
                let target = match path.as_deref() {
                    Some(path) => PathBuf::from(&root).join(path),
                    None => PathBuf::from(&root),
                };
                match std::env::set_current_dir(&target) {
                    Ok(()) => {
                        let label = path.unwrap_or_else(|| "workspace".to_string());
                        self.state.repo_root = Some(target.to_string_lossy().into_owned());
                        self.state.current_branch_releases = Default::default();
                        self.state.current_branch_releases_ref = None;
                        self.defer_release_status_job();
                        self.state.nested_repo_detail_path = None;
                        self.state.nested_repo_branches.clear();
                        self.state.nested_repo_remote_branches.clear();
                        self.state.set_status(format!("selected {label}"), false);
                        self.start_refresh(true);
                    }
                    Err(err) => self
                        .state
                        .set_status(format!("select repo failed: {err}"), true),
                }
            }
        }
    }
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
