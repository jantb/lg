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
    state.flow_branches_available = true;
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
    state.flow_branches_available = true;
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
fn files_panel_d_deletes_selected_file_or_folder() {
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
    assert_eq!(
        state.pending_action,
        Some(PendingAction::DeletePath {
            path: "src/main.rs".into(),
            is_dir: false,
        })
    );
}
