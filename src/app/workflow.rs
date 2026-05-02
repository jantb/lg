use crate::config::{BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST};
use crate::state::{AppState, ConflictFollowup, FlowAction, Modal, WorkflowJob, WorkflowMsg};

use super::spawn::git_job_running;

pub(crate) fn run_flow_action(state: &mut AppState, action: FlowAction, input: Option<String>) {
    if git_job_running(state) {
        return;
    }
    if !state.flow_available() {
        state.modal = Modal::None;
        state.set_status("flow unavailable: missing develop or release/next", true);
        return;
    }

    let current = state.branch.clone().unwrap_or_default();
    let label = action.label().to_owned();
    let steps = workflow_steps(action, &current, input.as_deref());
    let thread_steps = steps.clone();
    state.conflict_followup = conflict_followup_for_flow(action, &current);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut step_idx = 0usize;
        let mut progress = || {
            let _ = tx.send(WorkflowMsg::Progress(step_idx));
            step_idx += 1;
        };
        let res = match action {
            FlowAction::MergeMain => {
                crate::git::flow_merge_main_into_current_with_progress(&current, &mut progress)
            }
            FlowAction::ReleaseDev => {
                crate::git::flow_release_current_with_progress(&current, BRANCH_DEV, &mut progress)
            }
            FlowAction::ReleaseTest => {
                crate::git::flow_release_current_with_progress(&current, BRANCH_TEST, &mut progress)
            }
            FlowAction::ResetDev => crate::git::flow_reset_branch_from_main_with_progress(
                &current,
                BRANCH_DEV,
                &mut progress,
            ),
            FlowAction::ResetTest => crate::git::flow_reset_branch_from_main_with_progress(
                &current,
                BRANCH_TEST,
                &mut progress,
            ),
            FlowAction::NewFeature => {
                for _ in &thread_steps {
                    progress();
                }
                crate::git::flow_create_feature_branch(&current, &input.unwrap_or_default())
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
        spinner: 0,
        label,
        steps,
        current_step: None,
    });
    state.set_status("running flow workflow\u{2026}", false);
}

fn conflict_followup_for_flow(action: FlowAction, current: &str) -> Option<ConflictFollowup> {
    match action {
        FlowAction::MergeMain => Some(ConflictFollowup {
            push_branch: Some(current.to_string()),
            return_branch: Some(current.to_string()),
        }),
        FlowAction::ReleaseDev => Some(ConflictFollowup {
            push_branch: Some(BRANCH_DEV.to_string()),
            return_branch: Some(current.to_string()),
        }),
        FlowAction::ReleaseTest => Some(ConflictFollowup {
            push_branch: Some(BRANCH_TEST.to_string()),
            return_branch: Some(current.to_string()),
        }),
        FlowAction::ResetDev
        | FlowAction::ResetTest
        | FlowAction::NewFeature
        | FlowAction::CleanOrphans => None,
    }
}

pub(super) fn workflow_steps(
    action: FlowAction,
    current: &str,
    input: Option<&str>,
) -> Vec<String> {
    match action {
        FlowAction::MergeMain => vec![
            "stash current changes".into(),
            "create safety backup".into(),
            "fetch origin".into(),
            "checkout main".into(),
            "update main from origin/main".into(),
            format!("checkout {current}"),
            format!("merge origin/{} into {current}", BRANCH_MAIN),
            format!("push {current}"),
            "restore stashed changes".into(),
        ],
        FlowAction::ReleaseDev => release_steps(current, BRANCH_DEV),
        FlowAction::ReleaseTest => release_steps(current, BRANCH_TEST),
        FlowAction::ResetDev => reset_steps(current, BRANCH_DEV),
        FlowAction::ResetTest => reset_steps(current, BRANCH_TEST),
        FlowAction::NewFeature => vec![format!(
            "create {}",
            input.filter(|s| !s.is_empty()).unwrap_or("new branch")
        )],
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
    std::thread::spawn(move || {
        match crate::git::validate_conflict_resolution_with_followup(
            followup.as_ref().and_then(|f| f.push_branch.as_deref()),
            followup.as_ref().and_then(|f| f.return_branch.as_deref()),
        ) {
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
        spinner: 0,
        label: "validate conflict resolution".to_string(),
        steps: vec![
            "detect conflict state".to_string(),
            "continue Git operation if needed".to_string(),
            "push release branch if needed".to_string(),
            "return to feature branch if needed".to_string(),
        ],
        current_step: None,
    });
    state.set_status("validating conflict resolution\u{2026}", false);
}

pub(crate) fn abort_conflict_operation(state: &mut AppState) {
    if state.workflow_job.is_some() {
        return;
    }
    let return_branch = state
        .conflict_followup
        .as_ref()
        .and_then(|f| f.return_branch.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        match crate::git::abort_in_progress_operation_with_return(return_branch.as_deref()) {
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
        spinner: 0,
        label: "abort merge".to_string(),
        steps: Vec::new(),
        current_step: None,
    });
    state.set_status("aborting git operation\u{2026}", false);
}
