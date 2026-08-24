use super::common::*;

// ── Destructive actions require confirmation ──────────────────────────────────

#[test]
fn deleting_a_file_asks_before_touching_disk() {
    let mut state = make_state_with_files();
    state.focus = Pane::Files;
    state.files_idx = 1; // skip the "All changes" root row

    panel::files::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(
        state.pending_action, None,
        "the delete must not be dispatched before the user confirms"
    );
    let prompt = state.confirm.clone().expect("confirm prompt");
    assert!(matches!(prompt.action, PendingAction::DeletePath { .. }));
}

#[test]
fn confirming_a_delete_dispatches_the_original_action() {
    let mut state = make_state_with_files();
    state.focus = Pane::Files;
    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();
    let expected = state.confirm.clone().unwrap().action;

    panel::confirm::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.pending_action, Some(expected));
    assert!(state.confirm.is_none());
}

#[test]
fn cancelling_a_delete_dispatches_nothing() {
    let mut state = make_state_with_files();
    state.focus = Pane::Files;
    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();

    panel::confirm::handle_key(&mut state, key(KeyCode::Esc)).unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.pending_action, None);
    assert!(state.confirm.is_none());
}

#[test]
fn rollback_also_asks_before_discarding_changes() {
    let mut state = make_state_with_files();
    state.focus = Pane::Files;
    state.files_idx = 1;

    panel::files::handle_key(&mut state, key(KeyCode::Char('r'))).unwrap();

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(state.pending_action, None);
    assert!(matches!(
        state.confirm.as_ref().map(|c| &c.action),
        Some(PendingAction::RollbackPath { .. })
    ));
}

#[test]
fn unrelated_keys_leave_the_confirm_prompt_open() {
    let mut state = make_state_with_files();
    state.focus = Pane::Files;
    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();

    panel::confirm::handle_key(&mut state, key(KeyCode::Char('x'))).unwrap();

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert_eq!(state.pending_action, None);
}

#[test]
fn confirm_prompt_renders_the_path_and_the_warning() {
    let mut state = AppState::new();
    state.confirm_action(
        "Delete",
        "Delete this folder from disk?",
        "src/panel",
        PendingAction::DeletePath {
            path: "src/panel".into(),
            is_dir: true,
        },
    );

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| panel::confirm::render(&state, frame.area(), frame))
        .unwrap();

    let rendered = terminal_text(&terminal);
    assert!(
        rendered.contains("Delete this folder from disk?"),
        "{rendered}"
    );
    assert!(
        rendered.contains("src/panel"),
        "the prompt must name what is about to be destroyed:\n{rendered}"
    );
    assert!(rendered.contains("This cannot be undone."), "{rendered}");
}

// ── Help is scrollable ────────────────────────────────────────────────────────

#[test]
fn help_content_exceeds_a_normal_terminal() {
    // The whole point of the scroll support: the help text does not fit on screen.
    let area = Rect::new(0, 0, 100, 40);
    assert!(
        panel::help::max_offset(area) > 0,
        "expected the help text to overflow a 40-row terminal"
    );
}

#[test]
fn help_scrolls_down_and_clamps_at_the_end() {
    let area = Rect::new(0, 0, 100, 40);
    let mut state = AppState::new();
    state.modal = Modal::Help;

    panel::help::handle_key(&mut state, key(KeyCode::Char('j')), area).unwrap();
    assert_eq!(state.help_offset, 1);
    assert_eq!(state.modal, Modal::Help, "j must scroll, not close");

    panel::help::handle_key(&mut state, key(KeyCode::Char('G')), area).unwrap();
    assert_eq!(state.help_offset, panel::help::max_offset(area));

    panel::help::handle_key(&mut state, key(KeyCode::Char('j')), area).unwrap();
    assert_eq!(
        state.help_offset,
        panel::help::max_offset(area),
        "must not scroll past the last line"
    );
}

#[test]
fn help_scrolls_up_and_clamps_at_the_top() {
    let area = Rect::new(0, 0, 100, 40);
    let mut state = AppState::new();
    state.modal = Modal::Help;
    state.help_offset = 2;

    panel::help::handle_key(&mut state, key(KeyCode::Char('k')), area).unwrap();
    assert_eq!(state.help_offset, 1);
    panel::help::handle_key(&mut state, key(KeyCode::Char('k')), area).unwrap();
    panel::help::handle_key(&mut state, key(KeyCode::Char('k')), area).unwrap();
    assert_eq!(state.help_offset, 0);
}

#[test]
fn help_closes_on_esc_and_resets_scroll() {
    let area = Rect::new(0, 0, 100, 40);
    let mut state = AppState::new();
    state.modal = Modal::Help;
    state.help_offset = 5;

    panel::help::handle_key(&mut state, key(KeyCode::Esc), area).unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.help_offset, 0);
}

#[test]
fn help_renders_later_sections_once_scrolled() {
    let area = Rect::new(0, 0, 100, 40);
    let mut state = AppState::new();
    state.modal = Modal::Help;
    state.help_offset = panel::help::max_offset(area);

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| panel::help::render(&state, frame.area(), frame))
        .unwrap();

    let rendered = terminal_text(&terminal);
    assert!(
        rendered.contains("Push modal"),
        "the last help section should be reachable by scrolling, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Global"),
        "the first section should have scrolled out of view:\n{rendered}"
    );
}

fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for row in 0..buffer.area.height {
        for col in 0..buffer.area.width {
            out.push_str(buffer[(col, row)].symbol());
        }
        out.push('\n');
    }
    out
}

// ── Cancelling in-flight LLM work ─────────────────────────────────────────────

#[test]
fn esc_cancels_a_running_review_and_says_so() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    let (_tx, rx) = mpsc::channel();
    app.state.review_job = Some(lg::state::ReviewJob {
        rx,
        handle: None,
        spinner: 0,
    });
    app.state.diff_text = "building assisted review against main...".into();
    assert!(app.state.llm_job_running());

    app.send_key(key(KeyCode::Esc)).unwrap();

    assert!(app.state.review_job.is_none());
    assert!(!app.state.llm_job_running());
    assert_eq!(
        app.state.status.as_ref().map(|s| s.text.as_str()),
        Some("review cancelled")
    );
    assert_eq!(
        app.state.diff_text, "review cancelled",
        "the pane must stop claiming the review is still building"
    );
}

#[test]
fn cancelling_actually_stops_the_worker_not_just_the_display() {
    // The whole cancellation design rests on this: dropping the job drops the
    // receiver, and the llm stream loop returns as soon as a send fails.
    let mut state = AppState::new();
    let (tx, rx) = mpsc::channel::<lg::state::GenMsg>();
    let (observed_tx, observed_rx) = mpsc::channel();

    let worker = std::thread::spawn(move || {
        for _ in 0..10_000 {
            if tx.send(lg::state::GenMsg::Output("chunk".into())).is_err() {
                let _ = observed_tx.send("disconnected");
                return;
            }
            std::thread::yield_now();
        }
        let _ = observed_tx.send("ran to completion");
    });

    state.review_chat_job = Some(lg::state::ReviewChatJob {
        rx,
        handle: None,
        output: String::new(),
        spinner: 0,
    });

    assert_eq!(state.cancel_llm_jobs(), Some("review chat cancelled"));

    worker.join().unwrap();
    assert_eq!(
        observed_rx.recv().unwrap(),
        "disconnected",
        "the worker must observe the cancellation, not run to completion"
    );
}

#[test]
fn esc_dismisses_an_error_before_it_cancels_anything() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    let (_tx, rx) = mpsc::channel();
    app.state.review_job = Some(lg::state::ReviewJob {
        rx,
        handle: None,
        spinner: 0,
    });
    app.state.set_status("something failed", true);

    app.send_key(key(KeyCode::Esc)).unwrap();

    assert!(app.state.status.is_none(), "first Esc clears the error");
    assert!(
        app.state.review_job.is_some(),
        "and must not also cancel the job in the same keypress"
    );

    app.send_key(key(KeyCode::Esc)).unwrap();
    assert!(app.state.review_job.is_none(), "second Esc cancels");
}

#[test]
fn esc_with_nothing_running_leaves_state_alone() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    app.state.focus = Pane::Commits;

    app.send_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.state.focus, Pane::Commits);
    assert!(app.state.status.is_none());
}

#[test]
fn starting_a_second_review_reports_instead_of_doing_nothing() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    let (_tx, rx) = mpsc::channel();
    app.state.review_job = Some(lg::state::ReviewJob {
        rx,
        handle: None,
        spinner: 0,
    });

    app.send_key(key(KeyCode::Char('R'))).unwrap();

    let status = app.state.status.as_ref().expect("a status message");
    assert!(
        status.text.contains("already running"),
        "got {:?}",
        status.text
    );
}

#[test]
fn esc_in_review_chat_cancels_the_answer_before_closing() {
    let mut state = AppState::new();
    state.modal = Modal::ReviewChat;
    let (_tx, rx) = mpsc::channel();
    state.review_chat_job = Some(lg::state::ReviewChatJob {
        rx,
        handle: None,
        output: String::new(),
        spinner: 0,
    });

    panel::review_chat::handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    assert!(
        state.review_chat_job.is_none(),
        "first Esc stops the answer"
    );
    assert_eq!(
        state.modal,
        Modal::ReviewChat,
        "and leaves the chat open so the user sees what they have"
    );

    panel::review_chat::handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    assert_eq!(state.modal, Modal::None, "second Esc closes the chat");
}

/// The prompt is the last thing standing between a keypress and a deleted
/// branch, so every step it names has to survive rendering — the modal does not
/// scroll, and a step cut off is a step nobody agreed to.
#[test]
fn a_multi_step_confirm_shows_every_step_it_names() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    app.state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree(
            "/Users/someone/dev/very/deep/workspace.worktrees/feat-x",
            "feat/x",
        ),
    ];
    app.state.focus = Pane::Status;
    app.state.nested_repo_tree_idx = 1;
    app.send_key(key(KeyCode::Char('m'))).unwrap();

    app.render().unwrap();
    let screen = buffer_text(&app);

    for step in [
        "Merge feat/x into main and clean up?",
        "merge feat/x into main",
        "push main",
        "remove /Users/someone/dev/very/deep/workspace.worktrees/feat-x",
        "delete feat/x and origin/feat/x",
    ] {
        assert!(screen.contains(step), "the prompt hides {step:?}: {screen}");
    }
    assert!(
        screen.contains("y confirm"),
        "the keys are still reachable below the steps: {screen}"
    );
}
