//! Moving focus between panes, and what the branches panel shows.

use super::common::*;

#[test]
fn focus_cycles_through_all_panes_with_tab() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Files);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Branches);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Commits);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
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
        text.contains("no upstream"),
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
fn branches_o_opens_project() {
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

    panel::branches::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(state.pending_action, Some(PendingAction::OpenProject));
}

#[test]
fn branches_u_sets_matching_remote_upstream() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches = vec![Branch {
        name: "solution".into(),
        is_current: true,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];
    state.remote_branches = vec![RemoteBranch {
        name: "origin/solution".into(),
        remote: "origin".into(),
        local_name: "solution".into(),
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('u'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::SetBranchUpstream {
            branch: "solution".into(),
            upstream: "origin/solution".into(),
        })
    );
}

#[test]
fn branches_u_reports_missing_matching_remote() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches = vec![Branch {
        name: "solution".into(),
        is_current: true,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];

    panel::branches::handle_key(&mut state, key(KeyCode::Char('u'))).unwrap();

    assert_eq!(state.pending_action, None);
    assert_eq!(
        state.status.as_ref().map(|status| status.text.as_str()),
        Some("no matching remote branch for solution")
    );
}

#[test]
fn branches_panel_keeps_context_below_selected_local_row_while_scrolling() {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches_list.idx = 8;
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
    state.remote_branches_list.idx = 8;
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
    let help = help_text(&mut app, Rect::new(0, 0, 160, 48));
    assert!(
        help.contains("Toggle local and remote branch views"),
        "help should show remote toggle: {help}"
    );
    assert!(
        help.contains("Pull main or merge origin/main"),
        "help should show merge-main shortcut: {help}"
    );
    assert!(
        help.contains("Merge main into all branches and push"),
        "help should show sync-all shortcut: {help}"
    );
    assert!(
        help.contains("Delete local branch with no upstream"),
        "help should show local-only delete shortcut: {help}"
    );
}
