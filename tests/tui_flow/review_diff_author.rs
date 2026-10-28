use super::common::*;

// ── Panel transitions ─────────────────────────────────────────────────────────

#[test]
fn pressing_c_without_changed_files_does_not_open_commit_modal() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.send_key(key(KeyCode::Char('c'))).unwrap();
    assert_eq!(app.state.modal, Modal::None);
    assert_eq!(
        app.state.status.as_ref().map(|status| status.text.as_str()),
        Some("nothing to commit")
    );
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
fn help_overlay_closes_on_any_non_scroll_key() {
    let mut state = make_state_with_files();
    state.focus = Pane::Files;
    state.prev_focus = Pane::Files;
    state.modal = Modal::Help;

    panel::help::handle_key(
        &mut state,
        key(KeyCode::Char('?')),
        Rect::new(0, 0, 100, 40),
    )
    .unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.focus, Pane::Files);
}

#[test]
fn main_footer_and_help_call_out_the_guided_review() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 80)).unwrap();
    app.state.focus = Pane::Main;
    app.state.repo_root = Some("/nonexistent-lg-repo".into());
    app.render().unwrap();

    let footer = buffer_text(&app);
    assert!(
        footer.contains("guided review"),
        "main footer should name the guided review: {footer}"
    );

    app.state.modal = Modal::Help;
    app.state.prev_focus = Pane::Main;
    // Read the whole table: the overlay opens at the focused pane's own
    // section, so what is on screen at any one moment is only part of it.
    let help = help_text(&mut app, Rect::new(0, 0, 120, 80));
    assert!(
        help.contains("Guided review (on main: what is not pushed)"),
        "help should describe how to start a guided review: {help}"
    );
    assert!(
        !help.contains("Review mode"),
        "the old review mode is gone: {help}"
    );
}

#[test]
fn author_modal_saves_repository_identity_by_default() {
    let mut state = AppState::new();
    state.modal = Modal::Author;
    state.author.path = "/tmp/example-work".into();
    state.author.name = "Example User".into();
    state.author.email = "example@example.com".into();
    state.author.field = AuthorField::Email;

    panel::author::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::SaveAuthor {
            name: "Example User".into(),
            email: "example@example.com".into(),
        })
    );
}

#[test]
fn author_modal_can_save_repo_local_author() {
    let mut state = AppState::new();
    state.modal = Modal::Author;
    state.author.name = "Example User".into();
    state.author.email = "example@example.com".into();

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
    app.state.author.path = "/tmp/example".into();
    app.state.author.name = "Example User".into();
    app.state.author.email = "a@b".into();
    app.state.author.field = AuthorField::Email;

    app.render().unwrap();

    app.terminal.backend_mut().assert_cursor_position((24, 15));
}

#[test]
fn author_modal_shows_error_when_terminal_is_too_small() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(20, 6)).unwrap();
    app.state.modal = Modal::Author;
    app.state.author.field = AuthorField::Email;

    app.render().unwrap();

    let text = buffer_text(&app);
    assert!(
        text.contains("Terminal too small"),
        "missing small-terminal message: {text}"
    );
}

#[test]
fn model_modal_picks_and_saves_model() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.llm_model = "test-model".into();
    app.state.llm_provider = lg::llm::LlmProvider::Mtplx;

    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("llm mtplx/test-model"),
        "footer should show active model"
    );

    app.send_key(key(KeyCode::Char('L'))).unwrap();
    assert_eq!(app.state.modal, Modal::Model);
    assert_eq!(app.state.llm_model_input, app.state.llm_model);
    // Opening the modal may kick off the history scan, which locks the rows
    // until it answers; this test is about editing them afterwards.
    app.state.settings_suggest_job = None;
    app.state.llm_provider = lg::llm::LlmProvider::Mtplx;
    app.state.llm_provider_idx = 0;
    app.state.llm_model_idx = 0;
    app.state.llm_model_input = lg::config::LLM_MODEL_CHOICES[0].into();

    // Enter opens the model row's list; Down wraps, so with a single configured
    // choice it stays put. Enter confirms the pick without saving.
    let next = lg::config::LLM_MODEL_CHOICES[1 % lg::config::LLM_MODEL_CHOICES.len()];
    app.state.settings_field = lg::state::SettingsField::Model;
    panel::model::handle_key(&mut app.state, key(KeyCode::Enter)).unwrap();
    panel::model::handle_key(&mut app.state, key(KeyCode::Down)).unwrap();
    assert_eq!(app.state.llm_model_input, next);
    panel::model::handle_key(&mut app.state, key(KeyCode::Enter)).unwrap();
    assert_eq!(app.state.pending_action, None);

    app.state.settings_pr_language_input = "Norwegian".into();
    app.state.settings_comment_style_input = "terse, imperative".into();
    app.state.settings_field = lg::state::SettingsField::Save;
    panel::model::handle_key(&mut app.state, key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::SaveSettings {
            model: next.into(),
            provider: lg::llm::LlmProvider::Mtplx,
            pr_language: "Norwegian".into(),
            comment_style: "terse, imperative".into(),
        })
    );
}

/// What the pane drew for a node is kept between frames, but a review built
/// again — with the same shape and different text — must be drawn anew.
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
fn side_by_side_diff_highlights_java_syntax() {
    let rows = lg::ui::highlight_side_by_side_diff_text_for_path(
        "@@ -1,2 +1,2 @@\n-import java.util.List;\n+import module java.base;\n-public final class Shape {\n+public sealed interface Shape permits Circle {",
        160,
        "src/main/java/org/example/Shape.java",
    );
    let is_keyword = |word: &str| {
        rows.iter().flat_map(|row| &row.spans).any(|span| {
            span.content.as_ref() == word
                && span.style.fg == Some(Color::Yellow)
                && span.style.add_modifier.contains(Modifier::BOLD)
        })
    };

    for word in [
        "import",
        "module",
        "public",
        "final",
        "class",
        "sealed",
        "interface",
        "permits",
    ] {
        assert!(
            is_keyword(word),
            "{word} should be highlighted as a Java keyword"
        );
    }
}

#[test]
fn diff_line_highlighting_follows_the_syntax_of_the_path() {
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
fn diff_pane_o_opens_markdown_file_from_diff() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::All;
    state.diff_text = [
        "diff --git a/README.md b/README.md",
        "--- a/README.md",
        "+++ b/README.md",
        "@@ -1 +1 @@",
    ]
    .join("\n");

    panel::main::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::OpenFile("README.md".into()))
    );
}

#[test]
fn with_ai_assist_off_the_commit_modal_is_typed_by_hand() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state.ai_assist = false;
    app.state.files = vec![FileEntry {
        path: "b.rs".into(),
        x: 'A',
        y: ' ',
    }];

    app.send_key(key(KeyCode::Char('c'))).unwrap();
    assert_eq!(app.state.modal, Modal::Commit);
    assert_eq!(
        app.state.pending_action, None,
        "nothing is asked of the model"
    );
    assert!(!app.state.generating());

    for ch in "fix".chars() {
        app.send_key(key(KeyCode::Char(ch))).unwrap();
    }
    assert_eq!(app.state.commit_message, "fix");

    app.send_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.state.pending_action, None,
        "Ctrl-R does not start a generation"
    );
    assert_eq!(app.state.commit_message, "fix", "the typed message is kept");

    app.send_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.state.pending_action, Some(PendingAction::Commit));
}
