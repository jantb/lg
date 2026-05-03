use lg::{
    git::{
        AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, ReleaseTargetStatus,
        RemoteBranch, ReviewNode,
    },
    panel,
    state::{
        AppState, AuthorField, BranchView, Modal, Pane, PendingAction, ReleaseStatusJob, TreeKind,
        WorkflowJob, build_tree_rows,
    },
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    style::{Color, Modifier},
};
use std::{collections::HashSet, sync::mpsc};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn make_state_with_files() -> AppState {
    let mut s = AppState::new();
    s.files = vec![
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
        FileEntry {
            path: "c.rs".into(),
            x: '?',
            y: '?',
        },
    ];
    s
}

fn add_flow_branches(state: &mut AppState) {
    state.branches = vec![
        Branch {
            name: "develop".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            last_commit_unix: None,
        },
        Branch {
            name: "release/next".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            last_commit_unix: None,
        },
    ];
}

fn buffer_text(app: &lg::app::HeadlessApp<TestBackend>) -> String {
    let buf = app.terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }
    text
}

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
            last_commit_unix: None,
        },
        Branch {
            name: "feature".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
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
            last_commit_unix: None,
        },
        Branch {
            name: "feature".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
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

// ── Panel transitions ─────────────────────────────────────────────────────────

#[test]
fn pressing_c_opens_commit_modal() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.send_key(key(KeyCode::Char('c'))).unwrap();
    assert_eq!(app.state.modal, Modal::Commit);
}

#[test]
fn pressing_c_with_only_unstaged_changes_prompts_to_stage_all() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state.files = vec![FileEntry {
        path: "a.rs".into(),
        x: ' ',
        y: 'M',
    }];

    app.send_key(key(KeyCode::Char('c'))).unwrap();

    assert_eq!(app.state.modal, Modal::StageAllBeforeCommit);
}

#[test]
fn stage_all_before_commit_prompt_accepts_or_cancels() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state.modal = Modal::StageAllBeforeCommit;

    app.send_key(key(KeyCode::Char('y'))).unwrap();

    assert_eq!(app.state.modal, Modal::None);
    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::StageAllAndCommit)
    );

    app.state.pending_action = None;
    app.state.modal = Modal::StageAllBeforeCommit;
    app.send_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.state.modal, Modal::None);
    assert_eq!(app.state.pending_action, None);
}

#[test]
fn pressing_c_with_staged_changes_opens_commit_modal() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state.files = vec![FileEntry {
        path: "b.rs".into(),
        x: 'A',
        y: ' ',
    }];

    app.send_key(key(KeyCode::Char('c'))).unwrap();

    assert_eq!(app.state.modal, Modal::Commit);
}

#[test]
fn pressing_esc_in_commit_closes_modal_and_keeps_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "work in progress".into();

    panel::commit::handle_key(&mut state, key(KeyCode::Esc)).unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.commit_message, "work in progress");
}

#[test]
fn help_overlay_closes_on_any_key() {
    let mut state = make_state_with_files();
    state.prev_focus = Pane::Files;
    state.modal = Modal::Help;

    panel::help::handle_key(&mut state, key(KeyCode::Char('?'))).unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.focus, Pane::Files);
}

#[test]
fn main_footer_and_help_call_out_review_mode() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 60)).unwrap();
    app.state.focus = Pane::Main;
    app.render().unwrap();

    let footer = buffer_text(&app);
    assert!(
        footer.contains("review mode"),
        "main footer should name review mode: {footer}"
    );

    app.state.modal = Modal::Help;
    app.state.prev_focus = Pane::Main;
    app.render().unwrap();

    let help = buffer_text(&app);
    assert!(
        help.contains("Review mode"),
        "help should include a Review mode section: {help}"
    );
    assert!(
        help.contains("Enter review mode against main"),
        "help should describe how to enter review mode: {help}"
    );
}

#[test]
fn author_modal_saves_subtree_rule_from_fields() {
    let mut state = AppState::new();
    state.modal = Modal::Author;
    state.author_path_input = "/tmp/example-work".into();
    state.author_name_input = "Example User".into();
    state.author_email_input = "example@example.com".into();
    state.author_field = AuthorField::Email;

    panel::author::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::SaveSubtreeAuthor {
            path: "/tmp/example-work".into(),
            name: "Example User".into(),
            email: "example@example.com".into(),
        })
    );
}

#[test]
fn author_modal_can_save_repo_local_author() {
    let mut state = AppState::new();
    state.modal = Modal::Author;
    state.author_name_input = "Example User".into();
    state.author_email_input = "example@example.com".into();

    panel::author::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::SaveAuthor {
            name: "Example User".into(),
            email: "example@example.com".into(),
        })
    );
}

#[test]
fn author_modal_uses_terminal_cursor_for_active_field() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.modal = Modal::Author;
    app.state.author_path_input = "/tmp/example".into();
    app.state.author_name_input = "Example User".into();
    app.state.author_email_input = "a@b".into();
    app.state.author_field = AuthorField::Email;

    app.render().unwrap();

    app.terminal.backend_mut().assert_cursor_position((24, 15));
}

#[test]
fn author_modal_shows_error_when_terminal_is_too_small() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(20, 6)).unwrap();
    app.state.modal = Modal::Author;
    app.state.author_field = AuthorField::Email;

    app.render().unwrap();

    let text = buffer_text(&app);
    assert!(
        text.contains("Terminal too small"),
        "missing small-terminal message: {text}"
    );
}

#[test]
fn review_panel_expands_hunks_and_source_context() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("App.kt");
    std::fs::write(
        &source_path,
        "class App {\n    fun greeting() = \"hello review\"\n    val untouched = 1\n}\n",
    )
    .unwrap();
    let source_path = source_path.display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![
            ReviewNode {
                id: "branch".into(),
                parent: None,
                depth: 0,
                title: "Branch diff".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:entry:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: format!("{source_path}:2 in fun greeting - updates greeting (+1 -1)"),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:hunk:0".into(),
                parent: Some("branch:entry:0".into()),
                depth: 2,
                title: format!("{source_path}:2 - updates greeting (+1 -1)"),
                body: vec![
                    "effect: updates greeting (+1 -1)".into(),
                    "@@ -1,3 +1,3 @@".into(),
                    " class App {".into(),
                    "-    fun greeting() = \"hello\"".into(),
                    "+    fun greeting() = \"hello review\"".into(),
                    "     val untouched = 1".into(),
                ],
                context: vec![
                    "    1 | class App {".into(),
                    "    2 |     fun greeting() = \"hello review\"".into(),
                    "    3 | }".into(),
                ],
            },
        ],
    });
    app.state.review_collapsed.insert("branch:entry:0".into());
    app.state.review_collapsed.insert("branch:hunk:0".into());

    app.render().unwrap();
    let collapsed = buffer_text(&app);
    assert!(collapsed.contains("fun greeting"), "{collapsed}");
    assert!(
        !collapsed.contains("hello review"),
        "collapsed hunk should hide patch body: {collapsed}"
    );

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('j'))).unwrap();
    panel::main::handle_key(&mut app.state, key(KeyCode::Enter)).unwrap();
    panel::main::handle_key(&mut app.state, key(KeyCode::Char('j'))).unwrap();
    panel::main::handle_key(&mut app.state, key(KeyCode::Enter)).unwrap();
    app.render().unwrap();
    let expanded = buffer_text(&app);
    assert!(
        expanded.contains("+    fun greeting() = \"hello review\""),
        "expanded hunk should show patch: {expanded}"
    );

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let context = buffer_text(&app);
    assert!(
        context.contains("source context")
            && context.contains("source")
            && !context.contains("│ diff")
            && context.contains("-     fun greeting() = \"hello\"")
            && context.contains("+    fun greeting() = \"hello review\"")
            && context.contains("3 |     val untouched = 1"),
        "source context should be visible: {context}"
    );
    let buf = app.terminal.backend().buffer();
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "c" && cell.fg == Color::Yellow),
        "source context should syntax-highlight Kotlin keywords"
    );
}

#[test]
fn review_panel_sources_entry_subtree_across_files_and_drills_to_child_file() {
    let dir = tempfile::tempdir().unwrap();
    let caller_path = dir.path().join("Caller.kt");
    let callee_path = dir.path().join("Callee.kt");
    std::fs::write(
        &caller_path,
        "class Caller {\n    fun nextStep() = maybeTransfer()\n}\n",
    )
    .unwrap();
    std::fs::write(
        &callee_path,
        "class Callee {\n    fun maybeTransfer() = \"transfer\"\n}\n",
    )
    .unwrap();
    let caller_path = caller_path.display().to_string();
    let callee_path = callee_path.display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(160, 40)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![
            ReviewNode {
                id: "branch".into(),
                parent: None,
                depth: 0,
                title: "Branch diff".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:file:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: format!("{caller_path} - 1 entry point (+1 -1)"),
                body: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " class Caller {".into(),
                    "-    fun nextStep() = \"done\"".into(),
                    "+    fun nextStep() = maybeTransfer()".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:entry:0".into(),
                parent: Some("branch:file:0".into()),
                depth: 2,
                title: format!("{caller_path} in fun nextStep - updates nextStep (+1 -1)"),
                body: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " class Caller {".into(),
                    "-    fun nextStep() = \"done\"".into(),
                    "+    fun nextStep() = maybeTransfer()".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:hunk:0".into(),
                parent: Some("branch:entry:0".into()),
                depth: 3,
                title: format!("{caller_path}:2 - updates nextStep (+1 -1)"),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:file:1".into(),
                parent: Some("branch:entry:0".into()),
                depth: 3,
                title: format!("{callee_path} - 1 entry point (+1 -1)"),
                body: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " class Callee {".into(),
                    "-    fun maybeTransfer() = \"skip\"".into(),
                    "+    fun maybeTransfer() = \"transfer\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 2;
    app.state.review_collapsed.insert("branch:entry:0".into());

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let rendered = buffer_text(&app);
    assert!(
        rendered.contains(&format!("source {caller_path}")),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!("source {callee_path}")),
        "{rendered}"
    );
    assert!(
        rendered.contains("+     fun nextStep() = maybeTransfer()"),
        "{rendered}"
    );
    assert!(
        rendered.contains("+     fun maybeTransfer() = \"transfer\""),
        "{rendered}"
    );
    assert!(
        rendered.contains("↳"),
        "missing drill indicator: {rendered}"
    );
    let buf = app.terminal.backend().buffer();
    assert!(
        buf.content().iter().any(|cell| {
            cell.symbol() == "d"
                && cell.fg == Color::LightGreen
                && cell.modifier.contains(Modifier::BOLD)
        }),
        "drill shortcut should be highlighted when selected item can drill"
    );

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('d'))).unwrap();
    assert_eq!(
        app.state.review_idx, 4,
        "drill should prefer the nested file over the hunk"
    );
}

#[test]
fn review_navigation_keeps_selection_visible_without_early_scroll() {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = lg::state::DiffSource::Review;
    state.diff_viewport_height = 5;
    state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: (0..12)
            .map(|idx| ReviewNode {
                id: format!("node:{idx}"),
                parent: None,
                depth: 0,
                title: format!("src/module/File{idx}.rs - updates item"),
                body: Vec::new(),
                context: Vec::new(),
            })
            .collect(),
    });

    panel::main::handle_key(&mut state, key(KeyCode::Down)).unwrap();
    assert_eq!(state.review_idx, 1);
    assert_eq!(state.diff_offset, 0, "first down should not scroll");

    for _ in 0..4 {
        panel::main::handle_key(&mut state, key(KeyCode::Down)).unwrap();
    }
    assert_eq!(state.review_idx, 5);
    assert_eq!(
        state.diff_offset, 1,
        "offset should advance only when selection reaches the bottom"
    );
}

#[test]
fn review_panel_styles_tree_titles_and_change_counts() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 12)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![
            ReviewNode {
                id: "branch".into(),
                parent: None,
                depth: 0,
                title: "Full diff against main".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:entry:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: "src/main/kotlin/App.kt in fun greeting - updates greeting (+5 -3)".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });

    app.render().unwrap();
    let buf = app.terminal.backend().buffer().clone();

    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "s" && cell.fg == Color::LightCyan),
        "file path should be highlighted"
    );
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "+" && cell.fg == Color::LightGreen),
        "addition count should be green"
    );
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "-" && cell.fg == Color::LightRed),
        "removal count should be red"
    );
}

#[test]
fn diff_highlighting_colors_markers_and_changed_text_separately() {
    let added = lg::ui::highlight_diff_line("+added");
    let removed = lg::ui::highlight_diff_line("-removed");

    assert_eq!(added.spans[0].content.as_ref(), "+");
    assert_eq!(added.spans[0].style.fg, Some(Color::Green));
    assert!(added.spans[0].style.bg.is_some());
    assert_eq!(added.spans[1].content.as_ref(), "added");
    assert_eq!(added.spans[1].style.fg, Some(Color::Gray));
    assert_eq!(added.spans[1].style.bg, added.spans[0].style.bg);

    assert_eq!(removed.spans[0].content.as_ref(), "-");
    assert_eq!(removed.spans[0].style.fg, Some(Color::Red));
    assert!(removed.spans[0].style.bg.is_some());
    assert_eq!(removed.spans[1].content.as_ref(), "removed");
    assert_eq!(removed.spans[1].style.fg, Some(Color::Gray));
    assert_eq!(removed.spans[1].style.bg, removed.spans[0].style.bg);
}

#[test]
fn diff_highlighting_tracks_kotlin_syntax_by_file() {
    let lines = lg::ui::highlight_diff_text(
        "diff --git a/Foo.kt b/Foo.kt\n+++ b/Foo.kt\n+suspend fun getBalance() = \"ok\"",
    );
    let added = lines.last().expect("added kotlin line");

    assert_eq!(added.spans[0].content.as_ref(), "+");
    assert_eq!(added.spans[0].style.fg, Some(Color::Green));
    assert!(added.spans.iter().any(|span| {
        span.content.as_ref() == "suspend"
            && span.style.fg == Some(Color::Yellow)
            && span.style.bg == added.spans[0].style.bg
            && span.style.add_modifier.contains(Modifier::BOLD)
    }));
    assert!(added.spans.iter().any(|span| {
        span.content.as_ref() == "\"ok\"" && span.style.fg == Some(Color::LightYellow)
    }));
}

#[test]
fn review_diff_line_highlighting_uses_node_file_syntax() {
    let line = lg::ui::highlight_diff_line_for_path(
        "+    private val log by Logger()",
        "src/main/kotlin/org/example/service/ExampleService.kt",
    );

    assert!(line.spans.iter().any(|span| {
        span.content.as_ref() == "private"
            && span.style.fg == Some(Color::Yellow)
            && span.style.bg == line.spans[0].style.bg
            && span.style.add_modifier.contains(Modifier::BOLD)
    }));
    assert!(line.spans.iter().any(|span| {
        span.content.as_ref() == "val"
            && span.style.fg == Some(Color::Yellow)
            && span.style.bg == line.spans[0].style.bg
            && span.style.add_modifier.contains(Modifier::BOLD)
    }));
    assert!(line.spans.iter().any(|span| {
        span.content.as_ref() == "Logger"
            && span.style.fg == Some(Color::LightCyan)
            && span.style.bg == line.spans[0].style.bg
    }));
}

#[test]
fn diff_highlighting_marks_function_calls_without_addition_foreground() {
    let line = lg::ui::highlight_diff_line_for_path(
        "+    return mergeAccounts(hubFlow)",
        "src/main/kotlin/org/example/service/ExampleService.kt",
    );

    assert!(line.spans.iter().any(|span| {
        span.content.as_ref() == "mergeAccounts"
            && span.style.fg == Some(Color::LightMagenta)
            && span.style.bg == line.spans[0].style.bg
    }));
}

#[test]
fn diff_highlighting_tracks_rust_syntax_by_file() {
    let lines = lg::ui::highlight_diff_text(
        "diff --git a/src/main.rs b/src/main.rs\n+++ b/src/main.rs\n+pub fn main() { let value = \"ok\"; }",
    );
    let added = lines.last().expect("added rust line");

    assert!(added.spans.iter().any(|span| {
        span.content.as_ref() == "pub"
            && span.style.fg == Some(Color::Yellow)
            && span.style.add_modifier.contains(Modifier::BOLD)
    }));
    assert!(added.spans.iter().any(|span| {
        span.content.as_ref() == "fn"
            && span.style.fg == Some(Color::Yellow)
            && span.style.add_modifier.contains(Modifier::BOLD)
    }));
}

#[test]
fn diff_highlighting_adds_old_and_new_line_numbers_inside_hunks() {
    let lines = lg::ui::highlight_diff_text(
        "diff --git a/src/main.rs b/src/main.rs\n@@ -10,2 +20,2 @@\n context\n-old\n+new",
    );
    let context = &lines[2];
    let removed = &lines[3];
    let added = &lines[4];

    assert_eq!(context.spans[0].content.as_ref(), "  10");
    assert_eq!(context.spans[2].content.as_ref(), "  20");
    assert_eq!(removed.spans[0].content.as_ref(), "  11");
    assert_eq!(removed.spans[2].content.as_ref(), "    ");
    assert_eq!(added.spans[0].content.as_ref(), "    ");
    assert_eq!(added.spans[2].content.as_ref(), "  21");
    assert_eq!(added.spans[4].content.as_ref(), "+");
}

#[test]
fn log_highlighting_colors_graph_and_decorations() {
    let line = lg::ui::highlight_log_line("* commit abc123 (main)");

    assert_eq!(line.spans[0].content.as_ref(), "*");
    assert_eq!(line.spans[0].style.fg, Some(Color::Yellow));
    assert!(
        line.spans
            .iter()
            .any(|span| span.content.as_ref() == "abc123" && span.style.fg == Some(Color::Yellow))
    );
    assert!(
        line.spans
            .iter()
            .any(|span| span.content.as_ref() == "(main)"
                && span.style.fg == Some(Color::LightGreen))
    );
}

#[test]
fn branch_source_renders_log_view() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "* commit abc123 (main)\n| Author: Test User <test@example.com>".into();
    state.diff_line_count = 2;

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::main::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(text.contains("Log"), "missing log title: {text}");
    assert!(text.contains("abc123"), "missing commit id: {text}");
}

#[test]
fn diff_pane_o_opens_file_source() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::File("src/main.rs".into());

    panel::main::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenFile("src/main.rs".into()))
    );
}

#[test]
fn diff_pane_o_uses_diff_file_at_scroll_offset() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::All;
    state.diff_offset = 4;
    state.diff_text = [
        "diff --git a/src/first.kt b/src/first.kt",
        "--- a/src/first.kt",
        "+++ b/src/first.kt",
        "@@ -1 +1 @@",
        "diff --git a/src/second.rs b/src/second.rs",
        "--- a/src/second.rs",
        "+++ b/src/second.rs",
    ]
    .join("\n");

    panel::main::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenFile("src/second.rs".into()))
    );
}

#[test]
fn review_pane_o_opens_selected_source_file() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Review;
    state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![ReviewNode {
            id: "branch:file:0".into(),
            parent: None,
            depth: 0,
            title: "src/main/kotlin/App.kt in fun main - updates greeting".into(),
            body: Vec::new(),
            context: Vec::new(),
        }],
    });

    panel::main::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenFile("src/main/kotlin/App.kt".into()))
    );
}

#[test]
fn review_panel_explains_selected_subtree_with_ollama() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![
            ReviewNode {
                id: "branch".into(),
                parent: None,
                depth: 0,
                title: "Full diff against main".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:entry:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: "src/lib.rs:2 in fn greet - updates greet (+1 -1)".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 1;
    app.state.review_collapsed.insert("branch:entry:0".into());

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('l'))).unwrap();

    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::ReviewAssist("branch:entry:0".into()))
    );
    assert!(
        !app.state.review_collapsed.contains("branch:entry:0"),
        "selected subtree should open before explaining"
    );

    app.state.review_assists.insert(
        "branch:entry:0".into(),
        "Explains the greeting change.".into(),
    );
    app.render().unwrap();
    let rendered = buffer_text(&app);
    assert!(rendered.contains("ollama"), "{rendered}");
    assert!(
        rendered.contains("Explains the greeting change."),
        "{rendered}"
    );
}

#[test]
fn review_panel_renders_ollama_markdown() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(140, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![ReviewNode {
            id: "branch".into(),
            parent: None,
            depth: 0,
            title: "Full diff against main".into(),
            body: Vec::new(),
            context: Vec::new(),
        }],
    });
    app.state.review_assists.insert(
        "branch".into(),
        "# Summary\n- **BoldThing** calls `InlineCode`\n```kotlin\nfun runThing(value: String) = value\n```".into(),
    );

    app.render().unwrap();

    let rendered = buffer_text(&app);
    assert!(rendered.contains("Summary"), "{rendered}");
    assert!(
        rendered.contains("• BoldThing calls InlineCode"),
        "{rendered}"
    );
    assert!(rendered.contains("┌─ code kotlin"), "{rendered}");
    assert!(rendered.contains("fun runThing"), "{rendered}");
    assert!(!rendered.contains("**"), "{rendered}");
    assert!(!rendered.contains("```"), "{rendered}");

    let buf = app.terminal.backend().buffer();
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "B" && cell.modifier.contains(Modifier::BOLD)),
        "bold markdown should be rendered with bold style"
    );
    assert!(
        buf.content().iter().any(|cell| {
            cell.symbol() == "f"
                && cell.fg == Color::Yellow
                && cell.modifier.contains(Modifier::BOLD)
                && cell.bg != Color::Reset
        }),
        "kotlin code block should render highlighted code on a code background"
    );
}

#[test]
fn review_panel_starts_fully_collapsed_at_entry_roots() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![
            ReviewNode {
                id: "branch".into(),
                parent: None,
                depth: 0,
                title: "Full diff against main".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:entry:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: "src/lib.rs:2 in fn greet - updates greet (+1 -1)".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 0;
    app.state.review_collapsed.insert("branch".into());

    app.render().unwrap();
    let collapsed = buffer_text(&app);
    assert!(collapsed.contains("Full diff against main"), "{collapsed}");
    assert!(
        !collapsed.contains("fn greet"),
        "collapsed root should hide recursive children: {collapsed}"
    );
}

// ── Commit input ──────────────────────────────────────────────────────────────

#[test]
fn backspace_removes_last_char_of_commit_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "hello".into();
    state.commit_cursor = state.commit_message.chars().count();

    panel::commit::handle_key(&mut state, key(KeyCode::Backspace)).unwrap();

    assert_eq!(state.commit_message, "hell");
    assert_eq!(state.commit_cursor, 4);
}

#[test]
fn commit_input_edits_at_cursor_with_arrow_keys() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "helo".into();
    state.commit_cursor = state.commit_message.chars().count();

    panel::commit::handle_key(&mut state, key(KeyCode::Left)).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Char('l'))).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Right)).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Char('!'))).unwrap();

    assert_eq!(state.commit_message, "hello!");
    assert_eq!(state.commit_cursor, 6);
}

#[test]
fn commit_input_moves_cursor_vertically_and_with_control_shortcuts() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "one\ntwo\nthree".into();
    state.commit_cursor = 5;

    panel::commit::handle_key(&mut state, key(KeyCode::Up)).unwrap();
    assert_eq!(state.commit_cursor, 1);

    panel::commit::handle_key(&mut state, key(KeyCode::Down)).unwrap();
    assert_eq!(state.commit_cursor, 5);

    panel::commit::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(state.commit_cursor, 0);

    panel::commit::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(state.commit_cursor, state.commit_message.chars().count());
}

#[test]
fn commit_input_mouse_click_places_cursor() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "one\ntwo".into();

    let area = ratatui::layout::Rect::new(0, 0, 100, 30);
    assert!(panel::commit::place_cursor_at(&mut state, area, 12, 6));

    assert_eq!(state.commit_cursor, 5);
}

#[test]
fn commit_input_accepts_long_multiline_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "x".repeat(2_048);
    state.commit_cursor = state.commit_message.chars().count();

    panel::commit::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();

    assert_eq!(state.commit_message, format!("{}\ny", "x".repeat(2_048)));
}

#[test]
fn commit_modal_uses_terminal_cursor_for_multiline_message() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state = make_state_with_files();
    app.state.modal = Modal::Commit;
    app.state.commit_message = "one\ntwo".into();
    app.state.commit_cursor = app.state.commit_message.chars().count();

    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(all_text.contains("one"), "missing first message line");
    assert!(all_text.contains("two"), "missing second message line");
    assert!(
        !all_text.contains('\u{2588}'),
        "commit input should not render a block character as the cursor"
    );
    app.terminal.backend_mut().assert_cursor_position((14, 6));
}

#[test]
fn commit_modal_scrolls_to_cursor_for_long_multiline_message() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state = make_state_with_files();
    app.state.modal = Modal::Commit;
    app.state.commit_message = (0..40)
        .map(|i| format!("line-{i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.state.commit_cursor = app.state.commit_message.chars().count();

    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(all_text.contains("line-39"), "missing final message line");
    assert!(
        !all_text.contains("line-00"),
        "oldest message line should scroll out of the editor viewport"
    );
    app.terminal.backend_mut().assert_cursor_position((18, 23));
}

// ── Diff scroll clamping ──────────────────────────────────────────────────────

#[test]
fn diff_scroll_g_clamps_to_max_offset() {
    let mut state = AppState::new();
    // Seed 500 lines of content and set viewport metrics manually.
    state.diff_text = "line\n".repeat(500);
    state.diff_line_count = 500;
    state.diff_viewport_height = 10;

    // G should jump to max_offset = 500 - 10 = 490.
    panel::main::handle_key(&mut state, key(KeyCode::Char('G'))).unwrap();
    assert_eq!(state.diff_offset, 490, "G should set offset to max_offset");

    // j past the end must stay at 490.
    panel::main::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(
        state.diff_offset, 490,
        "j past end should stay at max_offset"
    );
    panel::main::handle_key(&mut state, key(KeyCode::Down)).unwrap();
    assert_eq!(
        state.diff_offset, 490,
        "Down past end should stay at max_offset"
    );
}

#[test]
fn diff_scroll_g_renders_non_blank_bottom_of_content() {
    // End-to-end: press `0`, seed a long diff, press `G`, render, then scan
    // the main-pane rect in the terminal buffer. Before the fix, `G` set the
    // offset to u16::MAX/2 and the pane rendered empty. This test would have
    // caught that.
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();

    // Focus the diff pane and render once so diff_viewport_height is set.
    app.send_key(key(KeyCode::Char('0'))).unwrap();
    app.render().unwrap();

    // Seed 200 numbered lines as the diff content.
    let body: String = (0..200).map(|i| format!("line {i:03}\n")).collect();
    app.state.diff_text = body;
    app.state.diff_line_count = 200;
    // viewport_height is already set from the previous render.

    // Press G — should jump to the end without blanking.
    app.send_key(key(KeyCode::Char('G'))).unwrap();

    let vh = app.state.diff_viewport_height;
    assert!(vh > 0, "viewport height should be set by render");
    assert_eq!(
        app.state.diff_offset,
        200 - vh,
        "G should land at line_count - viewport_height"
    );

    // Scan the main-pane rect in the buffer.
    let buf = app.terminal.backend().buffer().clone();
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: buf.area.width,
        height: buf.area.height,
    };
    let main = lg::ui::split_layout(area).main;

    // Collect the text inside the pane's inner area (skip the 1-cell border).
    let mut pane_text = String::new();
    for row in (main.y + 1)..(main.y + main.height - 1) {
        for col in (main.x + 1)..(main.x + main.width - 1) {
            pane_text.push_str(buf[(col, row)].symbol());
        }
        pane_text.push('\n');
    }

    // Sanity: the pane is not entirely whitespace.
    let non_space_cells = pane_text.chars().filter(|c| !c.is_whitespace()).count();
    assert!(
        non_space_cells > 0,
        "diff pane should not be blank after pressing G; got:\n{pane_text}"
    );

    // The last visible line in the buffer should be "line 199" (the final line
    // of content). If the offset were past the end, we'd see blanks instead.
    assert!(
        pane_text.contains("line 199"),
        "expected final line 'line 199' to be visible at bottom; got:\n{pane_text}"
    );

    // And pressing `j` further must not blank the pane.
    app.send_key(key(KeyCode::Char('j'))).unwrap();
    assert_eq!(
        app.state.diff_offset,
        200 - vh,
        "j past end should stay clamped"
    );

    let buf2 = app.terminal.backend().buffer().clone();
    let mut still_has_content = false;
    for row in (main.y + 1)..(main.y + main.height - 1) {
        for col in (main.x + 1)..(main.x + main.width - 1) {
            if !buf2[(col, row)].symbol().chars().all(char::is_whitespace) {
                still_has_content = true;
                break;
            }
        }
    }
    assert!(still_has_content, "pane blanked after j past end");
}

#[test]
fn branch_log_render_clamps_offset_to_rendered_lines() {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    state.diff_line_count = 500;
    state.diff_viewport_height = 4;
    state.diff_offset = 500;

    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::main::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains("message"),
        "missing log bottom: {rendered}"
    );
}

#[test]
fn branch_log_scroll_up_clamps_stale_offset_to_rendered_lines() {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    state.diff_line_count = 500;
    state.diff_viewport_height = 2;
    state.diff_offset = 500;

    panel::main::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();

    assert_eq!(state.diff_offset, 0);
}

#[test]
fn branch_log_max_scroll_offset_ignores_trailing_newline() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    state.diff_viewport_height = 2;

    assert_eq!(panel::main::max_scroll_offset(&state), 1);
}

#[test]
fn branch_log_scroll_bound_counts_wrapped_visual_rows() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "0123456789abcdefghij\nshort\n".into();
    state.diff_viewport_height = 1;
    state.diff_viewport_width = 10;

    assert_eq!(panel::main::max_scroll_offset(&state), 2);
    assert_eq!(panel::main::rendered_line_count(&state), 3);
}

#[test]
fn branch_log_fast_mouse_scroll_bursts_stay_in_bounds() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = (0..50)
        .map(|i| format!("* commit {i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    state.diff_text.push('\n');
    state.diff_viewport_height = 5;
    let max_offset = panel::main::max_scroll_offset(&state);

    for _ in 0..100 {
        panel::main::scroll(&mut state, true, 3);
        assert!(state.diff_offset <= max_offset);
    }
    assert_eq!(state.diff_offset, max_offset);

    for _ in 0..100 {
        panel::main::scroll(&mut state, false, 3);
        assert!(state.diff_offset <= max_offset);
    }
    assert_eq!(state.diff_offset, 0);
}

#[test]
fn render_clamps_stale_branch_log_offset_before_next_mouse_burst() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 24)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Branch("main".into());
    app.state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    app.state.diff_offset = 500;

    app.render().unwrap();

    assert_eq!(
        app.state.diff_offset,
        panel::main::max_scroll_offset(&app.state)
    );
    for _ in 0..100 {
        panel::main::scroll(&mut app.state, true, 3);
        assert!(app.state.diff_offset <= panel::main::max_scroll_offset(&app.state));
    }
}

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

    // Find the '?' cell (untracked) — should be cyan.
    let found_cyan_q = (0..buf.area.height).any(|row| {
        (0..buf.area.width).any(|col| {
            let cell = &buf[(col, row)];
            cell.symbol() == "?" && cell.style().fg == Some(Color::Cyan)
        })
    });
    assert!(found_cyan_q, "expected a cyan '?' cell for untracked file");
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
            panel::environments::render(&state, frame.area(), frame);
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
    assert!(text.contains("dev"), "missing dev badge: {text}");
    assert!(text.contains("test"), "missing test badge: {text}");
    assert!(
        text.contains("2026-04-29 14:20"),
        "missing release timestamp: {text}"
    );
    assert!(text.contains("+2 pending"), "missing pending count: {text}");
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
            panel::environments::render(&state, frame.area(), frame);
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
            panel::environments::render(&state, frame.area(), frame);
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

    assert!(text.contains("Merge conflict detected"), "{text}");
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
    assert_eq!(without_flow.files.y, with_flow.environments.y);
    assert_eq!(
        without_flow.files.height,
        with_flow.environments.height + with_flow.files.height
    );
    assert_eq!(without_flow.branches, with_flow.branches);
    assert_eq!(without_flow.commits, with_flow.commits);
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

#[test]
fn commits_panel_shows_author_names_with_distinct_colors() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["def5678".into()],
            is_first_parent: true,
            subject: "add feature".into(),
        },
        Commit {
            sha: "def5678".into(),
            author: "Bob Example".into(),
            author_short: "BE".into(),
            parents: vec!["root".into()],
            is_first_parent: true,
            subject: "fix bug".into(),
        },
    ];

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("AE"), "missing first author: {text}");
    assert!(text.contains("BE"), "missing second author: {text}");
    assert_ne!(buf[(11, 1)].fg, Color::Reset);
    assert_ne!(buf[(11, 2)].fg, Color::Reset);
    assert_ne!(buf[(11, 1)].fg, buf[(11, 2)].fg);
}

#[test]
fn commits_panel_render_clamps_stale_selection() {
    let mut state = AppState::new();
    state.focus = Pane::Commits;
    state.commits_idx = usize::MAX;
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["parent".into()],
        is_first_parent: true,
        subject: "initial".into(),
    }];

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains("abc1234"),
        "missing commit row: {rendered}"
    );
}

#[test]
fn commits_panel_colors_merged_authors_by_author_and_separates_hash() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE001".into(),
            author: "Top Person".into(),
            author_short: "TP".into(),
            parents: vec!["MAIN".into(), "12345678".into()],
            is_first_parent: true,
            subject: "merge side".into(),
        },
        Commit {
            sha: "12345678".into(),
            author: "Carol Example".into(),
            author_short: "CE".into(),
            parents: vec!["abcdef12".into()],
            is_first_parent: false,
            subject: "side one".into(),
        },
        Commit {
            sha: "abcdef12".into(),
            author: "Dave Example".into(),
            author_short: "DE".into(),
            parents: vec!["MAIN".into()],
            is_first_parent: false,
            subject: "side two".into(),
        },
    ];

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut second_row = String::new();
    for col in 0..buf.area.width {
        second_row.push_str(buf[(col, 2)].symbol());
    }

    assert!(
        second_row.contains("12345678 CE"),
        "hash and author should be separated by a space: {second_row}"
    );
    assert_ne!(buf[(10, 2)].fg, Color::DarkGray);
    assert_ne!(buf[(10, 3)].fg, Color::DarkGray);
    assert_ne!(buf[(10, 2)].fg, buf[(10, 3)].fg);
}

#[test]
fn commits_panel_marks_merge_commits() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["P".into(), "S".into()],
        is_first_parent: true,
        subject: "merge branch".into(),
    }];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    // ⏣─╮ at the merge row (lazygit-style).
    assert!(
        text.contains("\u{23e3}\u{2500}\u{256e}"),
        "missing merge connector: {text}"
    );
}

#[test]
fn commits_panel_draws_merge_connector_two_parent() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["P".into(), "S".into()],
        is_first_parent: true,
        subject: "merge branch".into(),
    }];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let marker_col = (0..buf.area.width)
        .find(|col| buf[(*col, 1)].symbol() == "\u{23e3}")
        .expect("merge marker should be rendered");
    let expected = [
        ("\u{23e3}", 0u16),
        ("\u{2500}", 1),
        ("\u{256e}", 2),
        (" ", 3),
        ("m", 4),
    ];
    for (symbol, offset) in expected {
        assert_eq!(buf[(marker_col + offset, 1)].symbol(), symbol);
    }
}

#[test]
fn commits_panel_merge_then_side_renders_lane_through() {
    // Merge with parents [P, S]. S is the next commit. The merge fork's right side
    // should render as ╮ on the merge row, then S's row should show │ ◯.
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["PARENT".into(), "SIDE".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "SIDE".into(),
            author: "Side".into(),
            author_short: "SD".into(),
            parents: vec!["PARENT".into()],
            is_first_parent: false,
            subject: "side commit".into(),
        },
    ];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
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
    assert!(
        row_text(1).contains("\u{23e3}\u{2500}\u{256e}"),
        "{}",
        row_text(1)
    );
    // SIDE row: │ ◯ (lane 0 still active, side commit on lane 1).
    assert!(row_text(2).contains("\u{2502} \u{25ef}"), "{}", row_text(2));
}

#[test]
fn commits_panel_uses_compact_graph_prefix_like_git_log() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into(), "def5678".into()],
            is_first_parent: true,
            subject: "merge branch".into(),
        },
        Commit {
            sha: "def5678".into(),
            author: "Bob Example".into(),
            author_short: "BE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let row_text = |row_idx| {
        let mut row = String::new();
        for col in 0..buf.area.width {
            row.push_str(buf[(col, row_idx)].symbol());
        }
        row
    };
    let cell_col = |row_idx, needle: &str| {
        let needle_chars: Vec<char> = needle.chars().collect();
        (0..buf.area.width).find(|start| {
            needle_chars.iter().enumerate().all(|(offset, ch)| {
                let col = start.saturating_add(offset as u16);
                col < buf.area.width && buf[(col, row_idx)].symbol() == ch.to_string()
            })
        })
    };
    let merge_row = row_text(1);
    let side_row = row_text(2);
    let merge_subject = cell_col(1, "merge branch").expect("merge subject");
    let side_marker = cell_col(2, "\u{25ef}").expect("side marker");
    let side_subject = cell_col(2, "side branch").expect("side subject");

    assert!(
        side_subject > side_marker,
        "side subject should follow its graph prefix:\n{side_row}"
    );
    let _ = merge_row;
    let _ = merge_subject;
}

#[test]
fn commits_panel_keeps_selected_hash_visible_and_graph_columns_stable() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "916a75688".into(),
            author: "Jan Example".into(),
            author_short: "JT".into(),
            parents: vec!["00e47360d".into()],
            is_first_parent: true,
            subject: "direct branch commit".into(),
        },
        Commit {
            sha: "00e47360d".into(),
            author: "Jan Example".into(),
            author_short: "JT".into(),
            parents: vec!["PARENT".into(), "a0f3424b0".into()],
            is_first_parent: true,
            subject: "merge side branch".into(),
        },
        Commit {
            sha: "a0f3424b0".into(),
            author: "Side Person".into(),
            author_short: "Sp".into(),
            parents: vec!["b3545f4c8".into()],
            is_first_parent: false,
            subject: "side branch commit".into(),
        },
        Commit {
            sha: "b3545f4c8".into(),
            author: "Renovate Bot".into(),
            author_short: "re".into(),
            parents: vec!["PARENT".into()],
            is_first_parent: false,
            subject: "second side commit".into(),
        },
    ];
    state.focus = Pane::Commits;
    state.commits_idx = 2;

    let backend = TestBackend::new(100, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let row_text = |row_idx| {
        let mut row = String::new();
        for col in 0..buf.area.width {
            row.push_str(buf[(col, row_idx)].symbol());
        }
        row
    };
    let cell_col = |row_idx, needle: &str| {
        let needle_chars: Vec<char> = needle.chars().collect();
        (0..buf.area.width).find(|start| {
            needle_chars.iter().enumerate().all(|(offset, ch)| {
                let col = start.saturating_add(offset as u16);
                col < buf.area.width && buf[(col, row_idx)].symbol() == ch.to_string()
            })
        })
    };

    let selected_row = row_text(3);
    assert!(
        selected_row.contains("a0f3424b0 Sp"),
        "selected hash and author should remain visible: {selected_row}"
    );
    assert_ne!(
        buf[(1, 3)].fg,
        buf[(1, 3)].bg,
        "selected hash foreground must contrast with selection background"
    );

    let merge_origin = cell_col(2, "\u{23e3}\u{2500}\u{256e}")
        .expect("merge row should show a visible branch origin");
    let selected_marker =
        cell_col(3, "\u{25ef}").expect("selected side commit should show a marker");
    assert!(
        selected_marker > merge_origin,
        "side commit marker should sit to the right of the merge origin:\n{}\n{}",
        row_text(2),
        selected_row
    );
    assert!(cell_col(3, "side branch commit").is_some());
    assert!(cell_col(4, "second side commit").is_some());
}

#[test]
fn commits_panel_highlights_selected_merge_connector() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        parents: vec!["P".into(), "S".into()],
        is_first_parent: true,
        subject: "merge branch".into(),
    }];
    state.focus = Pane::Commits;
    state.commits_idx = 0;

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let connector_start = (0..buf.area.width)
        .find(|col| buf[(*col, 1)].symbol() == "\u{23e3}")
        .expect("selected merge connector should include ⏣");
    // Selected merge → all glyphs (⏣ ─ ╮) are bolded white.
    for (offset, symbol) in ["\u{23e3}", "\u{2500}", "\u{256e}"].iter().enumerate() {
        let cell = &buf[(connector_start + offset as u16, 1)];
        assert_eq!(cell.symbol(), *symbol);
        assert_eq!(cell.fg, Color::White);
        assert_eq!(cell.bg, Color::DarkGray);
    }
}

#[test]
fn commits_panel_highlights_selected_side_commit() {
    // Two commits: a merge then its second-parent side commit. Selecting the side
    // commit should highlight its row (background) and bold its marker.
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];
    state.focus = Pane::Commits;
    state.commits_idx = 1;

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let marker_col = (0..buf.area.width)
        .find(|col| buf[(*col, 2)].symbol() == "\u{25ef}")
        .expect("selected commit marker");

    let marker = &buf[(marker_col, 2)];
    assert_eq!(marker.bg, Color::DarkGray);
}

#[test]
fn commits_panel_places_hash_and_author_before_graph() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut row = String::new();
    for col in 0..buf.area.width {
        row.push_str(buf[(col, 2)].symbol());
    }

    let hash = row.find("abc1234").expect("hash in row");
    let author = row.find("AE").expect("author in row");
    let graph = row.find("\u{25ef}").expect("graph marker in row");
    assert!(hash < author, "hash should precede author: {row}");
    assert!(author < graph, "author should precede graph: {row}");
}

#[test]
fn commits_panel_highlights_selected_row_without_shifting_columns() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];
    state.focus = Pane::Commits;
    state.commits_idx = 1;

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let marker = buf
        .content()
        .iter()
        .find(|cell| cell.symbol() == "\u{25ef}")
        .expect("selected graph marker");
    assert_eq!(marker.bg, Color::DarkGray);
}

#[test]
fn commits_panel_renders_nested_merges_like_lazygit() {
    // Simplified slice from the user's `feature/PNT-2594` log:
    //   M0 (merge feature into origin/main)  parents: [F, M1]
    //   M1 (merge spring-boot into main)    parents: [M2, S1]
    //   S1 (renovate spring-boot)           parents: [SP]
    //   M2 (merge kotlin-plugin)            parents: [P, K1]
    //   K1 (renovate kotlin)                parents: [P]
    //   P  (older main)                     parents: [G]
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "M0".into(),
            author: "Top".into(),
            author_short: "JT".into(),
            parents: vec!["F".into(), "M1".into()],
            is_first_parent: true,
            subject: "Merge origin/main into feature".into(),
        },
        Commit {
            sha: "M1".into(),
            author: "Sp".into(),
            author_short: "Sp".into(),
            parents: vec!["M2".into(), "S1".into()],
            is_first_parent: false,
            subject: "Merge spring-boot".into(),
        },
        Commit {
            sha: "S1".into(),
            author: "renovate".into(),
            author_short: "re".into(),
            parents: vec!["SP".into()],
            is_first_parent: false,
            subject: "Update spring boot".into(),
        },
        Commit {
            sha: "M2".into(),
            author: "Sp".into(),
            author_short: "Sp".into(),
            parents: vec!["P".into(), "K1".into()],
            is_first_parent: false,
            subject: "Merge kotlin".into(),
        },
        Commit {
            sha: "K1".into(),
            author: "renovate".into(),
            author_short: "re".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "Update kotlin".into(),
        },
        Commit {
            sha: "P".into(),
            author: "Sp".into(),
            author_short: "Sp".into(),
            parents: vec!["G".into()],
            is_first_parent: false,
            subject: "older main".into(),
        },
    ];

    let backend = TestBackend::new(120, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
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
    // M0: ⏣─╮ at col-pair 0/1 — top-level merge.
    assert!(
        row_text(1).contains("\u{23e3}\u{2500}\u{256e}"),
        "{}",
        row_text(1)
    );
    // M1: │ ⏣─╮ — feature lane stays vertical, merge lives on lane 1.
    assert!(
        row_text(2).contains("\u{2502} \u{23e3}\u{2500}\u{256e}"),
        "{}",
        row_text(2)
    );
    // S1: │ │ ◯ — both feature lane and main lane pass, S1 on lane 2.
    assert!(
        row_text(3).contains("\u{2502} \u{2502} \u{25ef}"),
        "{}",
        row_text(3)
    );
}

#[test]
fn commits_panel_dims_merged_in_commits() {
    let mut state = AppState::new();
    state.commits = vec![
        Commit {
            sha: "MERGE".into(),
            author: "Top".into(),
            author_short: "TP".into(),
            parents: vec!["P".into(), "abc1234".into()],
            is_first_parent: true,
            subject: "merge".into(),
        },
        Commit {
            sha: "abc1234".into(),
            author: "Alice Example".into(),
            author_short: "AE".into(),
            parents: vec!["P".into()],
            is_first_parent: false,
            subject: "side branch".into(),
        },
    ];

    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let text = buf
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        text.contains("\u{25ef}"),
        "missing merged-in marker: {text}"
    );
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "s" && cell.modifier.contains(Modifier::DIM)),
        "merged-in subject should be dimmed"
    );
}
