use crate::config::{BRANCH_MAIN, is_deploy_branch_name, protected_branch_list};
use crate::state::{
    AppState, BranchView, ConflictFollowup, FlowAction, FlowRun, Modal, Pane, SafetyRefCleanup,
    WorkflowJob, WorkflowMsg,
};

use super::spawn::git_job_running;

pub(crate) fn run_flow_action(state: &mut AppState, action: FlowAction, input: Option<String>) {
    if git_job_running(state) {
        return;
    }
    let current = state.branch.clone().unwrap_or_default();
    if matches!(action, FlowAction::MergeMain) && !state.merge_main_available() {
        state.modal = Modal::None;
        let status = merge_main_unavailable_status(&current);
        state.set_status(status, true);
        return;
    }
    let release_target = action
        .release_env()
        .and_then(|env| state.release_branch(env).map(str::to_string));
    if action.release_env().is_some() && release_target.is_none() {
        state.modal = Modal::None;
        state.set_status(
            format!(
                "branch action unavailable: no deploy branch in this repository ({})",
                protected_branch_list()
            ),
            true,
        );
        return;
    }

    let selected_branch = selected_action_branch(state, &current);
    if matches!(action, FlowAction::TransferDiff) && selected_branch.is_empty() {
        state.modal = Modal::None;
        state.set_status("select a local feature branch first", true);
        return;
    }
    let action_branch = if matches!(action, FlowAction::TransferDiff) {
        selected_branch
    } else {
        current.clone()
    };

    let label = state.flow_action_label(action);
    let steps = workflow_steps(
        action,
        &action_branch,
        input.as_deref(),
        release_target.as_deref(),
    );
    let thread_steps = steps.clone();
    // What the run is doing, resolved once here: the flow checks other branches
    // out as it goes, so the modal cannot read the names off the state it is
    // drawn from.
    let flow = FlowRun {
        action,
        branch: action_branch.clone(),
        target: release_target.clone(),
        input: input.clone().filter(|name| !name.is_empty()),
    };
    state.conflict_followup =
        conflict_followup_for_flow(action, &action_branch, release_target.as_deref());
    let target = release_target.unwrap_or_default();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = crate::git::spawn_pinned(move || {
        let mut step_idx = 0usize;
        let mut progress = || {
            let _ = tx.send(WorkflowMsg::Progress(step_idx));
            step_idx += 1;
        };
        let res = match action {
            FlowAction::MergeMain => {
                crate::git::flow_merge_main_into_current_with_progress(&current, &mut progress)
            }
            FlowAction::ReleaseDev | FlowAction::ReleaseTest => {
                crate::git::flow_release_current_with_progress(&current, &target, &mut progress)
            }
            FlowAction::ResetDev | FlowAction::ResetTest => {
                crate::git::flow_reset_branch_from_main_with_progress(
                    &current,
                    &target,
                    &mut progress,
                )
            }
            FlowAction::DiscardCheckout => {
                crate::git::flow_discard_checkout_from_remote_with_progress(&current, &mut progress)
            }
            FlowAction::NewFeature => {
                for _ in &thread_steps {
                    progress();
                }
                crate::git::flow_create_feature_branch(&current, &input.unwrap_or_default())
            }
            FlowAction::TransferDiff => {
                crate::git::flow_transfer_diff_to_feature_branch_with_progress(
                    &action_branch,
                    &input.unwrap_or_default(),
                    &mut progress,
                )
            }
            FlowAction::CleanOrphans => {
                for _ in &thread_steps {
                    progress();
                }
                crate::git::flow_clean_orphan_branches(&current)
            }
        };
        match res {
            Ok(s) => {
                let _ = tx.send(WorkflowMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(WorkflowMsg::Error(e.to_string()));
            }
        }
    });

    state.workflow_job = Some(WorkflowJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        label,
        steps,
        current_step: None,
        flow: Some(flow),
    });
    state.set_status("running branch action\u{2026}", false);
}

fn selected_action_branch(state: &AppState, current: &str) -> String {
    if state.focus == Pane::Branches
        && state.branch_view == BranchView::Local
        && let Some(branch) = state.selected_branch_ref()
    {
        return branch.to_string();
    }
    current.to_string()
}

fn merge_main_unavailable_status(current: &str) -> &'static str {
    if is_deploy_branch_name(current) {
        "current branch is not behind origin/main"
    } else {
        "checkout a feature branch before merging main"
    }
}

fn conflict_followup_for_flow(
    action: FlowAction,
    current: &str,
    release_target: Option<&str>,
) -> Option<ConflictFollowup> {
    match action {
        FlowAction::MergeMain => Some(ConflictFollowup {
            // Merging main in is the flow's only merge, so the conflict is that
            // merge: there is nothing left to catch up on.
            merge_branch: None,
            push_branch: Some(current.to_string()),
            return_branch: Some(current.to_string()),
            safety_ref_cleanup: Some(SafetyRefCleanup {
                label: "merge-main".to_string(),
                branch: current.to_string(),
            }),
            resume: None,
        }),
        FlowAction::ReleaseDev | FlowAction::ReleaseTest => Some(ConflictFollowup {
            // A release merges main in before it merges the feature, so a
            // conflict on the first leaves the second still to do.
            merge_branch: Some(current.to_string()),
            push_branch: release_target.map(str::to_string),
            return_branch: Some(current.to_string()),
            safety_ref_cleanup: Some(SafetyRefCleanup {
                label: "release-current".to_string(),
                branch: current.to_string(),
            }),
            resume: None,
        }),
        FlowAction::ResetDev
        | FlowAction::ResetTest
        | FlowAction::DiscardCheckout
        | FlowAction::NewFeature
        | FlowAction::TransferDiff
        | FlowAction::CleanOrphans => None,
    }
}

pub(crate) fn workflow_steps(
    action: FlowAction,
    current: &str,
    input: Option<&str>,
    release_target: Option<&str>,
) -> Vec<String> {
    let target = release_target.unwrap_or_default();
    match action {
        FlowAction::MergeMain => vec![
            "stash current changes".into(),
            "create safety backup".into(),
            "fetch origin".into(),
            format!("pull {current}"),
            format!("update {} from origin", BRANCH_MAIN),
            format!("merge {} into {current}", BRANCH_MAIN),
            format!("push {current}"),
            "restore stashed changes".into(),
            "remove safety backup".into(),
        ],
        FlowAction::ReleaseDev | FlowAction::ReleaseTest => release_steps(current, target),
        FlowAction::ResetDev | FlowAction::ResetTest => reset_steps(current, target),
        FlowAction::DiscardCheckout => vec![
            "fetch remote".into(),
            format!("reset {current} to remote"),
            "delete untracked files".into(),
        ],
        FlowAction::NewFeature => vec![
            format!(
                "create {}",
                input.filter(|s| !s.is_empty()).unwrap_or("new branch")
            ),
            "push and set upstream".into(),
        ],
        FlowAction::TransferDiff => vec![
            format!("fetch {}", BRANCH_MAIN),
            format!("diff {current} against {}", BRANCH_MAIN),
            format!(
                "create {}",
                input.filter(|s| !s.is_empty()).unwrap_or("new branch")
            ),
            "apply diff as staged changes".into(),
        ],
        FlowAction::CleanOrphans => vec!["scan branches".into(), "delete orphan branches".into()],
    }
}

fn release_steps(current: &str, target: &str) -> Vec<String> {
    vec![
        "stash current changes".into(),
        "create safety backup".into(),
        format!("push {current}"),
        "fetch origin".into(),
        format!("sync {target} from origin/{target}"),
        format!("checkout {target}"),
        format!("merge origin/{}", BRANCH_MAIN),
        format!("merge origin/{current}"),
        format!("push HEAD to origin/{target}"),
        format!("checkout {current}"),
        "restore stashed changes".into(),
    ]
}

fn reset_steps(current: &str, target: &str) -> Vec<String> {
    let mut steps = vec!["fetch origin".into()];
    if current != target {
        steps.push(format!("checkout {target}"));
    }
    steps.extend([
        "create safety backup".into(),
        format!("reset {target} to origin/{}", BRANCH_MAIN),
        format!("force push {target}"),
        "remove safety backup".into(),
    ]);
    if current != target {
        steps.push(format!("checkout {current}"));
    }
    steps
}

pub(crate) fn validate_conflict_resolution(state: &mut AppState) {
    if state.workflow_job.is_some() {
        return;
    }
    let followup = state.conflict_followup.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = crate::git::spawn_pinned(move || {
        let followup = crate::git::Followup {
            merge_branch: followup.as_ref().and_then(|f| f.merge_branch.as_deref()),
            push_branch: followup.as_ref().and_then(|f| f.push_branch.as_deref()),
            return_branch: followup.as_ref().and_then(|f| f.return_branch.as_deref()),
            safety_cleanup: followup
                .as_ref()
                .and_then(|f| f.safety_ref_cleanup.as_ref())
                .map(|cleanup| (cleanup.label.as_str(), cleanup.branch.as_str())),
        };
        match crate::git::validate_conflict_resolution(followup) {
            Ok(s) => {
                let _ = tx.send(WorkflowMsg::Done(s));
            }
            Err(e) => {
                let _ = tx.send(WorkflowMsg::Error(e.to_string()));
            }
        }
    });
    state.workflow_job = Some(WorkflowJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        label: "validate conflict resolution".to_string(),
        steps: vec![
            "detect conflict state".to_string(),
            "continue Git operation if needed".to_string(),
            "finish the merge the flow stopped before".to_string(),
            "push release branch if needed".to_string(),
            "return to feature branch if needed".to_string(),
        ],
        current_step: None,
        flow: None,
    });
    state.set_status("validating conflict resolution\u{2026}", false);
}

/// Hand the conflict to an agent session in the checkout it happened in.
///
/// Resolving a merge is reading two versions of a file and deciding what the
/// result should be, which is work lg deliberately does not do itself — so this
/// starts something that can, in the checkout the conflict is in, opening on
/// the files git could not merge. Nothing is settled by it: the flow is still
/// waiting, and `F` comes back here to finish it with `v`.
///
/// Which agent is whichever one was last started from the workspace picker: a
/// conflict is the same work as everything else in the checkout, so it goes to
/// the same tool.
pub(crate) fn start_conflict_session(state: &mut AppState, sandboxed: bool) {
    let Some(path) = state.repo_root.clone() else {
        state.set_status("no repository for a session", true);
        return;
    };
    let label = state
        .branch
        .clone()
        .unwrap_or_else(|| checkout_name(&path).to_string());
    let kind = state.preferred_agent;

    state.modal = Modal::None;
    // One of each agent per checkout: a session already open here is the one
    // that should hear about the conflict, and it cannot be told twice.
    if let Some(id) = state
        .sessions
        .for_dir_kind(std::path::Path::new(&path), kind)
    {
        state.show_session(id);
        state.session_capture = true;
        state.set_status(
            format!(
                "the {} here is already running \u{2014} tell it about the conflict yourself",
                kind.label()
            ),
            false,
        );
        return;
    }

    let prompt = conflict_prompt(
        &state.unresolved_conflicts(),
        &sorted(&state.conflict_resolved),
    );
    state.pending_action = Some(crate::state::PendingAction::StartSession {
        path,
        label,
        sandboxed,
        kind,
        prompt: Some(prompt),
    });
}

fn sorted(paths: &std::collections::HashSet<String>) -> Vec<String> {
    let mut paths: Vec<String> = paths.iter().cloned().collect();
    paths.sort();
    paths
}

/// What the session opens on. It names the files rather than describing the
/// conflict, because the files are the part lg knows and the rest is on disk.
/// Committing is left open on purpose: validation settles the merge either way,
/// and a session told not to commit would only be told wrong.
///
/// Files the local model already settled are named too, and named as done. A
/// session that is not told about them re-resolves work that is already on
/// disk; one that is told they are conflicted goes looking for markers that are
/// no longer there.
fn conflict_prompt(conflicts: &[String], already_resolved: &[String]) -> String {
    let mut prompt = String::from(
        "Resolve the git merge conflict in this repository. Edit each file so the \
         conflict markers are gone and the result is right; staging or committing \
         is up to you, lg finishes the merge either way.",
    );
    if !conflicts.is_empty() {
        prompt.push_str("\n\nGit reports these files as conflicted:\n");
        for path in conflicts {
            prompt.push_str("- ");
            prompt.push_str(path);
            prompt.push('\n');
        }
    }
    if !already_resolved.is_empty() {
        prompt.push_str(
            "\nA local model already merged these files and wrote them back, so they hold \
             no conflict markers. Leave them alone unless you find a mistake in one:\n",
        );
        for path in already_resolved {
            prompt.push_str("- ");
            prompt.push_str(path);
            prompt.push('\n');
        }
    }
    prompt
}

fn checkout_name(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

pub(crate) fn abort_conflict_operation(state: &mut AppState) {
    if state.workflow_job.is_some() {
        return;
    }
    let return_branch = state
        .conflict_followup
        .as_ref()
        .and_then(|f| f.return_branch.clone());
    let safety_cleanup = state
        .conflict_followup
        .as_ref()
        .and_then(|f| f.safety_ref_cleanup.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let handle =
        crate::git::spawn_pinned(
            move || match crate::git::abort_in_progress_operation_with_cleanup(
                return_branch.as_deref(),
                safety_cleanup
                    .as_ref()
                    .map(|cleanup| (cleanup.label.as_str(), cleanup.branch.as_str())),
            ) {
                Ok(s) => {
                    let _ = tx.send(WorkflowMsg::Done(s));
                }
                Err(e) => {
                    let _ = tx.send(WorkflowMsg::Error(e.to_string()));
                }
            },
        );
    state.workflow_job = Some(WorkflowJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        label: "abort merge".to_string(),
        steps: Vec::new(),
        current_step: None,
        flow: None,
    });
    state.set_status("aborting git operation\u{2026}", false);
}
