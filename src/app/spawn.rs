use anyhow::Result;

use crate::config::{COMMIT_LIST_LIMIT, DEFAULT_PUSH_REMOTE};
use crate::state::{
    AppState, CheckoutJob, CheckoutMsg, DiffSource, Modal, OperationJob, OperationKind,
    OperationMsg, Pane, PushJob, PushMsg, TreeKind,
};

pub(super) fn git_job_running(state: &AppState) -> bool {
    state.push_job.is_some()
        || state.checkout_job.is_some()
        || state.operation_job.is_some()
        || state.fetch_job.is_some()
        || state.workflow_job.is_some()
}

fn operation_block_reason(state: &AppState, kind: OperationKind) -> Option<&'static str> {
    if state.push_job.is_some() {
        Some("push in progress")
    } else if state.checkout_job.is_some() {
        Some("checkout in progress")
    } else if let Some(job) = &state.operation_job {
        Some(job.label)
    } else if state.workflow_job.is_some() {
        Some("workflow in progress")
    } else if state.fetch_job.is_some()
        && !matches!(
            kind,
            OperationKind::Index | OperationKind::StageAllAndCommit | OperationKind::FileSystem
        )
    {
        Some("fetch in progress")
    } else {
        None
    }
}

fn blocked_operation_status(label: &str, reason: &str) -> String {
    format!("{label} blocked: {reason}")
}

pub(super) fn selected_diff_source(state: &AppState) -> DiffSource {
    match state.focus {
        Pane::Files => {
            let rows = state.tree_rows();
            match rows.get(state.files_idx) {
                Some(row) => match &row.kind {
                    TreeKind::AllChanges => DiffSource::All,
                    TreeKind::Folder { .. } => DiffSource::Folder(row.path.clone()),
                    TreeKind::File { entry_idx } => state
                        .files
                        .get(*entry_idx)
                        .map(|f| DiffSource::File(f.path.clone()))
                        .unwrap_or(DiffSource::None),
                },
                None => DiffSource::None,
            }
        }
        Pane::Commits => state
            .commits
            .get(state.commits_idx)
            .filter(|c| !c.is_graph_row())
            .map(|c| DiffSource::Commit(c.sha.clone()))
            .unwrap_or(DiffSource::None),
        Pane::Branches => state
            .selected_branch_ref()
            .map(|branch| DiffSource::Branch(branch.to_string()))
            .unwrap_or(DiffSource::None),
        _ => DiffSource::None,
    }
}

pub(super) fn selected_commit_ref(state: &AppState) -> Option<String> {
    if state.focus == Pane::Branches {
        state
            .selected_branch_ref()
            .map(ToOwned::to_owned)
            .or_else(|| state.branch.clone())
    } else {
        state.branch.clone()
    }
}

pub(super) fn load_diff_text(source: &DiffSource) -> String {
    match source {
        DiffSource::None | DiffSource::Review => String::new(),
        DiffSource::All => crate::git::all_diffs().unwrap_or_else(|e| format!("error: {e}")),
        DiffSource::File(path) => {
            crate::git::file_diff(path).unwrap_or_else(|e| format!("error: {e}"))
        }
        DiffSource::Folder(path) => {
            crate::git::folder_diff(path).unwrap_or_else(|e| format!("error: {e}"))
        }
        DiffSource::Commit(sha) => {
            crate::git::show_commit(sha).unwrap_or_else(|e| format!("error: {e}"))
        }
        DiffSource::Branch(branch) => crate::git::branch_log(branch, COMMIT_LIST_LIMIT)
            .unwrap_or_else(|e| format!("error: {e}")),
    }
}

pub(super) fn spawn_push(state: &mut AppState) {
    if git_job_running(state) {
        return;
    }
    if state.branch_diverged_from_remote() {
        state.modal = Modal::Push;
        state.set_status("branch diverged; merge upstream before pushing?", false);
        return;
    }
    if state.branch_behind_remote() {
        state.set_status("branch is behind remote; pull before pushing", true);
        return;
    }
    let branch = state.branch.clone().unwrap_or_default();
    let remote = DEFAULT_PUSH_REMOTE.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let tbranch = branch.clone();
    let tremote = remote.clone();
    let handle = crate::git::spawn_pinned(move || match crate::git::push(&tremote, &tbranch) {
        Ok(out) => {
            let line = out
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("pushed")
                .to_owned();
            let _ = tx.send(PushMsg::Done(line));
        }
        Err(e) => {
            let _ = tx.send(PushMsg::Error(e.to_string()));
        }
    });
    state.push_job = Some(PushJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        branch,
        remote,
    });
    state.set_status("pushing\u{2026}", false);
}

pub(super) fn spawn_pull(state: &mut AppState) {
    if git_job_running(state) {
        return;
    }
    if !state.pull_available() {
        state.set_status("nothing to pull", false);
        return;
    }
    let branch = state.branch.clone().unwrap_or_default();
    spawn_operation(state, "pulling", OperationKind::WorkingTree, move || {
        let out = crate::git::pull(DEFAULT_PUSH_REMOTE, &branch)?;
        Ok(out
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or("pulled")
            .to_owned())
    });
}

pub(super) fn open_author_modal(state: &mut AppState) {
    let config = crate::git::author_config();
    let root = crate::git::repo_root().unwrap_or_default();
    match config {
        Ok(config) => {
            state.author_path_input = if state.author_path_input.trim().is_empty() {
                root
            } else {
                state.author_path_input.clone()
            };
            state.author_name_input = config
                .local_name
                .clone()
                .or(config.name)
                .unwrap_or_default();
            state.author_email_input = config
                .local_email
                .clone()
                .or(config.email)
                .unwrap_or_default();
            state.author_has_local_override =
                config.local_name.is_some() || config.local_email.is_some();
            state.author_has_subtree_rule =
                crate::git::subtree_author_rule_exists(&state.author_path_input);
            state.author_field = crate::state::AuthorField::Path;
            state.modal = Modal::Author;
        }
        Err(err) => {
            state.set_status(format!("author config failed: {err}"), true);
        }
    }
}

pub(super) fn open_model_modal(state: &mut AppState) {
    state.llm_model = crate::llm::current_model();
    state.llm_model_input = state.llm_model.clone();
    state.llm_model_choices = crate::llm::available_models();
    state.llm_model_idx = state
        .llm_model_choices
        .iter()
        .position(|model| *model == state.llm_model_input)
        .unwrap_or(0);
    state.llm_provider = crate::llm::current_provider();
    state.llm_provider_idx = crate::llm::LlmProvider::ALL
        .iter()
        .position(|provider| *provider == state.llm_provider)
        .unwrap_or(0);
    state.llm_config_path = crate::llm::config_file_display();
    state.settings_field = crate::state::SettingsField::Model;
    state.settings_mode = crate::state::SettingsMode::Browse;
    load_repo_settings_into_state(state);
    suggest_repo_settings_if_unset(state);
    state.modal = Modal::Model;
}

/// A checkout with no settings of its own gets its language and house style read
/// out of its own commit history, so the modal opens with a real starting point.
/// Anything already saved is left alone.
pub(crate) fn suggest_repo_settings_if_unset(state: &mut AppState) {
    if crate::settings::is_configured() || state.settings_suggest_job.is_some() {
        return;
    }
    let history = match crate::git::recent_commit_messages(30) {
        Ok(history) if !history.trim().is_empty() => history,
        _ => return,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        crate::llm::suggest_repo_conventions(history, tx);
    });
    state.settings_suggest_job = Some(crate::state::SettingsSuggestJob {
        rx,
        handle: Some(handle),
        spinner: 0,
    });
    state.set_status("reading this checkout's conventions", false);
}

/// Mirrors the stored per-checkout settings into the editable modal fields.
pub(crate) fn load_repo_settings_into_state(state: &mut AppState) {
    let settings = crate::settings::load();
    state.settings_prompt_is_custom = settings.commit_prompt_is_custom();
    state.settings_review_style_is_custom = settings.review_style_is_custom();
    state.settings_pr_language_input = settings.pr_language;
    state.settings_comment_style_input = settings.comment_style;
    state.settings_subject_max_input = settings.commit_subject_max_chars.to_string();
    state.settings_body_lines_input = settings.commit_body_max_lines.to_string();
    state.settings_dir = crate::settings::settings_dir_display();
    state.settings_derived_language = false;
    state.settings_derived_shape = false;
}

pub(crate) fn checkout_branch_async(state: &mut AppState, branch: String) {
    if git_job_running(state) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let target = branch.clone();
    let handle = crate::git::spawn_pinned(move || match crate::git::checkout_branch(&target) {
        Ok(out) => {
            let line = out
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("checked out")
                .to_owned();
            let _ = tx.send(CheckoutMsg::Done(line));
        }
        Err(e) => {
            let _ = tx.send(CheckoutMsg::Error(e.to_string()));
        }
    });
    state.checkout_job = Some(CheckoutJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        branch: branch.clone(),
    });
    state.set_status(format!("checking out {branch}\u{2026}"), false);
}

pub(crate) fn checkout_remote_branch_async(state: &mut AppState, remote_ref: String) {
    if git_job_running(state) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let target = remote_ref.clone();
    let handle =
        crate::git::spawn_pinned(move || match crate::git::checkout_remote_branch(&target) {
            Ok(out) => {
                let line = out
                    .lines()
                    .rfind(|l| !l.trim().is_empty())
                    .unwrap_or("checked out")
                    .to_owned();
                let _ = tx.send(CheckoutMsg::Done(line));
            }
            Err(e) => {
                let _ = tx.send(CheckoutMsg::Error(e.to_string()));
            }
        });
    state.checkout_job = Some(CheckoutJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        branch: remote_ref.clone(),
    });
    state.set_status(format!("checking out {remote_ref}\u{2026}"), false);
}

pub(crate) fn checkout_nested_branch_async(
    state: &mut AppState,
    repo_path: String,
    branch: String,
) {
    if git_job_running(state) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let target_repo = repo_path.clone();
    let target_branch = branch.clone();
    let workspace_root = state.workspace_root.clone();
    let handle = crate::git::spawn_pinned(move || {
        let result = if let Some(root) = workspace_root {
            crate::git::checkout_nested_branch_at(
                std::path::Path::new(&root),
                &target_repo,
                &target_branch,
            )
        } else {
            crate::git::checkout_nested_branch(&target_repo, &target_branch)
        };
        match result {
            Ok(out) => {
                let line = out
                    .lines()
                    .rfind(|l| !l.trim().is_empty())
                    .unwrap_or("checked out")
                    .to_owned();
                let _ = tx.send(CheckoutMsg::Done(line));
            }
            Err(e) => {
                let _ = tx.send(CheckoutMsg::Error(e.to_string()));
            }
        }
    });
    state.checkout_job = Some(CheckoutJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        branch: format!("{repo_path}:{branch}"),
    });
    state.set_status(
        format!("checking out {branch} in {repo_path}\u{2026}"),
        false,
    );
}

pub(crate) fn checkout_nested_remote_branch_async(
    state: &mut AppState,
    repo_path: String,
    remote_ref: String,
) {
    if git_job_running(state) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let target_repo = repo_path.clone();
    let target_ref = remote_ref.clone();
    let workspace_root = state.workspace_root.clone();
    let handle = crate::git::spawn_pinned(move || {
        let result = if let Some(root) = workspace_root {
            crate::git::checkout_nested_remote_branch_at(
                std::path::Path::new(&root),
                &target_repo,
                &target_ref,
            )
        } else {
            crate::git::checkout_nested_remote_branch(&target_repo, &target_ref)
        };
        match result {
            Ok(out) => {
                let line = out
                    .lines()
                    .rfind(|l| !l.trim().is_empty())
                    .unwrap_or("checked out")
                    .to_owned();
                let _ = tx.send(CheckoutMsg::Done(line));
            }
            Err(e) => {
                let _ = tx.send(CheckoutMsg::Error(e.to_string()));
            }
        }
    });
    state.checkout_job = Some(CheckoutJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        branch: format!("{repo_path}:{remote_ref}"),
    });
    state.set_status(
        format!("checking out {remote_ref} in {repo_path}\u{2026}"),
        false,
    );
}

pub(super) fn spawn_operation<F>(
    state: &mut AppState,
    label: &'static str,
    kind: OperationKind,
    work: F,
) where
    F: FnOnce() -> Result<String> + Send + 'static,
{
    spawn_operation_with_progress(state, label, kind, move |_| work());
}

/// Same, for work that takes long enough to be worth narrating. The closure is
/// handed a reporter to call as it moves between steps; a spinner that never
/// says more than "landing worktree" for half a minute reads as a hang.
pub(super) fn spawn_operation_with_progress<F>(
    state: &mut AppState,
    label: &'static str,
    kind: OperationKind,
    work: F,
) where
    F: FnOnce(&mut dyn FnMut(&str)) -> Result<String> + Send + 'static,
{
    if let Some(reason) = operation_block_reason(state, kind) {
        state.set_status(blocked_operation_status(label, reason), true);
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = crate::git::spawn_pinned(move || {
        let reports = tx.clone();
        let mut progress = |step: &str| {
            let _ = reports.send(OperationMsg::Progress(step.to_string()));
        };
        match work(&mut progress) {
            Ok(s) => {
                let _ = tx.send(OperationMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(OperationMsg::Error(e.to_string()));
            }
        }
    });
    state.operation_job = Some(OperationJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        label,
        kind,
        step: None,
    });
    state.set_status(format!("{label}\u{2026}"), false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::FetchJob;

    #[test]
    fn index_and_file_operations_can_start_during_fetch() {
        let mut state = AppState::new();
        let (_tx, rx) = std::sync::mpsc::channel();
        state.fetch_job = Some(FetchJob {
            rx,
            handle: None,
            spinner: 0,
        });

        assert!(operation_block_reason(&state, OperationKind::Index).is_none());
        assert!(operation_block_reason(&state, OperationKind::StageAllAndCommit).is_none());
        assert!(operation_block_reason(&state, OperationKind::FileSystem).is_none());
        assert!(operation_block_reason(&state, OperationKind::WorkingTree).is_some());
        assert!(operation_block_reason(&state, OperationKind::Commit).is_some());
    }

    #[test]
    fn blocked_operation_sets_status_message() {
        let mut state = AppState::new();
        let (_tx, rx) = std::sync::mpsc::channel();
        state.checkout_job = Some(CheckoutJob {
            rx,
            handle: None,
            spinner: 0,
            branch: "feature/demo".into(),
        });

        spawn_operation(
            &mut state,
            "deleting",
            OperationKind::FileSystem,
            || -> Result<String> { panic!("blocked operation should not run") },
        );

        assert!(state.operation_job.is_none());
        let status = state.status.as_ref().expect("blocked status");
        assert!(status.is_error);
        assert_eq!(status.text, "deleting blocked: checkout in progress");
    }
}
