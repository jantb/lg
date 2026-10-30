//! Worktrees in the repository panel: listing and entering them, the form
//! that adds one, and removing them.

use super::common::*;
use lg::panel::environments::menu::{self, RepoAction};

#[test]
fn repository_panel_lists_worktrees_without_repeating_the_active_checkout() {
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

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, true);
        })
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("feat/x"), "missing worktree branch: {text}");
    assert!(
        text.contains("feat-x"),
        "missing worktree directory: {text}"
    );
    assert_eq!(
        text.matches('\u{2387}').count(),
        1,
        "the active checkout already has the root row; it must not repeat as a worktree: {text}"
    );
}

#[test]
fn enter_on_a_worktree_row_switches_to_that_checkout() {
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
    panel::environments::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::SwitchRepository {
            target: RepoTarget::Path("/workspace.worktrees/feat-x".into())
        })
    );
}

#[test]
fn a_worktree_git_lost_track_of_cannot_be_entered() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        Worktree {
            prunable: Some("gitdir file points to non-existent location".into()),
            ..worktree("/workspace.worktrees/gone", "gone")
        },
    ];

    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::environments::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(state.pending_action, None);
    let status = state.status.as_ref().expect("status");
    assert!(status.is_error, "expected an error status: {status:?}");
    assert!(
        status.text.contains("missing"),
        "expected the status to explain why: {}",
        status.text
    );
}

#[test]
fn worktrees_hang_under_the_repository_they_belong_to() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.nested_repositories = vec![NestedRepo {
        path: "services/api".into(),
        branch: Some("main".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace/services/api", "main")
        },
        worktree("/workspace/services/api.worktrees/feat-x", "feat/x"),
    ];

    // Root, then the nested repository, then its worktree.
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::environments::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::SwitchRepository {
            target: RepoTarget::Path("/workspace/services/api.worktrees/feat-x".into())
        })
    );
}

#[test]
fn worktree_animation_keeps_the_form_still_and_stops_when_closed() {
    let mut state = AppState::new();
    state.repo_root = Some("/dev/lg".into());
    state.open_worktree_modal("origin/main".into());
    assert!(state.wants_animation());
    for (width, height) in [(100, 24), (76, 11), (28, 9), (20, 6), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        state.animation_ms = 0;
        terminal
            .draw(|frame| panel::worktree::render(&state, frame.area(), frame))
            .unwrap();
        let before = terminal.backend().buffer().clone();
        state.animation_ms = 600;
        terminal
            .draw(|frame| panel::worktree::render(&state, frame.area(), frame))
            .unwrap();
        let after = terminal.backend().buffer();
        let modal = lg::ui::centered(before.area, 76.min(width), 11.min(height));
        let mut changed = 0;
        for y in 0..height {
            for x in 0..width {
                assert_eq!(before[(x, y)].symbol(), after[(x, y)].symbol());
                if before[(x, y)] != after[(x, y)] {
                    assert!(
                        x == modal.x
                            || x == modal.right() - 1
                            || y == modal.y
                            || y == modal.bottom() - 1,
                        "animation changed a cell away from the border"
                    );
                    changed += 1;
                }
            }
        }
        if width >= 28 && height >= 9 {
            assert!(changed > 0, "border should move with elapsed time");
        } else {
            assert_eq!(changed, 0, "small-terminal fallback stays static");
        }
    }
    panel::worktree::handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    assert!(!state.wants_animation());
}

#[test]
fn the_worktree_form_derives_a_path_from_the_branch_until_it_is_edited() {
    let mut state = AppState::new();
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![Worktree {
        is_main: true,
        ..worktree("/dev/lg", "main")
    }];
    state.open_worktree_modal("origin/main".into());

    assert_eq!(state.modal, Modal::Worktree);
    for c in "feat/parser".chars() {
        panel::worktree::handle_key(&mut state, key(KeyCode::Char(c))).unwrap();
    }
    assert_eq!(state.worktree_form.path, "/dev/lg.worktrees/feat-parser");

    // Editing the path takes it over; the branch no longer drives it.
    panel::worktree::handle_key(&mut state, key(KeyCode::Tab)).unwrap();
    panel::worktree::handle_key(&mut state, key(KeyCode::Tab)).unwrap();
    panel::worktree::handle_key(&mut state, key(KeyCode::Char('2'))).unwrap();
    panel::worktree::handle_key(&mut state, key(KeyCode::Char('x'))).unwrap();
    assert_eq!(state.worktree_form.path, "/dev/lg.worktrees/feat-parser2x");

    panel::worktree::handle_key(&mut state, key(KeyCode::BackTab)).unwrap();
    panel::worktree::handle_key(&mut state, key(KeyCode::BackTab)).unwrap();
    panel::worktree::handle_key(&mut state, key(KeyCode::Backspace)).unwrap();
    assert_eq!(state.worktree_form.branch, "feat/parse");
    assert_eq!(
        state.worktree_form.path, "/dev/lg.worktrees/feat-parser2x",
        "an edited path must not snap back to the branch name"
    );
}

#[test]
fn the_worktree_form_turns_typed_spaces_into_dashes() {
    let mut state = AppState::new();
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![Worktree {
        is_main: true,
        ..worktree("/dev/lg", "main")
    }];
    state.open_worktree_modal("origin/main".into());

    for c in " parse  the   diff ".chars() {
        panel::worktree::handle_key(&mut state, key(KeyCode::Char(c))).unwrap();
    }
    assert_eq!(state.worktree_form.branch, "parse-the-diff-");
    assert_eq!(state.worktree_form.path, "/dev/lg.worktrees/parse-the-diff");

    panel::worktree::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::CreateWorktree {
            path: "/dev/lg.worktrees/parse-the-diff".into(),
            branch: "parse-the-diff-".into(),
            base: "origin/main".into(),
        })
    );
}

#[test]
fn the_worktree_form_creates_the_worktree_on_enter() {
    let mut state = AppState::new();
    state.repo_root = Some("/dev/lg".into());
    state.worktrees = vec![Worktree {
        is_main: true,
        ..worktree("/dev/lg", "main")
    }];
    state.open_worktree_modal("origin/main".into());
    for c in "feat/x".chars() {
        panel::worktree::handle_key(&mut state, key(KeyCode::Char(c))).unwrap();
    }
    panel::worktree::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(
        state.pending_action,
        Some(PendingAction::CreateWorktree {
            path: "/dev/lg.worktrees/feat-x".into(),
            branch: "feat/x".into(),
            base: "origin/main".into(),
        })
    );
}

#[test]
fn the_worktree_form_refuses_to_create_without_a_branch() {
    let mut state = AppState::new();
    state.repo_root = Some("/dev/lg".into());
    state.open_worktree_modal("origin/main".into());
    panel::worktree::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(state.modal, Modal::Worktree, "the form stays open");
    assert_eq!(state.pending_action, None);
    assert!(state.status.as_ref().expect("status").is_error);
}

#[test]
fn removing_a_dirty_worktree_confirms_and_forces() {
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
    menu::run(&mut state, RepoAction::RemoveWorktree);

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    let prompt = state.confirm.as_ref().expect("confirm prompt");
    assert!(
        prompt.detail.contains("uncommitted"),
        "the prompt must say what is lost: {}",
        prompt.detail
    );
    assert_eq!(
        prompt.action,
        PendingAction::RemoveWorktree {
            path: "/workspace.worktrees/feat-x".into(),
            force: true,
        }
    );
}

#[test]
fn the_main_checkout_is_not_removable_but_the_active_one_is() {
    // lg is showing a worktree whose main checkout sits outside the workspace,
    // so the main checkout gets a row of its own rather than being the root.
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/elsewhere/repo.worktrees/feat-x".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/elsewhere/repo", "main")
        },
        worktree("/elsewhere/repo.worktrees/feat-x", "feat/x"),
    ];

    // Row 1 is the main worktree, which git will not remove.
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    menu::run(&mut state, RepoAction::RemoveWorktree);
    assert_eq!(state.modal, Modal::None);
    assert!(
        state
            .status
            .as_ref()
            .expect("status")
            .text
            .contains("main checkout")
    );

    // Row 2 is the checkout lg is showing. Removing it is allowed: the
    // removal moves lg back to the main checkout before the directory goes.
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    menu::run(&mut state, RepoAction::RemoveWorktree);
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    let prompt = state.confirm.as_ref().expect("confirm prompt");
    assert_eq!(
        prompt.action,
        PendingAction::RemoveWorktree {
            path: "/elsewhere/repo.worktrees/feat-x".into(),
            force: false,
        }
    );
}

#[test]
fn a_scanned_worktree_of_the_active_repository_acts_as_a_worktree_row() {
    // The workspace scan finds the worktree directory like any repository. It
    // belongs to the active repository, so its row must offer the worktree
    // keys and sit under that repository rather than beside it.
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace/alviter.worktrees/worktree".into());
    state.nested_repositories = vec![
        NestedRepo {
            path: "alviter".into(),
            branch: Some("main".into()),
            detached_at: None,
            has_changes: false,
            worktree_of: None,
        },
        NestedRepo {
            path: "alviter.worktrees/worktree".into(),
            branch: Some("worktree".into()),
            detached_at: None,
            has_changes: false,
            worktree_of: Some("alviter".into()),
        },
    ];
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace/alviter", "main")
        },
        worktree("/workspace/alviter.worktrees/worktree", "worktree"),
    ];

    let backend = TestBackend::new(60, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, false);
        })
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }
    assert!(
        text.contains("\u{2387}"),
        "the worktree is drawn as a worktree, not as a repository: {text}"
    );
    assert!(
        !text.contains("alviter.worktrees/worktree"),
        "the worktree is named by its branch and directory, not its full path: {text}"
    );

    // Root, alviter, then the worktree row.
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    menu::run(&mut state, RepoAction::RemoveWorktree);
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(
        state.confirm.as_ref().expect("confirm prompt").action,
        PendingAction::RemoveWorktree {
            path: "/workspace/alviter.worktrees/worktree".into(),
            force: false,
        }
    );
}

#[test]
fn a_scanned_worktree_inside_the_workspace_checkout_can_be_removed() {
    // Worktrees kept inside the repository, the way `.claude/worktrees` is,
    // are found by the workspace scan. Their main checkout is the workspace
    // root rather than a nested repository, so the scan does not name one.
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.nested_repositories = vec![NestedRepo {
        path: ".claude/worktrees/feat-x".into(),
        branch: Some("claude/feat-x".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace/.claude/worktrees/feat-x", "claude/feat-x"),
    ];

    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::environments::handle_key(&mut state, key(KeyCode::Char('D'))).unwrap();

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(
        state.confirm.as_ref().expect("confirm prompt").action,
        PendingAction::RemoveWorktree {
            path: "/workspace/.claude/worktrees/feat-x".into(),
            force: false,
        }
    );
}

#[test]
fn removing_a_missing_worktree_prunes_instead() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        Worktree {
            prunable: Some("gitdir file points to non-existent location".into()),
            ..worktree("/workspace.worktrees/gone", "gone")
        },
    ];

    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    menu::run(&mut state, RepoAction::RemoveWorktree);

    assert_eq!(state.pending_action, Some(PendingAction::PruneWorktrees));
}

/// D removes a worktree straight from the tree, the way it deletes a branch
/// in the branches pane, without going through the menu.
#[test]
fn shift_d_on_a_worktree_row_asks_to_remove_it() {
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
    panel::environments::handle_key(&mut state, key(KeyCode::Char('D'))).unwrap();

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(
        state.confirm.as_ref().expect("confirm prompt").action,
        PendingAction::RemoveWorktree {
            path: "/workspace.worktrees/feat-x".into(),
            force: false,
        }
    );
}
