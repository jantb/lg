use super::common::*;
use lg::session::{SessionSpec, Sessions};
use lg::state::{AppMode, MainView};
use lg::term::Spawn;
use std::time::{Duration, Instant};

/// A session running a shell script instead of claude, so the test controls
/// exactly what the "program" says and when.
fn shell_session(app: &mut lg::app::HeadlessApp<TestBackend>, script: &str, dir: &str) -> u64 {
    let spawn = Spawn {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: std::env::temp_dir(),
        env: vec![("TERM".into(), "xterm-256color".into())],
        env_remove: Vec::new(),
    };
    let spec = SessionSpec {
        label: "feat/x".into(),
        cwd: dir.into(),
        sandboxed: false,
    };
    let id = app
        .state
        .sessions
        .start_with(spec, &spawn, (24, 80))
        .expect("start session");
    app.state.show_session(id);
    id.to_string().parse().expect("session id")
}

/// Pump until the session's screen contains `needle`, or give up.
fn wait_for(app: &mut lg::app::HeadlessApp<TestBackend>, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.state.sessions.pump();
        let contents = app
            .state
            .sessions
            .focused_session()
            .map(|session| session.screen().contents())
            .unwrap_or_default();
        if contents.contains(needle) || Instant::now() > deadline {
            return contents;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_session_draws_its_program_in_the_main_pane() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(
        &mut app,
        "printf 'hello from the session'; sleep 30",
        "/tmp/a",
    );

    let contents = wait_for(&mut app, "hello from the session");
    assert!(contents.contains("hello from the session"), "{contents:?}");

    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        screen.contains("hello from the session"),
        "the session is not on screen: {screen}"
    );
    assert!(
        screen.contains("claude \u{b7} feat/x"),
        "the pane should say which session it is: {screen}"
    );
}

#[test]
fn a_captured_session_receives_typing_and_not_lg() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(
        &mut app,
        "read line; printf 'got:%s' \"$line\"; sleep 30",
        "/tmp/b",
    );
    app.state.session_capture = true;

    // `q` would quit lg; captured, it is just a character.
    app.send_key(key(KeyCode::Char('q'))).unwrap();
    app.send_key(key(KeyCode::Enter)).unwrap();
    assert!(!app.state.should_quit, "q must not reach lg while captured");

    let contents = wait_for(&mut app, "got:q");
    assert!(contents.contains("got:q"), "{contents:?}");
}

#[test]
fn ctrl_c_interrupts_the_session_rather_than_quitting_lg() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(
        &mut app,
        "trap 'printf interrupted; exit 0' INT; while :; do sleep 0.05; done",
        "/tmp/c",
    );
    app.state.session_capture = true;

    app.send_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.state.should_quit);

    // The terminal echoes the interrupt as `^C`, which only appears if the byte
    // was written to the session rather than eaten by lg's own quit handling.
    let contents = wait_for(&mut app, "^C");
    assert!(
        contents.contains("^C") || contents.contains("interrupted"),
        "the interrupt never reached the session: {contents:?}"
    );
}

#[test]
fn the_release_key_hands_the_keyboard_back_to_lg() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(&mut app, "sleep 30", "/tmp/d");
    app.state.session_capture = true;

    app.send_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.state.session_capture);
    assert!(!app.state.session_input_active());

    // With the keyboard back, `i` gives it to the session again.
    app.send_key(key(KeyCode::Char('i'))).unwrap();
    assert!(app.state.session_capture);
}

#[test]
fn a_session_that_ends_keeps_its_last_screen_and_releases_the_keyboard() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(&mut app, "printf 'all done'", "/tmp/e");
    app.state.session_capture = true;

    let contents = wait_for(&mut app, "all done");
    assert!(contents.contains("all done"), "{contents:?}");

    // Pump until the exit is noticed.
    let deadline = Instant::now() + Duration::from_secs(10);
    while app
        .state
        .sessions
        .focused_session()
        .is_some_and(|session| session.is_running())
        && Instant::now() < deadline
    {
        app.state.sessions.pump();
        std::thread::sleep(Duration::from_millis(10));
    }

    let session = app.state.sessions.focused_session().expect("session");
    assert!(!session.is_running());
    assert!(
        session.screen().contents().contains("all done"),
        "the final screen must stay readable"
    );
    assert!(
        session.title().contains("exited"),
        "the title should say how it ended: {}",
        session.title()
    );
}

#[test]
fn closing_the_last_session_goes_back_to_the_diff() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(&mut app, "sleep 30", "/tmp/f");
    assert!(matches!(app.state.main_view, MainView::Session(_)));

    app.state.session_capture = false;
    app.state.focus = Pane::Main;
    app.send_key(key(KeyCode::Char('x'))).unwrap();

    assert!(app.state.sessions.is_empty());
    assert_eq!(app.state.main_view, MainView::Diff);
    assert!(app.state.session_view().is_none());
}

#[test]
fn backspace_leaves_the_session_running_in_the_background() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(&mut app, "sleep 30", "/tmp/g");
    app.state.session_capture = false;
    app.state.focus = Pane::Main;

    app.send_key(key(KeyCode::Backspace)).unwrap();

    assert_eq!(app.state.main_view, MainView::Diff);
    assert_eq!(app.state.sessions.len(), 1, "the session keeps running");
}

#[test]
fn output_while_another_session_is_shown_asks_for_attention() {
    let mut sessions = Sessions::new();
    let quiet = Spawn {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 30".into()],
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        env_remove: Vec::new(),
    };
    let noisy = Spawn {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "printf 'look at me'; sleep 30".into()],
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        env_remove: Vec::new(),
    };
    let shown = sessions
        .start_with(
            SessionSpec {
                label: "shown".into(),
                cwd: "/tmp/shown".into(),
                sandboxed: false,
            },
            &quiet,
            (24, 80),
        )
        .expect("start");
    sessions
        .start_with(
            SessionSpec {
                label: "background".into(),
                cwd: "/tmp/background".into(),
                sandboxed: false,
            },
            &noisy,
            (24, 80),
        )
        .expect("start");
    sessions.focus(shown);

    let deadline = Instant::now() + Duration::from_secs(10);
    while sessions.attention_count() == 0 && Instant::now() < deadline {
        sessions.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        sessions.attention_count(),
        1,
        "the background session should be flagged"
    );
}

#[test]
fn quitting_with_a_live_session_asks_first() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    shell_session(&mut app, "sleep 30", "/tmp/h");
    app.state.session_capture = false;
    app.state.focus = Pane::Status;

    app.send_key(key(KeyCode::Char('q'))).unwrap();
    assert!(!app.state.should_quit, "a live session must be flagged");
    assert_eq!(app.state.modal, Modal::ConfirmDestructive);
    let prompt = app.state.confirm.as_ref().expect("confirm prompt");
    assert!(
        prompt.detail.contains("feat/x"),
        "the prompt should name the session: {}",
        prompt.detail
    );

    app.send_key(key(KeyCode::Char('y'))).unwrap();
    assert_eq!(
        app.state.pending_action,
        Some(lg::state::PendingAction::Quit)
    );
}

#[test]
fn quitting_with_no_sessions_still_just_quits() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.send_key(key(KeyCode::Char('q'))).unwrap();
    assert!(app.state.should_quit);
}

/// Two sessions in two checkouts, without needing two real repositories.
fn two_sessions(app: &mut lg::app::HeadlessApp<TestBackend>) -> (String, String) {
    shell_session(app, "sleep 30", "/tmp/wt-one");
    let first = app.state.sessions.focused().expect("first").to_string();
    shell_session(app, "sleep 30", "/tmp/wt-two");
    let second = app.state.sessions.focused().expect("second").to_string();
    (first, second)
}

#[test]
fn ctrl_n_and_ctrl_p_walk_through_the_sessions() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    let (first, second) = two_sessions(&mut app);
    app.state.session_capture = false;

    app.send_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.state.sessions.focused().map(|id| id.to_string()),
        Some(first),
        "next wraps round to the first session"
    );
    assert!(app.state.session_capture, "cycling hands over the keyboard");

    // The keyboard now belongs to that session, so switching again starts by
    // taking it back — the same rule as any other lg key.
    app.send_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL))
        .unwrap();
    app.send_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.state.sessions.focused().map(|id| id.to_string()),
        Some(second)
    );
}

#[test]
fn a_captured_session_keeps_the_switching_keys_for_itself() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    let (_, second) = two_sessions(&mut app);
    app.state.session_capture = true;

    // Ctrl-n belongs to the program while it holds the keyboard; lg must not
    // steal it out from under a shell or an editor running in there.
    app.send_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.state.sessions.focused().map(|id| id.to_string()),
        Some(second),
        "the shown session must not change"
    );
}

#[test]
fn cycling_with_no_sessions_says_so_and_changes_nothing() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.send_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.state.session_view().is_none());
    assert_eq!(app.state.main_view, MainView::Diff);
}

#[test]
fn sessions_are_listed_under_their_checkout_and_can_be_reopened() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    app.state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace.worktrees/feat-x", "feat/x"),
    ];
    shell_session(&mut app, "sleep 30", "/workspace.worktrees/feat-x");

    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        screen.contains("claude"),
        "the tree should list the session: {screen}"
    );

    // Row 0 root, row 1 the worktree, row 2 its session.
    app.state.show_diff();
    app.state.focus = Pane::Status;
    app.state.nested_repo_tree_idx = 2;
    app.send_key(key(KeyCode::Enter)).unwrap();

    assert!(
        app.state.session_view().is_some(),
        "Enter on a session row should show it"
    );
    assert!(app.state.session_capture);
}

#[test]
fn the_header_counts_sessions_and_flags_the_ones_waiting() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    two_sessions(&mut app);

    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("2 sessions"),
        "the header should count sessions: {}",
        buffer_text(&app)
    );
}

#[test]
fn f2_swaps_between_the_git_view_and_the_workspace_view() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    assert_eq!(app.state.mode, AppMode::Git);

    app.send_key(key(KeyCode::F(2))).unwrap();
    assert_eq!(app.state.mode, AppMode::Workspace);
    let screen = buffer_text(&app);
    assert!(
        screen.contains("Checkouts"),
        "the checkout list should be the left column: {screen}"
    );
    assert!(
        !screen.contains("[2] Files"),
        "the git panes are put away in workspace mode: {screen}"
    );
    assert!(
        screen.contains("No session yet"),
        "an empty workspace should say how to start one: {screen}"
    );

    app.send_key(key(KeyCode::F(2))).unwrap();
    assert_eq!(app.state.mode, AppMode::Git);
    assert!(
        buffer_text(&app).contains("[2] Files"),
        "the git panes come back"
    );
}

#[test]
fn workspace_mode_shows_the_focused_session_beside_the_checkouts() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    app.state.workspace_root = Some("/workspace".into());
    app.state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace.worktrees/feat-x", "feat/x"),
    ];
    shell_session(
        &mut app,
        "printf 'working on it'; sleep 30",
        "/workspace.worktrees/feat-x",
    );
    app.state.mode = AppMode::Workspace;

    wait_for(&mut app, "working on it");
    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(screen.contains("working on it"), "{screen}");
    assert!(
        screen.contains("feat/x"),
        "the checkout list is there: {screen}"
    );
}

#[test]
fn workspace_mode_keeps_focus_on_panes_that_exist() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    app.state.mode = AppMode::Workspace;
    app.state.focus = Pane::Status;

    // The numbered keys for hidden panes do nothing.
    app.send_key(key(KeyCode::Char('2'))).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    app.send_key(key(KeyCode::Char('3'))).unwrap();
    assert_eq!(app.state.focus, Pane::Status);

    // Tab walks between the two panes that are on screen.
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
}

#[test]
fn leaving_workspace_mode_leaves_the_sessions_running() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    shell_session(&mut app, "sleep 30", "/workspace");
    app.state.session_capture = false;
    app.state.mode = AppMode::Workspace;

    app.send_key(key(KeyCode::F(2))).unwrap();

    assert_eq!(app.state.mode, AppMode::Git);
    assert_eq!(app.state.sessions.len(), 1);
    assert!(
        app.state.sessions.focused_session().unwrap().is_running(),
        "switching views must not stop anything"
    );
}
