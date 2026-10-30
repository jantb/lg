use ratatui::style::Color;
use ratatui::text::Line;

use crate::git::{Branch, ReleaseBranches};

use super::draw::{action_mark, step_lines};
use super::*;

fn state() -> AppState {
    let mut state = AppState::new();
    state.branch = Some("feature/send-cv".into());
    state.branches = vec![Branch {
        name: "feature/send-cv".into(),
        is_current: true,
        upstream: Some("origin/feature/send-cv".into()),
        upstream_gone: false,
        ahead: 1,
        behind: 0,
        behind_main: 2,
        last_commit_unix: None,
    }];
    state.branches_list.idx = 0;
    state.release_branches = ReleaseBranches::new(Some("develop".into()), Some("test".into()));
    state
}

fn text(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// On a feature branch the menu is about that branch: bring the trunk
/// in, release it, or throw the checkout away. Trunk housekeeping is not
/// offered until the trunk is checked out.
#[test]
fn a_feature_branch_offers_only_the_actions_about_itself() {
    let offered = available_actions(&state());
    assert!(offered.contains(&FlowAction::MergeMain), "{offered:?}");
    assert!(offered.contains(&FlowAction::ReleaseTest), "{offered:?}");
    assert!(
        offered.contains(&FlowAction::DiscardCheckout),
        "{offered:?}"
    );
    for general in [
        FlowAction::ResetTest,
        FlowAction::ResetDev,
        FlowAction::NewFeature,
        FlowAction::TransferDiff,
        FlowAction::CleanOrphans,
    ] {
        assert!(
            !offered.contains(&general),
            "{general:?} is trunk business: {offered:?}"
        );
    }
}

#[test]
fn the_trunk_offers_the_general_housekeeping_but_no_reset() {
    let mut state = state();
    state.branch = Some("main".into());
    state.branches[0].name = "main".into();
    let offered = available_actions(&state);
    for general in [FlowAction::NewFeature, FlowAction::CleanOrphans] {
        assert!(
            offered.contains(&general),
            "{general:?} missing: {offered:?}"
        );
    }
    assert!(
        !offered.contains(&FlowAction::ResetTest) && !offered.contains(&FlowAction::ResetDev),
        "resetting an environment belongs on its branch: {offered:?}"
    );
}

/// Resetting an environment is done standing on it: the test branch
/// offers to reset test and nothing else, so the menu cannot be used to
/// wipe an environment nobody is looking at.
#[test]
fn an_environment_branch_offers_to_reset_only_itself() {
    let mut state = state();
    state.branch = Some("test".into());
    state.branches[0].name = "test".into();
    let offered = available_actions(&state);
    assert!(offered.contains(&FlowAction::ResetTest), "{offered:?}");
    assert!(!offered.contains(&FlowAction::ResetDev), "{offered:?}");
}

/// An environment is not merged into another one: standing on test or
/// on the trunk there is nothing to release, only environments to reset.
#[test]
fn environments_are_not_released_into_each_other() {
    for checkout in ["test", "develop", "main"] {
        let mut state = state();
        state.branch = Some(checkout.into());
        state.branches[0].name = checkout.into();
        let offered = available_actions(&state);
        assert!(
            !offered.contains(&FlowAction::ReleaseTest)
                && !offered.contains(&FlowAction::ReleaseDev),
            "{checkout} must not be released: {offered:?}"
        );
    }
}

/// The steps shown are the steps run: both come from `workflow_steps`, so
/// what the menu promises cannot drift from what the progress list narrates.
#[test]
fn every_step_the_flow_would_run_is_listed() {
    let state = state();
    for action in available_actions(&state) {
        let run = flow_run(&state, action);
        let shown = text(&step_lines(&run));
        for step in steps_for(&run) {
            assert!(
                shown.contains(&step),
                "{action:?} does not list {step:?}: {shown}"
            );
        }
    }
}

/// A release is named after where it lands, so the steps have to name the
/// deploy branch rather than whatever branch happens to be checked out.
#[test]
fn the_steps_for_a_release_name_the_branch_it_lands_on() {
    let state = state();
    let steps = steps_for(&flow_run(&state, FlowAction::ReleaseTest)).join("\n");
    assert!(steps.contains("checkout test"), "{steps}");
    assert!(steps.contains("merge origin/feature/send-cv"), "{steps}");
}

/// Anything that throws work away is marked the same way, whatever it is
/// called: the menu should not need reading twice to spot them.
#[test]
fn destructive_actions_are_all_marked_in_red() {
    for action in [
        FlowAction::ResetDev,
        FlowAction::ResetTest,
        FlowAction::DiscardCheckout,
        FlowAction::CleanOrphans,
    ] {
        assert_eq!(action_mark(action).1, Color::Red, "{action:?}");
    }
    for action in [FlowAction::MergeMain, FlowAction::NewFeature] {
        assert_ne!(action_mark(action).1, Color::Red, "{action:?}");
    }
}
