//! Handing a worktree's branch over: landing it, merging main into it, and
//! bringing it home.

use super::common::*;
use lg::panel::environments::menu::{self, RepoAction};

/// A workspace whose only linked worktree is on `feat/x`, with the tree
/// selection already moved onto it.
fn state_on_a_worktree_row() -> AppState {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace.worktrees/feat-x", "feat/x"),
    ];
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    state
}

#[test]
fn m_on_a_worktree_row_parks_the_land_behind_a_confirmation() {
    let mut state = state_on_a_worktree_row();

    menu::run(&mut state, RepoAction::LandWorktree);

    assert_eq!(state.pending_action, None, "nothing runs before the y/n");
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    let confirm = state.confirm.as_ref().expect("confirm prompt");
    assert_eq!(
        confirm.action,
        PendingAction::LandWorktree {
            path: "/workspace.worktrees/feat-x".into(),
            branch: "feat/x".into(),
        }
    );
    assert!(
        confirm.detail.contains("origin/feat/x"),
        "the prompt says the remote branch goes too: {}",
        confirm.detail
    );
}

#[test]
fn shift_m_on_a_worktree_row_offers_to_merge_main_into_it() {
    let mut state = state_on_a_worktree_row();

    menu::run(&mut state, RepoAction::SyncWorktree);

    assert_eq!(state.pending_action, None, "nothing runs before the y/n");
    assert_eq!(
        state.confirm.as_ref().expect("confirm prompt").action,
        PendingAction::SyncWorktree {
            path: "/workspace.worktrees/feat-x".into(),
            branch: "feat/x".into(),
        }
    );
    let detail = &state.confirm.as_ref().expect("confirm prompt").detail;
    assert!(
        detail.contains("main is not touched"),
        "the prompt says main is safe: {detail}"
    );
}

#[test]
fn b_on_a_worktree_row_offers_to_move_the_branch_home() {
    let mut state = state_on_a_worktree_row();

    menu::run(&mut state, RepoAction::BranchHome);

    let confirm = state.confirm.as_ref().expect("confirm prompt");
    assert_eq!(
        confirm.action,
        PendingAction::BringWorktreeHome {
            path: "/workspace.worktrees/feat-x".into(),
            branch: "feat/x".into(),
        }
    );
    assert!(
        confirm.detail.contains("nothing is merged"),
        "the prompt separates this from landing: {}",
        confirm.detail
    );
}

#[test]
fn a_handover_is_refused_before_the_confirmation_when_the_worktree_is_dirty() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        Worktree {
            has_changes: true,
            ..worktree("/workspace.worktrees/feat-x", "feat/x")
        },
    ];
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();

    menu::run(&mut state, RepoAction::LandWorktree);

    assert!(state.confirm.is_none(), "no prompt for a dirty worktree");
    assert_eq!(state.pending_action, None);
    let status = state.status.as_ref().expect("status");
    assert!(status.is_error, "expected an error status: {status:?}");
    assert!(
        status.text.contains("commit or discard"),
        "the status names the way out: {}",
        status.text
    );
}

#[test]
fn the_handover_keys_do_nothing_on_the_root_row() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace.worktrees/feat-x", "feat/x"),
    ];

    for code in [RepoAction::LandWorktree, RepoAction::BranchHome] {
        menu::run(&mut state, code);
        assert!(state.confirm.is_none(), "{code:?} needs a worktree row");
        let status = state.status.as_ref().expect("status");
        assert!(
            status.text.contains("select a worktree row"),
            "the status says what to select: {}",
            status.text
        );
    }
}

#[test]
fn a_worktree_row_says_how_much_landing_it_would_move() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        Worktree {
            unmerged: Some(3),
            ..worktree("/workspace.worktrees/feat-x", "feat/x")
        },
        Worktree {
            unmerged: Some(0),
            ..worktree("/workspace.worktrees/done", "feat/done")
        },
        Worktree {
            unmerged: None,
            ..worktree("/workspace.worktrees/loose", "feat/loose")
        },
    ];

    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| panel::environments::render(&state, frame.area(), frame, true))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
        text.push('\n');
    }

    assert!(
        text.contains("\u{2191}3"),
        "three commits to land should show as a count: {text}"
    );
    assert!(
        text.contains("merged"),
        "a branch main already has should say so: {text}"
    );
    let loose = text
        .lines()
        .find(|line| line.contains("feat/loose"))
        .expect("the row with no answer is still drawn");
    assert!(
        !loose.contains('\u{2191}') && !loose.contains("merged"),
        "an unanswerable row claims nothing: {loose}"
    );
}
