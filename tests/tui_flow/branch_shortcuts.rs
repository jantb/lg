//! The merge, sync and delete shortcuts on the branches panel.

use super::common::*;

/// Answer the confirmation the last key raised with `y`.
fn confirm_prompt(state: &mut AppState) {
    assert_eq!(
        state.modal,
        Modal::ConfirmDestructive,
        "the shortcut should ask before it merges and pushes"
    );
    assert_eq!(state.pending_action, None, "nothing runs before the answer");
    panel::confirm::handle_key(state, key(KeyCode::Char('y'))).unwrap();
}

#[test]
fn branches_m_shortcut_queues_merge_main_workflow() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branch = Some("feature/demo".into());
    state.branches = vec![Branch {
        name: "feature/demo".into(),
        is_current: true,
        upstream: Some("origin/feature/demo".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 4,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();
    confirm_prompt(&mut state);

    assert_eq!(
        state.pending_action,
        Some(PendingAction::Flow(FlowAction::MergeMain))
    );
}

#[test]
fn branches_m_shortcut_allows_develop_when_behind_main() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branch = Some("develop".into());
    state.branches = vec![Branch {
        name: "develop".into(),
        is_current: true,
        upstream: Some("origin/develop".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 3,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();
    confirm_prompt(&mut state);

    assert_eq!(
        state.pending_action,
        Some(PendingAction::Flow(FlowAction::MergeMain))
    );
}

#[test]
fn branches_m_shortcut_pulls_main_when_behind_remote() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branch = Some("main".into());
    state.branches = vec![Branch {
        name: "main".into(),
        is_current: true,
        upstream: Some("origin/main".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 3,
        behind_main: 0,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();

    assert_eq!(state.pending_action, Some(PendingAction::Pull));
}

#[test]
fn branches_m_shortcut_blocks_release_next_when_not_behind_main() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branch = Some("test".into());
    state.branches = vec![Branch {
        name: "test".into(),
        is_current: true,
        upstream: Some("origin/test".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();

    assert_eq!(state.pending_action, None);
    assert_eq!(
        state.status.as_ref().map(|s| s.text.as_str()),
        Some("current branch is not behind origin/main")
    );
}

#[test]
fn branches_shift_m_shortcut_queues_sync_all_branches() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;

    panel::branches::handle_key(&mut state, key(KeyCode::Char('M'))).unwrap();
    confirm_prompt(&mut state);

    assert_eq!(
        state.pending_action,
        Some(PendingAction::MergeMainAllBranches)
    );
}

#[test]
fn branches_shift_modifier_m_shortcut_queues_sync_all_branches() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;

    panel::branches::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::SHIFT),
    )
    .unwrap();
    confirm_prompt(&mut state);

    assert_eq!(
        state.pending_action,
        Some(PendingAction::MergeMainAllBranches)
    );
}

/// Syncing every branch pushes branches that are not checked out, so the
/// prompt has to say which ones before anything is merged.
#[test]
fn branches_shift_m_names_every_branch_it_will_merge_and_push() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    let branch = |name: &str, upstream: Option<&str>| Branch {
        name: name.into(),
        is_current: false,
        upstream: upstream.map(Into::into),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    };
    state.branches = vec![
        branch("main", Some("origin/main")),
        branch("feature/tracked", Some("origin/feature/tracked")),
        branch("feature/local", None),
        branch("lg/backup/merge-main-feature-1", None),
    ];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('M'))).unwrap();

    let prompt = state.confirm.as_ref().expect("a confirmation prompt");
    let shown = format!("{}\n{}", prompt.question, prompt.detail);
    assert!(shown.contains("feature/tracked"), "{shown}");
    assert!(shown.contains("origin/feature/tracked"), "{shown}");
    assert!(shown.contains("feature/local"), "{shown}");
    assert!(
        !shown.contains("lg/backup"),
        "backups are not synced, so they are not named: {shown}"
    );

    panel::confirm::handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();
    assert_eq!(state.pending_action, None, "declining runs nothing");
}

#[test]
fn branches_d_shortcut_deletes_local_only_branch() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches = vec![Branch {
        name: "feature/local-only".into(),
        is_current: true,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();

    assert_eq!(state.modal, Modal::DeleteBranch);
    assert_eq!(state.delete_branch.target, "feature/local-only");
    assert!(state.delete_branch.local);
    assert!(!state.delete_branch.remote);
}

#[test]
fn branches_d_shortcut_blocks_protected_branch() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches = vec![Branch {
        name: "main".into(),
        is_current: true,
        upstream: Some("origin/main".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();

    assert_eq!(state.pending_action, None);
    assert_eq!(
        state.status.as_ref().map(|status| status.text.as_str()),
        Some("cannot delete protected branch main")
    );
}

#[test]
fn branches_d_shortcut_blocks_tracked_branch() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches = vec![Branch {
        name: "feature/tracked".into(),
        is_current: false,
        upstream: Some("origin/feature/tracked".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();

    assert_eq!(state.pending_action, None);
    assert_eq!(
        state.status.as_ref().map(|status| status.text.as_str()),
        Some("branch has a remote; use D for delete options")
    );
}

#[test]
fn branches_d_modal_allows_current_feature_branch() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches = vec![Branch {
        name: "feature/current".into(),
        is_current: true,
        upstream: Some("origin/feature/current".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('D'))).unwrap();

    assert_eq!(state.modal, Modal::DeleteBranch);
    assert_eq!(state.delete_branch.target, "feature/current");
}

#[test]
fn delete_branch_modal_hides_remote_option_for_local_only_branch() {
    let mut state = AppState::new();
    let branch = Branch {
        name: "lg/backup/merge-main-feature-demo-123".into(),
        is_current: false,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    };
    state.open_delete_branch_modal(&branch);

    let backend = TestBackend::new(100, 18);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::delete_branch::render(&state, frame.area(), frame);
        })
        .unwrap();

    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        !text.contains("delete remote"),
        "local-only branch should not show remote delete option: {text}"
    );
    assert!(
        text.contains("delete local branch"),
        "local-only branch should show fixed local delete action: {text}"
    );
    assert!(
        !text.contains("[x] delete local"),
        "local-only branch should not render delete local as a peer checkbox: {text}"
    );
    assert!(
        text.contains("force local delete"),
        "force option should explain that it is local-only: {text}"
    );

    assert_eq!(
        state.delete_branch.field,
        lg::state::DeleteBranchField::Force
    );
}

#[test]
fn delete_branch_modal_shows_remote_option_for_tracked_branch() {
    let mut state = AppState::new();
    let branch = Branch {
        name: "feature/tracked".into(),
        is_current: false,
        upstream: Some("origin/feature/tracked".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    };
    state.open_delete_branch_modal(&branch);

    let backend = TestBackend::new(100, 18);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::delete_branch::render(&state, frame.area(), frame);
        })
        .unwrap();

    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        text.contains("delete remote (origin)"),
        "tracked branch should show remote delete option: {text}"
    );
    assert!(
        !state.delete_branch.remote,
        "deleting the remote is offered, not chosen for the user"
    );
}
