use lg::{
    git::{
        AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, ReleaseTargetStatus,
        ReviewNode,
    },
    panel,
    state::{
        AppState, AuthorField, Modal, Pane, PendingAction, ReleaseStatusJob, TreeKind, WorkflowJob,
        build_tree_rows,
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
        },
        Branch {
            name: "release/next".into(),
            is_current: false,
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

    panel::commit::handle_key(&mut state, key(KeyCode::Backspace)).unwrap();

    assert_eq!(state.commit_message, "hell");
}

#[test]
fn commit_input_accepts_long_multiline_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "x".repeat(2_048);

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
    add_flow_branches(&mut app.state);
    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

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
    assert_eq!(resized.commits.height, 4);
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
            graph: "*".into(),
            is_first_parent: true,
            parent_count: 1,
            subject: "add feature".into(),
        },
        Commit {
            sha: "def5678".into(),
            author: "Bob Example".into(),
            author_short: "BE".into(),
            graph: "*".into(),
            is_first_parent: true,
            parent_count: 1,
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
fn commits_panel_marks_merge_commits() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        graph: "*".into(),
        is_first_parent: true,
        parent_count: 2,
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

    assert!(text.contains("\u{25cb}"), "missing merge marker: {text}");
    assert!(
        text.contains("\u{21a9}"),
        "missing merge return arrow: {text}"
    );
}

#[test]
fn commits_panel_places_hash_and_author_before_graph() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        graph: "| *".into(),
        is_first_parent: false,
        parent_count: 1,
        subject: "side branch".into(),
    }];

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
        row.push_str(buf[(col, 1)].symbol());
    }

    let hash = row.find("abc1234").expect("hash in row");
    let author = row.find("AE").expect("author in row");
    let graph = row.find("\u{25cb}").expect("graph marker in row");
    assert!(hash < author, "hash should precede author: {row}");
    assert!(author < graph, "author should precede graph: {row}");
}

#[test]
fn commits_panel_highlights_selected_row_without_shifting_columns() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        graph: "| *".into(),
        is_first_parent: false,
        parent_count: 1,
        subject: "side branch".into(),
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
    assert_eq!(buf[(1, 1)].symbol(), "a");
    let marker = buf
        .content()
        .iter()
        .find(|cell| cell.symbol() == "\u{25cb}")
        .expect("selected graph marker");
    assert_eq!(marker.fg, Color::LightMagenta);
    assert_eq!(marker.bg, Color::DarkGray);
}

#[test]
fn commits_panel_dims_merged_in_commits() {
    let mut state = AppState::new();
    state.commits = vec![Commit {
        sha: "abc1234".into(),
        author: "Alice Example".into(),
        author_short: "AE".into(),
        graph: "| *".into(),
        is_first_parent: false,
        parent_count: 1,
        subject: "side branch".into(),
    }];

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
        text.contains("\u{25cb}"),
        "missing merged-in marker: {text}"
    );
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "s" && cell.modifier.contains(Modifier::DIM)),
        "merged-in subject should be dimmed"
    );
}
