use crate::app;
use crate::config::is_protected_branch_name;
use crate::state::{AppState, BranchView, FlowAction, FlowRun};

mod draw;
mod input;
mod preview;

pub use draw::render;
pub(crate) use draw::sync_scroll_offset;
pub use input::{handle_key, handle_mouse};

fn merge_main_available(state: &AppState) -> bool {
    state.merge_main_available()
}

/// The actions the menu offers for this checkout. On a feature branch only
/// what concerns that branch is listed: merging the trunk in, releasing it,
/// and resetting the checkout. Resetting an environment is offered on that
/// environment's own branch; starting features and cleaning up are the
/// trunk's business and wait there.
pub fn available_actions(state: &AppState) -> Vec<FlowAction> {
    let feature = state.on_feature_branch();
    FlowAction::ALL
        .into_iter()
        .filter(|action| match action {
            FlowAction::MergeMain => merge_main_available(state),
            // Only a feature branch is released. An environment is not merged
            // into another one — trunk or deploy branch, the checkout stays
            // where it is.
            FlowAction::ReleaseDev | FlowAction::ReleaseTest => {
                feature
                    && action
                        .release_env()
                        .is_some_and(|env| state.release_branch(env).is_some())
            }
            FlowAction::ResetDev | FlowAction::ResetTest => action
                .release_env()
                .and_then(|env| state.release_branch(env))
                .is_some_and(|deploy| state.branch.as_deref() == Some(deploy)),
            FlowAction::TransferDiff => !feature && selected_feature_branch(state).is_some(),
            FlowAction::DiscardCheckout => state.branch.is_some(),
            FlowAction::NewFeature | FlowAction::CleanOrphans => !feature,
        })
        .collect()
}

/// What the highlighted action would run against, resolved the way running it
/// resolves it. The graph and the step list are both built from this, so the
/// menu cannot draw one branch and run against another.
fn flow_run(state: &AppState, action: FlowAction) -> FlowRun {
    let branch = match action {
        FlowAction::TransferDiff => selected_feature_branch(state),
        _ => state.branch.clone(),
    }
    .unwrap_or_default();
    FlowRun {
        action,
        branch,
        target: action
            .release_env()
            .and_then(|env| state.release_branch(env))
            .map(str::to_string),
        // Nothing has been typed yet; a run carries the name it was given.
        input: None,
    }
}

/// The steps a run performs, asked of the same code that narrates them while it
/// runs — so what the menu promises and what the progress list shows cannot
/// drift apart.
fn steps_for(run: &FlowRun) -> Vec<String> {
    app::workflow_steps(
        run.action,
        &run.branch,
        run.input.as_deref(),
        run.target.as_deref(),
    )
}

fn selected_feature_branch(state: &AppState) -> Option<String> {
    if state.branch_view != BranchView::Local {
        return None;
    }
    let branch = state.selected_branch_ref()?;
    if is_protected_branch_name(branch) {
        None
    } else {
        Some(branch.to_string())
    }
}

fn selected_branch_line(state: &AppState) -> String {
    match state.selected_branch_ref() {
        Some(branch) if state.branch_view == BranchView::Local => format!("selected: {branch}"),
        Some(branch) => format!("selected remote: {branch}"),
        None => "no branch selected".to_string(),
    }
}

#[cfg(test)]
mod tests;
