use super::common::*;

// ── Navigation ────────────────────────────────────────────────────────────────

#[test]
fn files_panel_navigation_moves_selection() {
    let mut state = make_state_with_files();
    assert_eq!(state.files_idx, 0);
    panel::files::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(state.files_idx, 1);
}

#[test]
fn files_panel_k_moves_selection_up() {
    let mut state = make_state_with_files();
    state.files_idx = 2;
    panel::files::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();
    assert_eq!(state.files_idx, 1);
}

#[test]
fn files_panel_keeps_context_below_selected_row_while_scrolling() {
    let mut state = AppState::new();
    state.focus = Pane::Files;
    state.files_idx = 8;
    state.files = (0..14)
        .map(|idx| FileEntry {
            path: format!("file_{idx:02}.rs"),
            x: ' ',
            y: 'M',
        })
        .collect();

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::files::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let row_text = |row_idx: u16| {
        let mut row = String::new();
        for col in 0..buf.area.width {
            row.push_str(buf[(col, row_idx)].symbol());
        }
        row
    };
    let selected_row = (0..buf.area.height)
        .find(|row| row_text(*row).contains("file_07.rs"))
        .expect("selected file should be visible");

    assert!(
        selected_row < buf.area.height - 2,
        "selected file should not stick to the bottom:\n{}",
        (0..buf.area.height)
            .map(row_text)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(row_text(selected_row + 1).contains("file_08.rs"));
    assert!(row_text(selected_row + 2).contains("file_09.rs"));
}

#[test]
fn scroll_handlers_clamp_stale_indices_before_moving() {
    let mut state = make_state_with_files();
    state.files_idx = usize::MAX;
    panel::files::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(state.files_idx, state.tree_rows().len() - 1);

    state.branches = vec![
        Branch {
            name: "main".into(),
            is_current: true,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "feature".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
    ];
    state.branches_idx = usize::MAX;
    panel::branches::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(state.branches_idx, 1);

    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["parent".into()],
        is_first_parent: true,
        subject: "initial".into(),
    }];
    state.commits_idx = usize::MAX;
    panel::commits::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(state.commits_idx, 0);

    state.conflicts = vec!["src/lib.rs".into()];
    state.conflict_idx = usize::MAX;
    panel::conflict::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(state.conflict_idx, 0);

    add_flow_branches(&mut state);
    state.release_branches = ReleaseBranches::new(Some("develop".into()), Some("test".into()));
    state.branch = Some("feature/demo".into());
    state.flow_idx = usize::MAX;
    panel::flow::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert!(state.flow_idx < lg::state::FlowAction::ALL.len());
}

#[test]
fn scroll_handlers_clamp_stale_indices_before_moving_up() {
    let mut state = make_state_with_files();
    state.files_idx = usize::MAX;
    panel::files::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();
    assert_eq!(state.files_idx, state.tree_rows().len() - 2);

    state.branches = vec![
        Branch {
            name: "main".into(),
            is_current: true,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "feature".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
    ];
    state.branches_idx = usize::MAX;
    panel::branches::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();
    assert_eq!(state.branches_idx, 0);

    state.commits = vec![
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["parent".into()],
            is_first_parent: true,
            subject: "top".into(),
        },
        Commit {
            sha: "def5678".into(),
            author: "Bob Example".into(),
            author_short: "BE".into(),
            parents: vec!["abc1234".into()],
            is_first_parent: true,
            subject: "bottom".into(),
        },
    ];
    state.commits_idx = usize::MAX;
    panel::commits::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();
    assert_eq!(state.commits_idx, 0);

    state.conflicts = vec!["src/lib.rs".into(), "src/main.rs".into()];
    state.conflict_idx = usize::MAX;
    panel::conflict::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();
    assert_eq!(state.conflict_idx, 0);

    add_flow_branches(&mut state);
    state.release_branches = ReleaseBranches::new(Some("develop".into()), Some("test".into()));
    state.branch = Some("feature/demo".into());
    state.flow_idx = usize::MAX;
    panel::flow::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();
    assert!(state.flow_idx < lg::state::FlowAction::ALL.len());
}

#[test]
fn branch_without_upstream_and_local_commits_can_push() {
    let mut state = AppState::new();
    state.branch = Some("main".into());
    state.ahead_behind = None;
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Test User".into(),
        author_short: "TU".into(),
        parents: vec![],
        is_first_parent: true,
        subject: "initial commit".into(),
    }];

    assert!(state.has_unpushed_commits());
}

#[test]
fn conflict_modal_o_opens_selected_conflicted_file() {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.conflicts = vec!["src/a.rs".into(), "src/b.rs".into()];

    panel::conflict::handle_key(&mut state, key(KeyCode::Down)).unwrap();
    panel::conflict::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenFile("src/b.rs".into()))
    );
}

#[test]
fn conflict_modal_enter_opens_selected_conflicted_file() {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.conflicts = vec!["src/conflict.rs".into()];

    panel::conflict::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenFile("src/conflict.rs".into()))
    );
}

#[test]
fn files_panel_o_opens_selected_source_file() {
    let mut state = make_state_with_files();
    state.files = vec![FileEntry {
        path: "main.rs".into(),
        x: ' ',
        y: 'M',
    }];
    state.files_idx = 1;

    panel::files::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenFile("main.rs".into()))
    );
}

#[test]
fn files_panel_o_opens_project_from_top_level_or_folder() {
    let mut state = make_state_with_files();

    panel::files::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();
    assert_eq!(state.pending_action, Some(PendingAction::OpenProject));

    state.pending_action = None;
    state.files = vec![FileEntry {
        path: "src/main.rs".into(),
        x: ' ',
        y: 'M',
    }];
    state.files_idx = 1;

    panel::files::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();
    assert_eq!(state.pending_action, Some(PendingAction::OpenProject));
}

#[test]
fn files_panel_i_ignores_selected_file_or_folder() {
    let mut state = AppState::new();
    state.files = vec![FileEntry {
        path: "src/main.rs".into(),
        x: '?',
        y: '?',
    }];

    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Char('i'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::IgnorePath {
            path: "src".into(),
            is_dir: true,
        })
    );

    state.pending_action = None;
    state.files_idx = 2;
    panel::files::handle_key(&mut state, key(KeyCode::Char('i'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::IgnorePath {
            path: "src/main.rs".into(),
            is_dir: false,
        })
    );
}

#[test]
fn files_panel_d_deletes_selected_file_or_folder_after_confirmation() {
    let mut state = AppState::new();
    state.files = vec![FileEntry {
        path: "src/main.rs".into(),
        x: '?',
        y: '?',
    }];

    panel::files::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();
    assert_eq!(state.pending_action, None);
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|status| status.text.contains("select a file or folder"))
    );

    state.status = None;
    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(state.pending_action, None);
    panel::confirm::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::DeletePath {
            path: "src".into(),
            is_dir: true,
        })
    );

    state.pending_action = None;
    state.files_idx = 2;
    panel::files::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(state.pending_action, None);
    panel::confirm::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::DeletePath {
            path: "src/main.rs".into(),
            is_dir: false,
        })
    );
}

#[test]
fn files_panel_r_rolls_back_selected_file_or_folder_after_confirmation() {
    let mut state = AppState::new();
    state.files = vec![FileEntry {
        path: "src/main.rs".into(),
        x: ' ',
        y: 'M',
    }];

    panel::files::handle_key(&mut state, key(KeyCode::Char('r'))).unwrap();
    assert_eq!(state.pending_action, None);
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|status| status.text.contains("select a file or folder"))
    );

    state.status = None;
    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Char('r'))).unwrap();
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(state.pending_action, None);
    panel::confirm::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::RollbackPath {
            path: "src".into(),
            is_dir: true,
        })
    );

    state.pending_action = None;
    state.files_idx = 2;
    panel::files::handle_key(&mut state, key(KeyCode::Char('r'))).unwrap();
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(state.pending_action, None);
    panel::confirm::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::RollbackPath {
            path: "src/main.rs".into(),
            is_dir: false,
        })
    );
}

// ── Jumping through a list ────────────────────────────────────────────────────

fn app_with_many_files(rows: usize) -> lg::app::HeadlessApp<TestBackend> {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    app.state.files = (0..rows)
        .map(|i| FileEntry {
            path: format!("f{i:02}.rs"),
            x: ' ',
            y: 'M',
        })
        .collect();
    app.state.focus = Pane::Files;
    app.render().unwrap();
    app
}

#[test]
fn g_and_shift_g_jump_to_the_ends_of_a_list() {
    let mut app = app_with_many_files(40);
    let last = app.state.tree_rows().len() - 1;

    app.send_key(key(KeyCode::Char('G'))).unwrap();
    assert_eq!(app.state.files_idx, last, "G should land on the last row");

    app.send_key(key(KeyCode::Char('g'))).unwrap();
    assert_eq!(app.state.files_idx, 0, "g should land on the first row");
}

#[test]
fn page_keys_move_further_than_a_single_step() {
    let mut app = app_with_many_files(40);

    app.send_key(key(KeyCode::PageDown)).unwrap();
    let paged = app.state.files_idx;
    assert!(
        paged > 1,
        "PageDown should move more than one row, moved to {paged}"
    );

    app.send_key(key(KeyCode::PageUp)).unwrap();
    assert_eq!(app.state.files_idx, 0, "PageUp should come back to the top");
}

#[test]
fn ctrl_d_pages_the_file_list_instead_of_deleting_a_file() {
    let mut app = app_with_many_files(40);

    app.send_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(
        app.state.modal,
        Modal::None,
        "Ctrl-d must not fall through to the plain d that deletes a file"
    );
    assert!(
        app.state.files_idx > 0,
        "Ctrl-d should have moved the selection down a half page"
    );

    let down = app.state.files_idx;
    app.send_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(
        app.state.files_idx < down,
        "Ctrl-u should move back up rather than unstaging"
    );
}

#[test]
fn a_jump_in_the_commits_pane_skips_the_graph_rows() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    app.state.commits = (0..30)
        .map(|i| Commit {
            sha: format!("{i:07}a"),
            author: "a@b.c".into(),
            author_short: "ab".into(),
            subject: format!("commit {i}"),
            parents: Vec::new(),
            is_first_parent: true,
        })
        .collect();
    app.state.focus = Pane::Commits;
    app.render().unwrap();

    app.send_key(key(KeyCode::Char('G'))).unwrap();

    let landed = &app.state.commits[app.state.commits_idx];
    assert!(!landed.is_graph_row(), "G must land on a real commit row");
}
