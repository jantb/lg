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
            last_commit_unix: None,
        },
        Branch {
            name: "docs".into(),
            is_current: false,
            upstream: Some("origin/docs".into()),
            upstream_gone: true,
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
}

#[test]
fn branches_panel_keeps_missing_upstream_visible_for_long_names() {
    let mut state = AppState::new();
    state.branches = vec![Branch {
        name: "feature/very-long-branch-name-that-would-otherwise-hide-status".into(),
        is_current: false,
        upstream: Some("origin/removed".into()),
        upstream_gone: true,
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
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 48)).unwrap();
    app.state.focus = Pane::Branches;
    app.render().unwrap();
    let footer = buffer_text(&app);
    assert!(
        footer.contains("r remotes"),
        "branches footer should show remote toggle: {footer}"
    );

    app.state.prev_focus = Pane::Branches;
    app.state.modal = Modal::Help;
    app.render().unwrap();
    let help = buffer_text(&app);
    assert!(
        help.contains("Toggle local and remote branch views"),
        "help should show remote toggle: {help}"
    );
}

#[test]
fn pressing_f_opens_flow_modal_with_deploy_map() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    add_flow_branches(&mut app.state);
    app.send_key(key(KeyCode::Char('F'))).unwrap();
    assert_eq!(app.state.modal, Modal::Flow);

    let buf = app.terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(
        text.contains("production"),
        "missing production path: {text}"
    );
    assert!(text.contains("develop"), "missing develop branch: {text}");
    assert!(text.contains("dev"), "missing dev deployment: {text}");
    assert!(
        text.contains("release/next"),
        "missing release/next branch: {text}"
    );
    assert!(text.contains("test"), "missing test deployment: {text}");
}

#[test]
fn pressing_f_does_not_open_flow_without_release_branches() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.send_key(key(KeyCode::Char('F'))).unwrap();

    assert_eq!(app.state.modal, Modal::None);
}

#[test]
fn flow_modal_hides_merge_main_on_protected_branches() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("main".into());
    state.modal = Modal::Flow;

    let backend = TestBackend::new(100, 30);
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
        !text.contains("Merge origin/main into current branch"),
        "merge-main should be hidden on protected branches: {text}"
    );
    assert!(
        text.contains("Start new feature from origin/main"),
        "other flow actions should remain visible: {text}"
    );
}
