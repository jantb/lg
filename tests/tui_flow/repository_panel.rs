//! The repository panel: repositories, nested ones and their branches, and
//! the mouse over it.

use super::common::*;

#[test]
fn repository_panel_marks_the_active_repository() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace/services/api".into());
    state.nested_repositories = vec![
        NestedRepo {
            path: "services/api".into(),
            branch: Some("feature/api".into()),
            detached_at: None,
            has_changes: false,
            worktree_of: None,
        },
        NestedRepo {
            path: "services/web".into(),
            branch: Some("main".into()),
            detached_at: None,
            has_changes: false,
            worktree_of: None,
        },
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
        text.contains("Repositories \u{2022} api"),
        "panel title should name the active repository: {text}"
    );
    assert!(
        text.contains("* services/api"),
        "active repository row should be marked: {text}"
    );
    assert!(
        text.contains("  services/web"),
        "inactive repository row should not be marked: {text}"
    );
}

#[test]
fn repository_panel_shows_nested_repository_branches() {
    let mut state = AppState::new();
    state.nested_repositories = vec![
        NestedRepo {
            path: "services/api".into(),
            branch: Some("feature/api".into()),
            detached_at: None,
            has_changes: true,
            worktree_of: None,
        },
        NestedRepo {
            path: "libs/core".into(),
            branch: None,
            detached_at: Some("abc1234".into()),
            has_changes: false,
            worktree_of: None,
        },
    ];

    let backend = TestBackend::new(80, 8);
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

    assert!(text.contains("Repositories"), "missing repo panel: {text}");
    assert!(text.contains("services/api"), "missing repo path: {text}");
    assert!(text.contains("feature/api"), "missing branch: {text}");
    assert!(
        text.contains("detached@abc1234"),
        "missing detached ref: {text}"
    );
    assert!(text.contains("!"), "missing dirty marker: {text}");
}

#[test]
fn repository_panel_highlights_active_nested_repository_background() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace/services/api".into());
    state.nested_repositories = vec![
        NestedRepo {
            path: "services/api".into(),
            branch: Some("main".into()),
            detached_at: None,
            has_changes: false,
            worktree_of: None,
        },
        NestedRepo {
            path: "libs/core".into(),
            branch: Some("main".into()),
            detached_at: None,
            has_changes: false,
            worktree_of: None,
        },
    ];

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let active_bg = Some(Color::Rgb(24, 54, 34));
    let mut active_repo_has_background = false;
    let mut inactive_repo_has_background = false;

    for row in 0..buf.area.height {
        let mut row_text = String::new();
        for col in 0..buf.area.width {
            row_text.push_str(buf[(col, row)].symbol());
        }

        if row_text.contains("services/api") {
            active_repo_has_background =
                (0..buf.area.width).any(|col| buf[(col, row)].style().bg == active_bg);
        }
        if row_text.contains("libs/core") {
            inactive_repo_has_background =
                (0..buf.area.width).any(|col| buf[(col, row)].style().bg == active_bg);
        }
    }

    assert!(
        active_repo_has_background,
        "active repository row should use active background"
    );
    assert!(
        !inactive_repo_has_background,
        "inactive repository row should not use active background"
    );
}

#[test]
fn repository_panel_tree_shows_nested_branch_lists() {
    let mut state = AppState::new();
    state.nested_repositories = vec![NestedRepo {
        path: "services/api".into(),
        branch: Some("main".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];
    state.nested_repo_detail_path = Some("services/api".into());
    state.nested_repo_branches = vec![
        Branch {
            name: "main".into(),
            is_current: true,
            upstream: Some("origin/main".into()),
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "feature/api".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 1,
            behind: 2,
            behind_main: 0,
            last_commit_unix: None,
        },
    ];
    state.nested_repo_remote_branches = vec![RemoteBranch {
        name: "origin/feature/remote".into(),
        remote: "origin".into(),
        local_name: "feature/remote".into(),
        last_commit_unix: None,
    }];

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

    assert!(text.contains("Repositories"), "missing tree title: {text}");
    assert!(text.contains("services/api"), "missing repo row: {text}");
    assert!(text.contains("* main"), "missing current branch: {text}");
    assert!(
        text.contains("feature/api"),
        "missing nested feature branch: {text}"
    );

    panel::environments::handle_key(&mut state, key(KeyCode::Char('r'))).unwrap();
    assert_eq!(state.nested_repo_branch_view, BranchView::Remote);
}

#[test]
fn clicking_repository_row_switches_repository() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    app.state.nested_repositories = vec![NestedRepo {
        path: "services/api".into(),
        branch: Some("feature/api".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];

    let area = Rect::new(0, 0, 120, 32);
    let rects = lg::ui::split_layout_with_sizes(
        area,
        app.state.environments_visible(),
        app.state.left_column_width,
        app.state.left_panel_heights,
    );
    app.send_mouse(left_click(
        rects.environments.x.saturating_add(2),
        rects.environments.y.saturating_add(2),
    ))
    .unwrap();

    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::SwitchRepository {
            target: RepoTarget::Nested("services/api".into())
        })
    );
}

#[test]
fn repositories_o_opens_selected_repo_or_branch_project() {
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
    state.nested_repo_detail_path = Some("services/api".into());
    state.nested_repo_branches = vec![Branch {
        name: "feature/api".into(),
        is_current: false,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    state.nested_repo_tree_idx = 1;
    panel::environments::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenProjectAt(
            "/workspace/services/api".into()
        ))
    );

    state.pending_action = None;
    state.nested_repo_tree_idx = 2;
    panel::environments::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenProjectAt(
            "/workspace/services/api".into()
        ))
    );
}

#[test]
fn repository_panel_keeps_deployment_status_visible_in_full_layout() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 34)).unwrap();
    app.state.repo_root = Some("/tmp/work/lg/services/api".into());
    app.state.workspace_root = Some("/tmp/work/lg".into());
    app.state.branch = Some("feature/api".into());
    add_flow_branches(&mut app.state);
    app.state.nested_repositories = vec![NestedRepo {
        path: "services/api".into(),
        branch: Some("feature/api".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];
    app.state.current_branch_releases = BranchReleaseStatus {
        environments: Default::default(),
        main: None,
        develop: Some(ReleaseTargetStatus {
            released_at: "2026-04-29 14:20".into(),
            missing_commits: 0,
        }),
        test: Some(ReleaseTargetStatus {
            released_at: "2026-04-29 14:25".into(),
            missing_commits: 2,
        }),
    };

    app.render().unwrap();

    let text = buffer_text(&app);
    assert!(text.contains("Repositories"), "missing repo panel: {text}");
    assert!(
        text.contains("Branch inclusion"),
        "missing deployment status panel: {text}"
    );
    assert!(text.contains("services/api"), "missing nested repo: {text}");
    assert!(
        text.contains("2026-04-29 14:20"),
        "missing develop deployment timestamp: {text}"
    );
    assert!(
        text.contains("test"),
        "missing test deployment target: {text}"
    );
    assert!(
        text.contains("2026-04-29 14:25"),
        "missing test deployment timestamp: {text}"
    );
    assert!(text.contains("+2"), "missing test pending distance: {text}");
}

#[test]
fn repository_panel_accepts_mouse_focus_without_flow_branches() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state.focus = Pane::Branches;
    app.state.nested_repositories = vec![NestedRepo {
        path: "services/api".into(),
        branch: Some("feature/api".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];

    app.render().unwrap();
    let rects = lg::ui::split_layout_with_sizes(
        Rect::new(0, 0, 80, 24),
        app.state.environments_visible(),
        app.state.left_column_width,
        app.state.left_panel_heights,
    );
    app.send_mouse(left_click(2, rects.environments.y + 2))
        .unwrap();

    assert_eq!(app.state.focus, Pane::Status);
    assert_eq!(app.state.nested_repo_tree_idx, 1);
}

#[test]
fn repository_panel_divider_can_be_dragged_without_flow_branches() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state.nested_repositories = vec![NestedRepo {
        path: "services/api".into(),
        branch: Some("feature/api".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];

    app.render().unwrap();
    let rects = lg::ui::split_layout_with_sizes(
        Rect::new(0, 0, 80, 24),
        app.state.environments_visible(),
        app.state.left_column_width,
        app.state.left_panel_heights,
    );
    app.send_mouse(left_click(2, rects.files.y)).unwrap();
    app.send_mouse(left_drag(2, rects.files.y.saturating_sub(2)))
        .unwrap();

    let heights = app
        .state
        .left_panel_heights
        .expect("drag should save left panel heights");
    assert!(
        heights[1] < rects.environments.height,
        "environment height should shrink after dragging up: {heights:?}"
    );
    assert!(
        heights[2] > rects.files.height,
        "files height should grow after dragging up: {heights:?}"
    );
}

#[test]
fn repository_panel_tree_esc_collapses_expanded_repo() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 8)).unwrap();
    app.state.focus = Pane::Status;
    app.state.nested_repositories = vec![NestedRepo {
        path: "services/api".into(),
        branch: Some("main".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }];
    app.state.nested_repo_detail_path = Some("services/api".into());
    app.state.nested_repo_branches = vec![Branch {
        name: "main".into(),
        is_current: true,
        upstream: Some("origin/main".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];
    app.state.nested_repo_tree_idx = 1;

    app.send_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.state.nested_repo_detail_path, None);
    assert!(app.state.nested_repo_branches.is_empty());
}
