use super::common::*;

// ── Focus cycling ─────────────────────────────────────────────────────────────

#[test]
fn focus_cycles_through_all_panes_with_tab() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    assert_eq!(app.state.focus, Pane::Files);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Branches);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Commits);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Files);
}

#[test]
fn numeric_keys_set_focus() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.send_key(key(KeyCode::Char('1'))).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    app.send_key(key(KeyCode::Char('3'))).unwrap();
    assert_eq!(app.state.focus, Pane::Branches);
    app.send_key(key(KeyCode::Char('0'))).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
}

#[test]
fn branches_panel_shows_remote_and_missing_upstream_indicators() {
    let mut state = AppState::new();
    state.branches = vec![
        Branch {
            name: "feature/remote".into(),
            is_current: true,
            upstream: Some("origin/feature/remote".into()),
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "docs".into(),
            is_current: false,
            upstream: Some("origin/docs".into()),
            upstream_gone: true,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "feature/local".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
    ];

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        text.contains("\u{2713}"),
        "missing remote indicator: {text}"
    );
    assert!(
        text.contains("(upstream gone)"),
        "missing upstream gone indicator: {text}"
    );
    assert!(
        text.contains("no remote"),
        "missing local-only indicator: {text}"
    );
}

#[test]
fn branches_panel_shows_ahead_and_behind_counts() {
    let mut state = AppState::new();
    state.branches = vec![Branch {
        name: "feature/diverged".into(),
        is_current: true,
        upstream: Some("origin/feature/diverged".into()),
        upstream_gone: false,
        ahead: 1,
        behind: 6,
        behind_main: 0,
        last_commit_unix: None,
    }];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(text.contains("\u{2191}1"), "missing ahead count: {text}");
    assert!(text.contains("\u{2193}6"), "missing behind count: {text}");
}

#[test]
fn branches_panel_shows_behind_main_count() {
    let mut state = AppState::new();
    state.branches = vec![Branch {
        name: "feature/stale-main".into(),
        is_current: true,
        upstream: Some("origin/feature/stale-main".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 17,
        last_commit_unix: None,
    }];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, false);
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
        text.contains("main\u{2193}17"),
        "missing behind-main count: {text}"
    );
}

#[test]
fn branches_panel_keeps_context_below_selected_local_row_while_scrolling() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches_idx = 8;
    state.branches = (0..14)
        .map(|idx| Branch {
            name: format!("feature/{idx:02}"),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        })
        .collect();

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, true);
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
        .find(|row| row_text(*row).contains("feature/08"))
        .expect("selected branch should be visible");

    assert!(
        selected_row < buf.area.height - 2,
        "selected local branch should not stick to the bottom:\n{}",
        (0..buf.area.height)
            .map(row_text)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(row_text(selected_row + 1).contains("feature/09"));
    assert!(row_text(selected_row + 2).contains("feature/10"));
}

#[test]
fn branches_panel_keeps_context_below_selected_remote_row_while_scrolling() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branch_view = BranchView::Remote;
    state.remote_branches_idx = 8;
    state.remote_branches = (0..14)
        .map(|idx| RemoteBranch {
            name: format!("origin/feature/{idx:02}"),
            remote: "origin".into(),
            local_name: format!("feature/{idx:02}"),
            last_commit_unix: None,
        })
        .collect();

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, true);
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
        .find(|row| row_text(*row).contains("origin/feature/08"))
        .expect("selected remote branch should be visible");

    assert!(
        selected_row < buf.area.height - 2,
        "selected remote branch should not stick to the bottom:\n{}",
        (0..buf.area.height)
            .map(row_text)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(row_text(selected_row + 1).contains("origin/feature/09"));
    assert!(row_text(selected_row + 2).contains("origin/feature/10"));
}

#[test]
fn branches_panel_keeps_missing_upstream_visible_for_long_names() {
    let mut state = AppState::new();
    state.branches = vec![Branch {
        name: "feature/very-long-branch-name-that-would-otherwise-hide-status".into(),
        is_current: false,
        upstream: Some("origin/removed".into()),
        upstream_gone: true,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    let backend = TestBackend::new(42, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        text.contains("(upstream gone)"),
        "status should remain visible for long branch names: {text}"
    );
}

#[test]
fn branches_panel_shows_time_since_last_commit() {
    let mut state = AppState::new();
    state.branches = vec![Branch {
        name: "feature/recent".into(),
        is_current: false,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: Some(chrono::Utc::now().timestamp() - 2 * 60 * 60),
    }];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(text.contains("2h"), "missing branch age: {text}");
}

#[test]
fn branches_panel_toggles_remote_view() {
    let mut state = AppState::new();
    state.remote_branches = vec![RemoteBranch {
        name: "origin/feature/remote".into(),
        remote: "origin".into(),
        local_name: "feature/remote".into(),
        last_commit_unix: Some(chrono::Utc::now().timestamp() - 3 * 60),
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('r'))).unwrap();
    assert_eq!(state.branch_view, BranchView::Remote);

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        text.contains("Remote Branches"),
        "missing remote title: {text}"
    );
    assert!(
        text.contains("origin/feature/remote"),
        "missing remote branch: {text}"
    );
    assert!(text.contains("3m"), "missing remote branch age: {text}");
}

#[test]
fn remote_branch_view_hides_checked_out_branches() {
    let mut state = AppState::new();
    state.branch_view = BranchView::Remote;
    state.branches = vec![Branch {
        name: "feature/local".into(),
        is_current: false,
        upstream: Some("origin/feature/local".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];
    state.remote_branches = vec![
        RemoteBranch {
            name: "origin/feature/local".into(),
            remote: "origin".into(),
            local_name: "feature/local".into(),
            last_commit_unix: None,
        },
        RemoteBranch {
            name: "origin/feature/new".into(),
            remote: "origin".into(),
            local_name: "feature/new".into(),
            last_commit_unix: None,
        },
    ];

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::branches::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        !text.contains("origin/feature/local"),
        "checked-out remote should be hidden: {text}"
    );
    assert!(
        text.contains("origin/feature/new"),
        "unchecked-out remote should remain visible: {text}"
    );
    assert_eq!(state.branch_list_len(), 1);
    assert_eq!(state.selected_branch_ref(), Some("origin/feature/new"));
}

#[test]
fn branches_shortcuts_show_remote_toggle() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(160, 48)).unwrap();
    app.state.focus = Pane::Branches;
    app.render().unwrap();
    let footer = buffer_text(&app);
    assert!(
        footer.contains("r remotes"),
        "branches footer should show remote toggle: {footer}"
    );
    assert!(
        footer.contains("m pull/merge main"),
        "branches footer should show merge-main shortcut: {footer}"
    );
    assert!(
        footer.contains("M sync all"),
        "branches footer should show sync-all shortcut: {footer}"
    );
    assert!(
        footer.contains("d drop local"),
        "branches footer should show local-only delete shortcut: {footer}"
    );

    app.state.prev_focus = Pane::Branches;
    app.state.modal = Modal::Help;
    app.render().unwrap();
    let help = buffer_text(&app);
    assert!(
        help.contains("Toggle local and remote branch views"),
        "help should show remote toggle: {help}"
    );
    assert!(
        help.contains("Pull main, or merge origin/main"),
        "help should show merge-main shortcut: {help}"
    );
    assert!(
        help.contains("Merge main into all branches and push"),
        "help should show sync-all shortcut: {help}"
    );
    assert!(
        help.contains("Delete selected local branch with no remote"),
        "help should show local-only delete shortcut: {help}"
    );
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
    state.branch = Some("release/next".into());
    state.branches = vec![Branch {
        name: "release/next".into(),
        is_current: true,
        upstream: Some("origin/release/next".into()),
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

    assert_eq!(
        state.pending_action,
        Some(PendingAction::MergeMainAllBranches)
    );
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
    assert_eq!(state.delete_branch_target, "feature/local-only");
    assert!(state.delete_branch_local);
    assert!(!state.delete_branch_remote);
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
    assert_eq!(state.delete_branch_target, "feature/current");
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
        state.delete_branch_field,
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
    assert!(state.delete_branch_remote);
}

#[test]
fn pressing_f_opens_branch_actions_from_branches_pane() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    add_flow_branches(&mut app.state);
    app.state.focus = Pane::Branches;
    app.send_key(key(KeyCode::Char('F'))).unwrap();
    assert_eq!(app.state.modal, Modal::Flow);

    let text = buffer_text(&app);

    assert!(
        text.contains("Branch Actions"),
        "missing branch actions title: {text}"
    );
    assert!(
        text.contains("Release current branch into develop"),
        "missing develop release action: {text}"
    );
    assert!(
        text.contains("Start new feature from origin/main"),
        "missing new feature action: {text}"
    );
}

#[test]
fn pressing_f_does_not_open_branch_actions_outside_branches_pane() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.send_key(key(KeyCode::Char('F'))).unwrap();

    assert_eq!(app.state.modal, Modal::None);
}

fn render_flow_text(state: &AppState) -> String {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::flow::render(state, frame.area(), frame);
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

#[test]
fn flow_modal_hides_merge_main_on_main() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("main".into());
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        !text.contains("Merge origin/main into current branch"),
        "merge-main should be hidden on main: {text}"
    );
    assert!(
        text.contains("Start new feature from origin/main"),
        "other flow actions should remain visible: {text}"
    );
}

#[test]
fn flow_modal_shows_merge_main_on_develop_when_behind_main() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("develop".into());
    if let Some(branch) = state
        .branches
        .iter_mut()
        .find(|branch| branch.name == "develop")
    {
        branch.is_current = true;
        branch.behind_main = 2;
    }
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        text.contains("Merge origin/main into current branch"),
        "merge-main should be shown on stale develop: {text}"
    );
}

#[test]
fn flow_modal_hides_merge_main_on_release_next_when_not_behind_main() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("release/next".into());
    if let Some(branch) = state
        .branches
        .iter_mut()
        .find(|branch| branch.name == "release/next")
    {
        branch.is_current = true;
    }
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        !text.contains("Merge origin/main into current branch"),
        "merge-main should be hidden on up-to-date release/next: {text}"
    );
}

#[test]
fn branch_actions_show_transfer_diff_for_selected_feature_branch() {
    let mut state = AppState::new();
    state.branch = Some("feature/current".into());
    state.focus = Pane::Branches;
    state.branches = vec![
        Branch {
            name: "main".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "feature/current".into(),
            is_current: true,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
    ];
    state.branches_idx = 1;
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        text.contains("Transfer selected feature diff to new branch"),
        "missing transfer action: {text}"
    );
}
