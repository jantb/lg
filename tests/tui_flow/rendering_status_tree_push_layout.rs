use super::common::*;

// ── XY-rendering smoke test ───────────────────────────────────────────────────

#[test]
fn files_panel_renders_xy_codes_with_color() {
    let state = make_state_with_files();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::files::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();

    // Find the 'M' cell (y-side modification for a.rs) and check it's yellow.
    let found_yellow_m = (0..buf.area.height).any(|row| {
        (0..buf.area.width).any(|col| {
            let cell = &buf[(col, row)];
            cell.symbol() == "M" && cell.style().fg == Some(Color::Yellow)
        })
    });
    assert!(
        found_yellow_m,
        "expected a yellow 'M' cell for modified file"
    );

    // Untracked files render as an unstaged marker rather than raw "??".
    let found_unstaged = (0..buf.area.height).any(|row| {
        (0..buf.area.width.saturating_sub("unstaged".len() as u16)).any(|col| {
            (0.."unstaged".len()).all(|offset| {
                let cell = &buf[((col + offset as u16), row)];
                cell.symbol() == &"unstaged"[offset..offset + 1]
                    && cell.style().fg == Some(Color::DarkGray)
            })
        })
    });
    assert!(
        found_unstaged,
        "expected a grey unstaged marker for untracked file"
    );
}

#[test]
fn status_panel_renders_change_counts() {
    let state = make_state_with_files();
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::status::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("3 files"), "missing total file count: {text}");
    assert!(text.contains("S1"), "missing staged count: {text}");
    assert!(text.contains("U1"), "missing unstaged count: {text}");
    assert!(text.contains("?1"), "missing untracked count: {text}");
}

#[test]
fn status_panel_shows_active_generation() {
    let mut state = make_state_with_files();
    let (_tx, rx) = std::sync::mpsc::channel::<lg::state::GenMsg>();
    state.generation = Some(lg::state::Generation {
        rx,
        handle: None,
        output: String::new(),
        spinner: 0,
    });

    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::status::render(&state, frame.area(), frame, false);
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
        text.contains("generating"),
        "expected active generation cue: {text}"
    );
}

#[test]
fn current_branch_panel_renders_environment_history() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("feature/released".into());
    state.current_branch_releases = BranchReleaseStatus {
        main: None,
        develop: Some(ReleaseTargetStatus {
            released_at: "2026-04-29 14:20".into(),
            missing_commits: 2,
        }),
        test: Some(ReleaseTargetStatus {
            released_at: "2026-04-29 14:25".into(),
            missing_commits: 0,
        }),
    };

    let backend = TestBackend::new(90, 8);
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
        text.contains("Deployment Status"),
        "missing deployment status panel: {text}"
    );
    assert!(
        text.contains("feature/released"),
        "missing branch name: {text}"
    );
    assert!(text.contains("develop"), "missing develop badge: {text}");
    assert!(
        text.contains("release/next"),
        "missing release/next badge: {text}"
    );
    assert!(
        text.contains("2026-04-29 14:20"),
        "missing release timestamp: {text}"
    );
    assert!(text.contains("+2"), "missing pending count: {text}");
}

#[test]
fn current_branch_panel_shows_deployment_status_loading() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("feature/loading".into());
    let (_tx, rx) = mpsc::channel();
    state.release_status_job = Some(ReleaseStatusJob {
        rx,
        handle: None,
        spinner: 0,
        branch: "feature/loading".into(),
    });

    let backend = TestBackend::new(90, 8);
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

    assert!(text.contains("checking"), "missing loading state: {text}");
}

#[test]
fn current_branch_panel_hides_environment_history_without_release_branches() {
    let state = AppState::new();
    let backend = TestBackend::new(90, 8);
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
        !text.contains("Deployment Status"),
        "environment box should be hidden: {text}"
    );
    assert!(!text.contains("dev"), "dev status should be hidden: {text}");
    assert!(
        !text.contains("test"),
        "test status should be hidden: {text}"
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
        },
        NestedRepo {
            path: "libs/core".into(),
            branch: None,
            detached_at: Some("abc1234".into()),
            has_changes: false,
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
        },
        NestedRepo {
            path: "libs/core".into(),
            branch: Some("main".into()),
            detached_at: None,
            has_changes: false,
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
            path: Some("services/api".into())
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
    }];
    app.state.current_branch_releases = BranchReleaseStatus {
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
        text.contains("Deployment Status"),
        "missing deployment status panel: {text}"
    );
    assert!(text.contains("services/api"), "missing nested repo: {text}");
    assert!(
        text.contains("2026-04-29 14:20"),
        "missing develop deployment timestamp: {text}"
    );
    assert!(
        text.contains("release/next"),
        "missing release/next deployment target: {text}"
    );
    assert!(
        text.contains("2026-04-29 14:25"),
        "missing release/next deployment timestamp: {text}"
    );
    assert!(
        text.contains("+2"),
        "missing release/next pending distance: {text}"
    );
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

#[test]
fn flow_modal_renders_running_workflow_steps() {
    let mut state = AppState::new();
    let (_tx, rx) = std::sync::mpsc::channel();
    state.workflow_job = Some(WorkflowJob {
        rx,
        handle: None,
        spinner: 1,
        label: "Release current branch into release/next".into(),
        steps: vec![
            "push feature/demo".into(),
            "merge origin/feature/demo".into(),
            "push release/next".into(),
        ],
        current_step: Some(1),
    });

    let backend = TestBackend::new(90, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::flow::render(&state, frame.area(), frame);
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
        text.contains("[x] push feature/demo"),
        "missing completed step: {text}"
    );
    assert!(
        text.contains(">/< merge origin/feature/demo"),
        "missing active step marker: {text}"
    );
    assert!(
        text.contains("[ ] push release/next"),
        "missing pending step: {text}"
    );
}

#[test]
fn conflict_modal_asks_user_to_resolve_externally() {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.conflicts = vec!["src/conflict.rs".into()];
    state.conflict_log = "merge failed".into();

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::conflict::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("Git conflict detected"), "{text}");
    assert!(
        text.contains("Resolve the conflict outside lg"),
        "modal should ask for external resolution: {text}"
    );
    assert!(
        text.contains("validate resolved/staged/merged state"),
        "modal should expose validate action: {text}"
    );
    assert!(!text.contains("ours/theirs/both"), "{text}");
    assert!(!text.contains("LLM"), "{text}");
    assert!(!text.contains("stage + continue"), "{text}");
}

// ── Tree building ─────────────────────────────────────────────────────────────

#[test]
fn tree_flat_files_emit_all_changes_plus_each_file() {
    let files = vec![
        FileEntry {
            path: "a.rs".into(),
            x: ' ',
            y: 'M',
        },
        FileEntry {
            path: "b.rs".into(),
            x: 'A',
            y: ' ',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());
    assert_eq!(rows.len(), 3);
    assert!(matches!(rows[0].kind, TreeKind::AllChanges));
    assert!(matches!(rows[1].kind, TreeKind::File { entry_idx: 0 }));
    assert!(matches!(rows[2].kind, TreeKind::File { entry_idx: 1 }));
}

#[test]
fn tree_groups_nested_files_under_folders() {
    let files = vec![
        FileEntry {
            path: "src/lib.rs".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/util/mod.rs".into(),
            x: 'A',
            y: ' ',
        },
        FileEntry {
            path: "README.md".into(),
            x: ' ',
            y: 'M',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());
    // Interleaved alphabetical at each depth:
    //   root: README.md ('r') < src/ ('s')
    //   src/: lib.rs ('l') < util/ ('u')
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[1].path, "README.md");
    assert_eq!(rows[2].path, "src");
    match rows[2].kind {
        TreeKind::Folder {
            expanded,
            total,
            staged,
        } => {
            assert!(expanded);
            assert_eq!(total, 2);
            assert_eq!(staged, 2);
        }
        _ => panic!("expected folder row"),
    }
    assert_eq!(rows[3].path, "src/lib.rs");
    assert_eq!(rows[4].path, "src/util");
    assert_eq!(rows[5].path, "src/util/mod.rs");
}

#[test]
fn tree_compacts_single_subdir_chains() {
    let files = vec![
        FileEntry {
            path: "src/main/kotlin/org/example/inventory/App.kt".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/main/kotlin/org/example/inventory/Service.kt".into(),
            x: ' ',
            y: 'M',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());

    assert_eq!(rows[1].path, "src/main/kotlin/org/example/inventory");
    assert_eq!(rows[1].label, "src/main/kotlin/org/example/inventory");
    assert_eq!(rows[2].path, "src/main/kotlin/org/example/inventory/App.kt");
    assert_eq!(
        rows[3].path,
        "src/main/kotlin/org/example/inventory/Service.kt"
    );
}

#[test]
fn tree_renders_untracked_directory_with_name() {
    let files = vec![FileEntry {
        path: "src/main/kotlin/no/alv/exchange_rate/model/response/".into(),
        x: '?',
        y: '?',
    }];

    let rows = build_tree_rows(&files, &HashSet::new());

    assert!(
        rows.iter().any(|row| row.label == "response/"),
        "untracked directory row should keep its display name: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.label.is_empty()),
        "tree should not emit an empty file label: {rows:?}"
    );
}

#[test]
fn tree_collapsed_folder_hides_children() {
    let files = vec![
        FileEntry {
            path: "src/a.rs".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/b.rs".into(),
            x: 'A',
            y: ' ',
        },
    ];
    let mut collapsed = HashSet::new();
    collapsed.insert("src".to_string());
    let rows = build_tree_rows(&files, &collapsed);
    assert_eq!(rows.len(), 2); // AllChanges + collapsed src folder only
    match rows[1].kind {
        TreeKind::Folder {
            expanded,
            total,
            staged,
        } => {
            assert!(!expanded);
            assert_eq!(total, 2);
            assert_eq!(staged, 2);
        }
        _ => panic!("expected folder row"),
    }
}

#[test]
fn files_panel_enter_toggles_folder_collapse() {
    use ratatui::crossterm::event::KeyCode;
    let mut state = AppState::new();
    state.files = vec![FileEntry {
        path: "src/lib.rs".into(),
        x: 'M',
        y: ' ',
    }];
    // Initial rows: AllChanges, src/, src/lib.rs. Move cursor to folder row.
    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(
        state.collapsed_dirs.contains("src"),
        "Enter on expanded folder should collapse it"
    );
    // Now only AllChanges + collapsed src remain
    assert_eq!(state.tree_rows().len(), 2);

    // Enter again re-expands.
    panel::files::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(!state.collapsed_dirs.contains("src"));
    assert_eq!(state.tree_rows().len(), 3);
}

// ── Push animation ────────────────────────────────────────────────────────────

#[test]
fn push_modal_renders_spinner_when_job_running() {
    use lg::state::{PushJob, PushMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut state = AppState::new();
    state.modal = Modal::Push;
    state.branch = Some("feature/x".into());
    let (_tx, rx) = std::sync::mpsc::channel::<PushMsg>();
    state.push_job = Some(PushJob {
        rx,
        handle: None,
        spinner: 0,
        branch: "feature/x".into(),
        remote: "origin".into(),
    });

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::push::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }
    assert!(text.contains("Pushing"), "expected 'Pushing' label: {text}");
    assert!(
        text.contains("feature/x"),
        "expected branch name in modal: {text}"
    );
}

#[test]
fn push_modal_handle_key_is_noop_while_running() {
    use lg::state::{PushJob, PushMsg};
    use ratatui::crossterm::event::KeyCode;
    let mut state = AppState::new();
    state.modal = Modal::Push;
    let (_tx, rx) = std::sync::mpsc::channel::<PushMsg>();
    state.push_job = Some(PushJob {
        rx,
        handle: None,
        spinner: 0,
        branch: "main".into(),
        remote: "origin".into(),
    });

    panel::push::handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    // Esc must not close the modal while push is running.
    assert_eq!(state.modal, Modal::Push);
    panel::push::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(state.pending_action.is_none());
}

#[test]
fn push_modal_enter_merges_when_branch_diverged() {
    let mut state = AppState::new();
    state.modal = Modal::Push;
    state.branch = Some("feature/diverged".into());
    state.ahead_behind = Some((1, 6));

    panel::push::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(state.pending_action, Some(PendingAction::MergeUpstream));
}

#[test]
fn layout_renders_all_panel_borders() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state = make_state_with_files();
    app.state.repo_root = Some("/tmp/work/lg".into());
    app.state.branch = Some("main".into());
    add_flow_branches(&mut app.state);
    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(all_text.contains("lg"), "missing project header");
    assert!(all_text.contains("Status"), "missing Status panel title");
    assert!(
        all_text.contains("Deployment Status"),
        "missing Deployment Status panel title"
    );
    assert!(all_text.contains("Files"), "missing Files panel title");
    assert!(
        all_text.contains("Branches"),
        "missing Branches panel title"
    );
    assert!(all_text.contains("Commits"), "missing Commits panel title");
}

#[test]
fn layout_gives_files_panel_environment_space_when_flow_is_hidden() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let with_flow = lg::ui::split_layout_with_environments(area, true);
    let without_flow = lg::ui::split_layout_with_environments(area, false);

    assert_eq!(without_flow.environments.height, 0);
    assert_eq!(
        without_flow.files.y,
        without_flow
            .status
            .y
            .saturating_add(without_flow.status.height)
    );
    assert!(
        without_flow.files.height > with_flow.files.height,
        "hidden environment height should be given to files"
    );
    assert_eq!(
        without_flow.branches.y,
        without_flow
            .files
            .y
            .saturating_add(without_flow.files.height)
    );
    assert_eq!(
        without_flow.commits.y,
        without_flow
            .branches
            .y
            .saturating_add(without_flow.branches.height)
    );
}

#[test]
fn layout_accepts_resized_left_column_width() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 24,
    };
    let resized = lg::ui::split_layout_with_width(area, true, Some(40));

    assert_eq!(resized.status.width, 40);
    assert_eq!(resized.main.x, 40);
    assert_eq!(resized.main.width, 80);
}

#[test]
fn layout_accepts_resized_left_panel_heights() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 24,
    };
    let resized = lg::ui::split_layout_with_sizes(area, true, Some(40), Some([4, 6, 5, 4, 4]));

    assert_eq!(resized.status.height, 4);
    assert_eq!(resized.environments.height, 6);
    assert_eq!(resized.files.height, 5);
    assert_eq!(resized.branches.height, 4);
    assert_eq!(resized.commits.height, 3);
}

#[test]
fn hidden_environment_layout_merges_saved_environment_height_into_files() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 24,
    };
    let resized = lg::ui::split_layout_with_sizes(area, false, Some(40), Some([4, 6, 5, 4, 4]));

    assert_eq!(resized.environments.height, 0);
    assert_eq!(resized.files.height, 11);
}
