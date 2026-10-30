//! The route each branch action takes: its lanes and the moves between them,
//! in the order the flow runs them.

use crate::state::{AppState, FlowAction, FlowRun};

use super::{MAX_LABEL, Preview};

/// The diagram for `run`, or `None` when there is nothing worth drawing.
///
/// Everything is read off the run rather than off the state: while a flow works
/// it checks other branches out, and a picture that followed the checkout would
/// redraw itself as something else halfway through.
pub(in crate::panel::flow) fn preview(state: &AppState, run: &FlowRun) -> Option<Preview> {
    let configured_base = crate::preferences::base_branch();
    if run.branch.is_empty() {
        return None;
    }
    let current = run.branch.clone();
    let steps = super::super::steps_for(run);
    let remote_main = format!("origin/{configured_base}");
    let created = run
        .input
        .clone()
        .unwrap_or_else(|| "new branch".to_string());

    Some(match run.action {
        FlowAction::MergeMain => {
            let mut preview = Preview::new(
                format!("{remote_main} lands in {current}, which is then pushed"),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let branch = preview.lane(state, &current);
            let remote = preview.lane(state, &format!("origin/{current}"));
            preview.merge(main, branch, &format!("merge {configured_base} into"));
            preview.merge(branch, remote, &format!("push {current}"));
            preview
        }
        FlowAction::ReleaseDev | FlowAction::ReleaseTest => {
            let target = run.target.clone()?;
            let mut preview = Preview::new(
                format!(
                    "{configured_base} and {current} merge into {target}, which is then pushed"
                ),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let feature = preview.lane(state, &format!("origin/{current}"));
            let deploy = preview.lane(state, &target);
            let remote = preview.lane(state, &format!("origin/{target}"));
            preview.merge(main, deploy, &format!("merge origin/{configured_base}"));
            preview.merge(feature, deploy, &format!("merge origin/{current}"));
            preview.merge(deploy, remote, "push HEAD to");
            preview
        }
        FlowAction::ResetDev | FlowAction::ResetTest => {
            let target = run.target.clone()?;
            let mut preview = Preview::new(
                format!(
                    "{target} is thrown away and rebuilt from {remote_main}, then force-pushed"
                ),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let deploy = preview.lane(state, &target);
            let remote = preview.lane(state, &format!("origin/{target}"));
            preview.rebuild(main, deploy, &format!("reset {target} to"));
            preview.rebuild(deploy, remote, "force push");
            preview
        }
        FlowAction::DiscardCheckout => {
            let mut preview = Preview::new(
                format!(
                    "everything in {current} that is not on its remote goes, untracked files included"
                ),
                steps,
            );
            let remote = preview.lane(state, &format!("origin/{current}"));
            let branch = preview.lane(state, &current);
            preview.rebuild(remote, branch, &format!("reset {current} to"));
            preview
        }
        FlowAction::NewFeature => {
            let mut preview = Preview::new(
                format!("a new branch starts from {remote_main} and is pushed"),
                steps,
            );
            let main = preview.lane(state, &remote_main);
            let branch = preview.lane(state, &created);
            let remote = preview.lane(state, &format!("origin/{created}"));
            preview.merge(main, branch, "create ");
            preview.merge(branch, remote, "push and set upstream");
            preview
        }
        FlowAction::TransferDiff => {
            let mut preview = Preview::new(
                format!(
                    "what {current} changed against {configured_base} is staged on a new branch"
                ),
                steps,
            );
            let from = preview.lane(state, &current);
            let branch = preview.lane(state, &created);
            preview.merge(from, branch, "apply diff");
            preview
        }
        FlowAction::CleanOrphans => {
            let mut preview = Preview::new(
                "merged local branches with no remote are deleted; unmerged ones, tracked ones and lg backups stay",
                steps,
            );
            preview.lane(state, "tracked");
            let orphan = preview.lane(state, "no upstream");
            preview.drop_lane(orphan, "delete orphan");
            preview
        }
    })
}

/// A label short enough to leave the track room, cut at the end because branch
/// names differ at the end far more often than at the start.
pub(super) fn shorten(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL {
        return label.to_string();
    }
    let kept: String = label.chars().take(MAX_LABEL - 1).collect();
    format!("{kept}\u{2026}")
}
