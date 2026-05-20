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
fn help_overlay_closes_on_any_key() {
    let mut state = make_state_with_files();
    state.focus = Pane::Files;
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
fn model_modal_picks_and_saves_model() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.llm_model = "test-model".into();
    app.state.llm_provider = lg::llm::LlmProvider::LlamaServer;

    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("llm llama-server/test-model"),
        "footer should show active model"
    );

    app.send_key(key(KeyCode::Char('L'))).unwrap();
    assert_eq!(app.state.modal, Modal::Model);
    assert_eq!(app.state.llm_model_input, app.state.llm_model);
    app.state.llm_provider = lg::llm::LlmProvider::LlamaServer;
    app.state.llm_provider_idx = 0;
    app.state.llm_model_idx = 0;
    app.state.llm_model_input = lg::config::LLM_MODEL_CHOICES[0].into();

    panel::model::handle_key(&mut app.state, key(KeyCode::Down)).unwrap();
    assert_eq!(app.state.llm_model_input, lg::config::LLM_MODEL_CHOICES[1]);

    panel::model::handle_key(&mut app.state, key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::SaveLlmSettings {
            model: lg::config::LLM_MODEL_CHOICES[1].into(),
            provider: lg::llm::LlmProvider::LlamaServer,
        })
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
            && context.contains("review note: updates greeting")
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
        rendered.contains("review note: updates nextStep (+1 -1)"),
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
fn review_source_inlines_style_and_llm_notes_and_jumps_between_them() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("App.kt");
    let mut source = String::from(
        "class App {\n    fun first() = \"changed\"\n    fun second() = \"changed\"\n",
    );
    for idx in 0..24 {
        source.push_str(&format!("    val untouched{idx} = {idx}\n"));
    }
    source.push_str("}\n");
    std::fs::write(&source_path, source).unwrap();
    let source_path = source_path.display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 24)).unwrap();
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
                title: format!("{source_path} - 2 entry points (+2 -2)"),
                body: vec![
                    "@@ -1,4 +1,4 @@".into(),
                    " class App {".into(),
                    "-    fun first() = \"old\"".into(),
                    "+    fun first() = \"changed\"".into(),
                    "-    fun second() = \"old\"".into(),
                    "+    fun second() = \"changed\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 1;
    app.state.review_style_findings.insert(
        source_path.clone(),
        ReviewStyleFinding {
            severity: ReviewStyleSeverity::Warn,
            reason: "Business rule belongs in a Service file.".into(),
        },
    );
    app.state.review_assists.insert(
        "branch:file:0".into(),
        "- This changes both greeting branches.".into(),
    );

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let rendered = buffer_text(&app);
    assert!(rendered.contains("review note: style warn"), "{rendered}");
    assert!(
        rendered.contains("review note: llm: This changes both"),
        "{rendered}"
    );

    let before = app.state.diff_offset;
    panel::main::handle_key(&mut app.state, key(KeyCode::Char('n'))).unwrap();
    assert!(
        app.state.diff_offset > before,
        "n should jump to the next inline note"
    );
    let first_note_offset = app.state.diff_offset;
    panel::main::handle_key(&mut app.state, key(KeyCode::Char('n'))).unwrap();
    let second_note_offset = app.state.diff_offset;
    assert!(
        second_note_offset >= first_note_offset,
        "second note jump should not move before the first note"
    );
    panel::main::handle_key(&mut app.state, key(KeyCode::Char('N'))).unwrap();
    assert!(
        app.state.diff_offset <= second_note_offset,
        "N should jump to the previous inline note"
    );
}

#[test]
fn review_panel_sources_full_diff_subtree_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let lib_path = dir.path().join("lib.rs");
    let app_path = dir.path().join("App.kt");
    std::fs::write(
        &lib_path,
        "pub fn greet() -> &'static str {\n    \"hello review\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        &app_path,
        "class App {\n    fun greeting() = \"hello review\"\n}\n",
    )
    .unwrap();
    let lib_path = lib_path.display().to_string();
    let app_path = app_path.display().to_string();

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
                title: "Full diff against main".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:file:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: format!("{lib_path} - 1 entry point (+1 -1)"),
                body: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " pub fn greet() -> &'static str {".into(),
                    "-    \"hello\"".into(),
                    "+    \"hello review\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:file:1".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: format!("{app_path} - 1 entry point (+1 -1)"),
                body: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " class App {".into(),
                    "-    fun greeting() = \"hello\"".into(),
                    "+    fun greeting() = \"hello review\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 0;

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let rendered = buffer_text(&app);

    assert!(
        app.state.review_context_open.contains("branch"),
        "full diff root should open source context"
    );
    assert!(
        rendered.contains(&format!("source {lib_path}")),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!("source {app_path}")),
        "{rendered}"
    );
    assert!(rendered.contains("+     \"hello review\""), "{rendered}");
    assert!(
        rendered.contains("+     fun greeting() = \"hello review\""),
        "{rendered}"
    );
}

#[test]
fn review_panel_sources_full_diff_from_report() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("lib.rs");
    std::fs::write(
        &source_path,
        "pub fn greet() -> &'static str {\n    \"hello review\"\n}\n",
    )
    .unwrap();
    let source_path = source_path.display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(160, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: format!(
            "Assisted review against main\n\nFull diff against main\n\
             diff --git a/{source_path} b/{source_path}\n\
             --- a/{source_path}\n\
             +++ b/{source_path}\n\
             @@ -1,3 +1,3 @@\n\
              pub fn greet() -> &'static str {{\n\
             -    \"hello\"\n\
             +    \"hello review\"\n\
              }}\n"
        ),
        nodes: vec![ReviewNode {
            id: "branch".into(),
            parent: None,
            depth: 0,
            title: "Full diff against main".into(),
            body: Vec::new(),
            context: Vec::new(),
        }],
    });
    app.state.review_idx = 0;

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let rendered = buffer_text(&app);

    assert!(app.state.review_context_open.contains("branch"));
    assert!(rendered.contains("source"), "{rendered}");
    assert!(rendered.contains("hello review"), "{rendered}");
}

#[test]
fn review_source_shows_removed_only_hunks_inline() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("App.kt");
    std::fs::write(&source_path, "class App {\n    fun kept() = \"ok\"\n}\n").unwrap();
    let source_path = source_path.display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(140, 28)).unwrap();
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
                title: format!("{source_path} - 1 entry point (+0 -1)"),
                body: vec![
                    "@@ -1,4 +1,3 @@".into(),
                    " class App {".into(),
                    "-    fun removed() = \"old\"".into(),
                    "     fun kept() = \"ok\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 1;

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let rendered = buffer_text(&app);

    assert!(
        rendered.contains("-     fun removed() = \"old\""),
        "{rendered}"
    );
    assert!(rendered.contains("fun kept()"), "{rendered}");
}

#[test]
fn review_source_falls_back_to_deleted_file_diff() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("Deleted.kt").display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(140, 28)).unwrap();
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
                title: format!("{source_path} - 1 entry point (+0 -2)"),
                body: vec![
                    "@@ -1,2 +0,0 @@".into(),
                    "-class Deleted {".into(),
                    "-    fun gone() = \"old\"".into(),
                ],
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 1;

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let rendered = buffer_text(&app);

    assert!(rendered.contains("│ diff"), "{rendered}");
    assert!(rendered.contains("-class Deleted"), "{rendered}");
    assert!(rendered.contains("-    fun gone()"), "{rendered}");
}

#[test]
fn review_source_reads_renamed_file_at_new_path() {
    let dir = tempfile::tempdir().unwrap();
    let old_path = dir.path().join("OldName.kt").display().to_string();
    let new_path_buf = dir.path().join("NewName.kt");
    std::fs::write(
        &new_path_buf,
        "class NewName {\n    fun renamed() = \"new\"\n}\n",
    )
    .unwrap();
    let new_path = new_path_buf.display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(160, 28)).unwrap();
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
                title: format!("{new_path} - 1 entry point (+1 -1)"),
                body: vec![
                    format!("diff --git a/{old_path} b/{new_path}"),
                    "rename from OldName.kt".into(),
                    "rename to NewName.kt".into(),
                    "@@ -1,3 +1,3 @@".into(),
                    "-class OldName {".into(),
                    "+class NewName {".into(),
                    "-    fun renamed() = \"old\"".into(),
                    "+    fun renamed() = \"new\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 1;

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    let rendered = buffer_text(&app);

    assert!(rendered.contains("source"), "{rendered}");
    assert!(rendered.contains("NewName.kt"), "{rendered}");
    assert!(rendered.contains("+ class NewName"), "{rendered}");
    assert!(
        rendered.contains("+     fun renamed() = \"new\""),
        "{rendered}"
    );
}

#[test]
fn review_panel_source_toggle_restores_collapsed_file() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("lib.rs");
    std::fs::write(
        &source_path,
        "pub fn greet() -> &'static str {\n    \"hello review\"\n}\n",
    )
    .unwrap();
    let source_path = source_path.display().to_string();

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(140, 32)).unwrap();
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
                id: "branch:file:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: format!("{source_path} - 1 entry point (+1 -1)"),
                body: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " pub fn greet() -> &'static str {".into(),
                    "-    \"hello\"".into(),
                    "+    \"hello review\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:entry:0".into(),
                parent: Some("branch:file:0".into()),
                depth: 2,
                title: format!("{source_path} in fn greet - updates greet (+1 -1)"),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 1;
    app.state.review_collapsed.insert("branch:file:0".into());

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();

    assert!(app.state.review_context_open.contains("branch:file:0"));
    assert!(!app.state.review_collapsed.contains("branch:file:0"));

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('s'))).unwrap();

    assert!(!app.state.review_context_open.contains("branch:file:0"));
    assert!(
        app.state.review_collapsed.contains("branch:file:0"),
        "closing source should restore the file's prior collapsed state"
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
        state.diff_offset, 3,
        "offset should keep the selection away from the viewport edge when possible"
    );
}

#[test]
fn review_panel_mouse_click_selects_visible_item() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Review;
    state.review = Some(AssistedReview {
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
                id: "branch:file:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: "src/lib.rs - 1 entry point (+1 -1)".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "summary".into(),
                parent: None,
                depth: 0,
                title: "Summary: feature vs origin/main (1 commit, 1 file)".into(),
                body: vec!["The branch changes 1 file across 1 commit.".into()],
                context: Vec::new(),
            },
        ],
    });

    panel::main::select_mouse_row(&mut state, Rect::new(40, 1, 80, 12), 3);
    assert_eq!(state.review_idx, 1);

    state.diff_offset = 1;
    panel::main::select_mouse_row(&mut state, Rect::new(40, 1, 80, 12), 2);
    assert_eq!(state.review_idx, 1);

    panel::main::select_mouse_row(&mut state, Rect::new(40, 1, 80, 12), 4);
    assert_eq!(state.review_idx, 2);
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
                title:
                    "src/main/kotlin/BalanceService.kt in fun greeting - updates greeting (+5 -3)"
                        .into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 1;
    app.state.review_style_findings.insert(
        "src/main/kotlin/BalanceService.kt".into(),
        ReviewStyleFinding {
            severity: ReviewStyleSeverity::Warn,
            reason: "Controller-style flow deserves manual attention.".into(),
        },
    );

    app.render().unwrap();
    let buf = app.terminal.backend().buffer().clone();
    let rendered = buffer_text(&app);

    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "s" && cell.fg == Color::LightCyan),
        "file path should be highlighted"
    );
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "B" && cell.bg == Color::Rgb(78, 57, 18)),
        "warning style finding should keep its warning background"
    );
    assert!(rendered.contains("style warn"), "{rendered}");
    assert!(
        rendered.contains("Controller-style flow deserves"),
        "{rendered}"
    );
    assert!(rendered.contains("manual attention."), "{rendered}");
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
fn review_panel_unanalyzed_paths_have_no_style_background() {
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
                title: "src/service.rs in fn service - updates service (+1 -0)".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });

    app.render().unwrap();
    let buf = app.terminal.backend().buffer().clone();

    assert!(
        !buf.content()
            .iter()
            .any(|cell| cell.bg == Color::Rgb(78, 57, 18)),
        "unflagged service paths should not receive the warning background"
    );
}

#[test]
fn review_panel_colors_style_severity_scale() {
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
                title: "src/main/kotlin/Good.kt in class Good - updates flow (+1 -0)".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: "branch:entry:1".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: "src/main/kotlin/Bad.kt in class Bad - updates flow (+1 -0)".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });
    app.state.review_style_findings.insert(
        "src/main/kotlin/Good.kt".into(),
        ReviewStyleFinding {
            severity: ReviewStyleSeverity::Ok,
            reason: "No style issue found.".into(),
        },
    );
    app.state.review_style_findings.insert(
        "src/main/kotlin/Bad.kt".into(),
        ReviewStyleFinding {
            severity: ReviewStyleSeverity::Fail,
            reason: "Direct Kafka publish from flow.".into(),
        },
    );

    app.render().unwrap();
    let buf = app.terminal.backend().buffer().clone();
    let rendered = buffer_text(&app);

    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "G" && cell.bg == Color::Rgb(24, 54, 34)),
        "OK style finding should be green"
    );
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "B" && cell.bg == Color::Rgb(70, 24, 28)),
        "FAIL style finding should be red"
    );
    assert!(rendered.contains("style ok"), "{rendered}");
    assert!(rendered.contains("style fail"), "{rendered}");
}

#[test]
fn review_panel_marks_active_style_analysis_separately() {
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
                title: "src/main/kotlin/BalanceService.kt in class BalanceService - updates flow (+1 -0)"
                    .into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    });
    app.state.review_flag_active_path = Some("src/main/kotlin/BalanceService.kt".into());

    app.render().unwrap();
    let buf = app.terminal.backend().buffer().clone();

    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "B" && cell.bg == Color::Rgb(28, 48, 70)),
        "active style analysis should have its own background"
    );
}

#[test]
fn review_panel_renders_summary_body_as_markdown_without_cutoff() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(70, 14)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.diff_viewport_width = 68;
    app.state.review = Some(AssistedReview {
        report: "flat report".into(),
        nodes: vec![ReviewNode {
            id: "summary".into(),
            parent: None,
            depth: 0,
            title: "Summary: feature/example vs origin/main (12 commits, 20 files)".into(),
            body: vec![
                "- **Generated summary** keeps alpha beta gamma delta epsilon zeta eta theta iota kappa lambda omega.".into(),
            ],
            context: Vec::new(),
        }],
    });

    app.render().unwrap();
    let rendered = buffer_text(&app);

    assert!(rendered.contains("• Generated summary"), "{rendered}");
    assert!(
        rendered.contains("lambda omega"),
        "summary body should wrap instead of truncating: {rendered}"
    );
    let buf = app.terminal.backend().buffer();
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "•" && cell.fg == Color::Yellow),
        "summary markdown bullet should be rendered: {rendered}"
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
fn review_panel_explains_selected_subtree_with_llm() {
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
    assert!(rendered.contains("llm"), "{rendered}");
    assert!(
        rendered.contains("Explains the greeting change."),
        "{rendered}"
    );
}

#[test]
fn review_panel_explains_full_diff_with_llm() {
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
                id: "branch:file:0".into(),
                parent: Some("branch".into()),
                depth: 1,
                title: "src/lib.rs - 1 entry point (+1 -1)".into(),
                body: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " pub fn greet() -> &'static str {".into(),
                    "-    \"hello\"".into(),
                    "+    \"hello review\"".into(),
                    " }".into(),
                ],
                context: Vec::new(),
            },
        ],
    });
    app.state.review_idx = 0;
    app.state.review_collapsed.insert("branch".into());

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('l'))).unwrap();

    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::ReviewAssist("branch".into()))
    );
    assert!(
        !app.state.review_collapsed.contains("branch"),
        "full diff subtree should open before explaining"
    );

    app.state
        .review_assists
        .insert("branch".into(), "Explains the whole diff.".into());
    app.render().unwrap();
    let rendered = buffer_text(&app);
    assert!(rendered.contains("llm"), "{rendered}");
    assert!(rendered.contains("Explains the whole diff."), "{rendered}");
}

#[test]
fn review_panel_opens_chat_about_full_review() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(AssistedReview {
        report: "Assisted review against main\nFull diff against main\nsrc/lib.rs:2".into(),
        nodes: vec![ReviewNode {
            id: "branch".into(),
            parent: None,
            depth: 0,
            title: "Full diff against main".into(),
            body: Vec::new(),
            context: Vec::new(),
        }],
    });

    panel::main::handle_key(&mut app.state, key(KeyCode::Char('C'))).unwrap();

    assert_eq!(app.state.modal, Modal::ReviewChat);

    panel::review_chat::handle_key(&mut app.state, key(KeyCode::Char('w'))).unwrap();
    panel::review_chat::handle_key(&mut app.state, key(KeyCode::Char('e'))).unwrap();
    panel::review_chat::handle_key(&mut app.state, key(KeyCode::Char('a'))).unwrap();
    panel::review_chat::handle_key(&mut app.state, key(KeyCode::Char('k'))).unwrap();
    panel::review_chat::handle_key(&mut app.state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::ReviewChat("weak".into()))
    );
    assert_eq!(app.state.review_chat_messages.len(), 1);
    assert_eq!(app.state.review_chat_messages[0].role, ReviewChatRole::User);
    assert_eq!(app.state.review_chat_messages[0].content, "weak");
    assert!(app.state.review_chat_input.is_empty());
}

#[test]
fn review_chat_docked_renders_markdown_conversation() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.modal = Modal::ReviewChat;
    app.state.diff_text = "diff --git a/src/lib.rs b/src/lib.rs".into();
    app.state.review_chat_height = Some(18);
    app.state
        .review_chat_messages
        .push(lg::state::ReviewChatMessage {
            role: ReviewChatRole::User,
            content: "find weaknesses".into(),
        });
    app.state
        .review_chat_messages
        .push(lg::state::ReviewChatMessage {
            role: ReviewChatRole::Assistant,
            content: "- **Risk** in `src/lib.rs:2` needs test coverage.".into(),
        });

    app.render().unwrap();

    let rendered = buffer_text(&app);
    assert!(rendered.contains("diff --git"), "{rendered}");
    assert!(rendered.contains("Review chat"), "{rendered}");
    assert!(rendered.contains("you"), "{rendered}");
    assert!(rendered.contains("llm"), "{rendered}");
    assert!(
        rendered.contains("• Risk in src/lib.rs:2 needs test coverage."),
        "{rendered}"
    );
    let buf = app.terminal.backend().buffer();
    assert!(
        buf.content()
            .iter()
            .any(|cell| cell.symbol() == "R" && cell.modifier.contains(Modifier::BOLD)),
        "bold markdown should render in chat: {rendered}"
    );
}

#[test]
fn review_chat_docks_under_review_context() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.modal = Modal::ReviewChat;
    app.state.review = Some(AssistedReview {
        report: "Assisted review against main".into(),
        nodes: (0..10)
            .map(|idx| ReviewNode {
                id: format!("node-{idx}"),
                parent: None,
                depth: 0,
                title: if idx == 7 {
                    "Keep this review context visible".into()
                } else {
                    format!("Review node {idx}")
                },
                body: Vec::new(),
                context: Vec::new(),
            })
            .collect(),
    });
    app.state
        .review_chat_messages
        .push(lg::state::ReviewChatMessage {
            role: ReviewChatRole::User,
            content: "why this change?".into(),
        });

    app.render().unwrap();

    let rendered = buffer_text(&app);
    assert!(
        rendered.contains("Keep this review context visible"),
        "{rendered}"
    );
    assert!(rendered.contains("Review chat"), "{rendered}");
    assert!(rendered.contains("why this change?"), "{rendered}");
}

#[test]
fn review_chat_mouse_scrolls_and_resizes_when_docked() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 32)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.modal = Modal::ReviewChat;
    app.state.review = Some(AssistedReview {
        report: "Assisted review against main".into(),
        nodes: vec![ReviewNode {
            id: "branch".into(),
            parent: None,
            depth: 0,
            title: "Full diff against main".into(),
            body: Vec::new(),
            context: Vec::new(),
        }],
    });
    for idx in 0..12 {
        app.state
            .review_chat_messages
            .push(lg::state::ReviewChatMessage {
                role: ReviewChatRole::Assistant,
                content: format!("chat line {idx}"),
            });
    }

    app.render().unwrap();
    let area = Rect::new(0, 0, 120, 32);
    let rects = lg::ui::split_layout_with_sizes(
        area,
        app.state.environments_visible(),
        app.state.left_column_width,
        app.state.left_panel_heights,
    );
    let chunks = panel::main::review_chat_layout(&app.state, rects.main);
    let chat = chunks[1];
    let initial_height = chat.height;

    app.send_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: chat.x.saturating_add(2),
        row: chat.y.saturating_add(2),
        modifiers: KeyModifiers::NONE,
    })
    .unwrap();
    assert_eq!(app.state.review_chat_scroll, 3);

    app.send_mouse(left_click(chat.x.saturating_add(2), chat.y))
        .unwrap();
    app.send_mouse(left_drag(
        chat.x.saturating_add(2),
        chat.y.saturating_sub(3),
    ))
    .unwrap();

    assert!(
        app.state.review_chat_height.unwrap_or_default() > initial_height,
        "dragging the splitter upward should increase docked chat height"
    );
}

#[test]
fn review_panel_renders_llm_markdown() {
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
