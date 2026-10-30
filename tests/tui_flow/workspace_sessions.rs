//! Sessions across the workspace: listed under their checkouts, the header
//! that counts them, the workspace view, and worktrees they keep busy.

use super::common::*;
use super::sessions::{session_of, shell_session, two_sessions, wait_for};
use lg::panel::environments::menu::{self, RepoAction};
use lg::session::SessionKind;
use lg::state::{AppMode, MainView};
use std::time::{Duration, Instant};

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

/// Closing the session that was resolving a conflict is the end of that work,
/// so the flow waiting on it comes back rather than being left behind a diff.
#[test]
fn closing_a_session_puts_an_unfinished_conflict_back_up() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    app.state.conflict.files = vec!["src/a.rs".into()];
    shell_session(&mut app, "exit 0", "/workspace");
    app.state.session_capture = true;

    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.state.sessions.is_empty() && Instant::now() < deadline {
        app.drain_sessions();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        app.state.sessions.is_empty(),
        "the session should have gone"
    );

    assert_eq!(
        app.state.modal,
        Modal::Conflict,
        "the conflict the flow stopped on should be back"
    );
    let status = app.state.status.as_ref().expect("status");
    assert!(
        status.text.contains("press v"),
        "the status should say how to carry on: {}",
        status.text
    );
}

/// Nothing to come back to means nothing pops up: closing an ordinary session
/// leaves the diff alone.
#[test]
fn closing_a_session_with_no_conflict_opens_no_modal() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    shell_session(&mut app, "exit 0", "/workspace");

    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.state.sessions.is_empty() && Instant::now() < deadline {
        app.drain_sessions();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.state.modal, Modal::None);
}

/// A checkout holds an agent and a shell at once, and both get a row under it —
/// starting a terminal must not be mistaken for the claude already there.
#[test]
fn a_checkout_lists_its_claude_and_its_terminal_side_by_side() {
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
    let dir = "/workspace.worktrees/feat-x";
    let claude = session_of(&mut app, "sleep 30", dir, SessionKind::Claude);
    let terminal = session_of(&mut app, "sleep 30", dir, SessionKind::Terminal);
    assert_ne!(
        claude, terminal,
        "a terminal is a session of its own, not the claude already running there"
    );
    assert_eq!(app.state.sessions.len(), 2);

    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        screen.contains("claude") && screen.contains("terminal"),
        "both sessions should have a row: {screen}"
    );

    // Row 0 root, row 1 the worktree, rows 2 and 3 its two sessions.
    app.state.show_diff();
    app.state.focus = Pane::Status;
    app.state.nested_repo_tree_idx = 3;
    app.send_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.state.session_view().map(|id| id.to_string()),
        Some(terminal.to_string()),
        "the second row under the checkout is the terminal"
    );
}

/// A checkout can hold several shells at once: one running a build, the next
/// reading its log. Each gets its own row, and its own number to be told by.
#[test]
fn a_checkout_holds_several_terminals_at_once() {
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
    let dir = "/workspace.worktrees/feat-x";
    let first = session_of(&mut app, "sleep 30", dir, SessionKind::Terminal);
    let second = session_of(&mut app, "sleep 30", dir, SessionKind::Terminal);
    assert_ne!(first, second, "asking for another terminal must open one");
    assert_eq!(app.state.sessions.len(), 2);

    app.state.show_diff();
    app.state.focus = Pane::Status;
    // Root, the worktree, then a row for each of its terminals.
    app.state.nested_repo_tree_idx = 3;
    app.send_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.state.session_view().map(|id| id.to_string()),
        Some(second.to_string()),
        "each terminal is reachable from its own row"
    );

    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        screen.contains("terminal 2"),
        "the second terminal should be told from the first: {screen}"
    );
}

/// Opening a session moves lg to the checkout it runs in: the diff beside a
/// worktree's terminal should be that worktree's, not the one left behind.
#[test]
fn showing_a_session_moves_the_repository_to_its_checkout() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    let dir = "/workspace.worktrees/feat-x";
    session_of(&mut app, "sleep 30", dir, SessionKind::Terminal);
    assert_eq!(
        app.state.pending_action,
        Some(lg::state::PendingAction::SwitchRepository {
            target: lg::state::RepoTarget::Path(dir.into()),
        }),
        "the session's checkout becomes the one lg is showing"
    );
}

/// A session in the checkout already being shown moves nothing: switching to
/// where lg already is would reload the panes for no reason.
#[test]
fn showing_a_session_in_the_current_checkout_switches_nothing() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    session_of(&mut app, "sleep 30", "/workspace", SessionKind::Terminal);
    assert_eq!(app.state.pending_action, None);
}

/// An agent is still one to a checkout: two claudes editing the same files is
/// a mix-up, so asking again lands on the one already running.
#[test]
fn asking_for_an_agent_twice_finds_the_one_already_running() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    let dir = "/workspace.worktrees/feat-x";
    let first = session_of(&mut app, "sleep 30", dir, SessionKind::Claude);
    let again = session_of(&mut app, "sleep 30", dir, SessionKind::Claude);
    assert_eq!(first, again);
    assert_eq!(app.state.sessions.len(), 1);
}

#[test]
fn the_header_counts_sessions_and_says_what_they_are_doing() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    two_sessions(&mut app);

    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        screen.contains("2 sessions"),
        "the header should count sessions: {screen}"
    );
    // Both are asleep at their prompt. Output alone is not somebody waiting on
    // an answer, and the badge must not say it is.
    assert!(
        !screen.contains("waiting") && !screen.contains("need"),
        "idle sessions must not be reported as blocked: {screen}"
    );
}

/// The badge counts the same states the dots do. A session drawing a question
/// is the one worth interrupting somebody for, so it is what gets counted.
#[test]
fn the_header_calls_out_a_session_blocked_on_a_question() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    shell_session(&mut app, "sleep 30", "/tmp/quiet");
    shell_session(
        &mut app,
        "printf 'Do you want to proceed?\\n\\342\\235\\257 1. Yes\\n  2. No\\n'; sleep 30",
        "/tmp/asking",
    );

    wait_for(&mut app, "1. Yes");
    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        screen.contains("2 sessions") && screen.contains("1 needs input"),
        "the header should name the blocked session: {screen}"
    );
}

#[test]
fn w_swaps_between_the_git_view_and_the_workspace_view() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    assert_eq!(app.state.mode, AppMode::Git);

    app.send_key(key(KeyCode::Char('w'))).unwrap();
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

    app.send_key(key(KeyCode::Char('w'))).unwrap();
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

    app.send_key(key(KeyCode::Char('w'))).unwrap();

    assert_eq!(app.state.mode, AppMode::Git);
    assert_eq!(app.state.sessions.len(), 1);
    assert!(
        app.state.sessions.focused_session().unwrap().is_running(),
        "switching views must not stop anything"
    );
}

#[test]
fn selecting_a_file_puts_the_session_in_the_background() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    app.state.files = vec![
        FileEntry {
            path: "src/a.rs".into(),
            x: ' ',
            y: 'M',
        },
        FileEntry {
            path: "src/b.rs".into(),
            x: ' ',
            y: 'M',
        },
    ];
    shell_session(
        &mut app,
        "printf 'claude is working'; sleep 30",
        "/workspace",
    );
    wait_for(&mut app, "claude is working");

    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("claude is working"),
        "the session starts on screen"
    );

    // Focusing the file pane and moving the selection is a request for a diff.
    app.send_key(key(KeyCode::Char('2'))).unwrap();

    assert_eq!(app.state.main_view, MainView::Diff);
    assert!(
        !app.state.session_capture,
        "the keyboard comes back to lg with the diff"
    );
    let screen = buffer_text(&app);
    assert!(
        !screen.contains("claude is working"),
        "the session is no longer drawn: {screen}"
    );
    assert!(
        app.state.sessions.focused_session().unwrap().is_running(),
        "backgrounding must not stop it"
    );
}

#[test]
fn focusing_the_repository_pane_leaves_the_session_on_screen() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    shell_session(&mut app, "printf 'still here'; sleep 30", "/workspace");
    wait_for(&mut app, "still here");

    // The repository pane is where sessions are picked, so reaching it must not
    // hide the one that is running.
    app.send_key(key(KeyCode::Char('1'))).unwrap();

    assert_eq!(app.state.focus, Pane::Status);
    assert!(matches!(app.state.main_view, MainView::Session(_)));
    assert!(buffer_text(&app).contains("still here"));
}

#[test]
fn a_worktree_running_a_session_is_not_removed_from_under_it() {
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

    // Row 0 root, row 1 the worktree, row 2 its session.
    app.state.show_diff();
    app.state.focus = Pane::Status;
    app.state.nested_repo_tree_idx = 1;

    // Landing, bringing home and removing all end in `git worktree remove`.
    for code in [
        RepoAction::LandWorktree,
        RepoAction::BranchHome,
        RepoAction::RemoveWorktree,
    ] {
        app.state.status = None;
        // Showing the session queued a switch to its checkout; this test is
        // about what the menu action does, so it starts from nothing pending.
        app.state.pending_action = None;
        menu::run(&mut app.state, code);

        assert!(
            app.state.confirm.is_none(),
            "{code:?} must not even offer to remove a checkout in use"
        );
        assert_eq!(app.state.pending_action, None, "{code:?} ran something");
        let status = app.state.status.as_ref().expect("status");
        assert!(
            status.is_error,
            "expected an error for {code:?}: {status:?}"
        );
        assert!(
            status.text.contains("close the sessions"),
            "the status names the blocker for {code:?}: {}",
            status.text
        );
        assert!(
            status.text.contains("Ctrl-]") && status.text.contains(" x"),
            "and the keys that do it for {code:?}: {}",
            status.text
        );
    }
}

#[test]
fn closing_the_session_frees_the_worktree_to_be_landed() {
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

    // x closes the shown session, the way the footer and help say it does,
    // once the prompt for stopping a running agent is answered.
    app.state.session_capture = false;
    app.send_key(key(KeyCode::Char('x'))).unwrap();
    app.send_key(key(KeyCode::Char('y'))).unwrap();

    app.state.show_diff();
    app.state.focus = Pane::Status;
    app.state.nested_repo_tree_idx = 1;
    menu::run(&mut app.state, RepoAction::LandWorktree);

    let confirm = app.state.confirm.as_ref().expect("confirm prompt");
    assert_eq!(
        confirm.action,
        PendingAction::LandWorktree {
            path: "/workspace.worktrees/feat-x".into(),
            branch: "feat/x".into(),
        }
    );
}

#[test]
fn a_finished_session_leaves_the_tree_on_its_own() {
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
    shell_session(&mut app, "exit 0", "/workspace.worktrees/feat-x");

    // Row 0 the root, row 1 the worktree, row 2 its session — until it ends.
    app.state.show_diff();
    app.state.focus = Pane::Status;
    app.state.nested_repo_tree_idx = 2;
    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("claude"),
        "the running session has a row"
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.state.sessions.is_empty() {
        app.drain_sessions();
        assert!(Instant::now() < deadline, "the session never ended");
        std::thread::sleep(Duration::from_millis(10));
    }

    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        !screen.contains("claude"),
        "the row goes with it, with no key to press: {screen}"
    );
    assert_eq!(
        app.state.nested_repo_tree_idx, 1,
        "the selection follows the rows that are left"
    );
}

#[test]
fn x_on_a_row_that_is_not_a_session_says_so() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    app.state.worktrees = vec![Worktree {
        is_main: true,
        ..worktree("/workspace", "main")
    }];
    app.state.focus = Pane::Status;
    app.state.nested_repo_tree_idx = 0;

    app.send_key(key(KeyCode::Char('x'))).unwrap();

    let status = app.state.status.as_ref().expect("status");
    assert!(
        status.text.contains("select a session row"),
        "it says what to select: {}",
        status.text
    );
}
