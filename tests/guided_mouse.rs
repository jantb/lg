//! The mouse in a guided review: the wheel over the step list moves between
//! steps, over the pane beside it scrolls the commentary, and a click on a
//! step goes to it. None of it marks a step reviewed.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use lg::app::HeadlessApp;
use lg::state::Modal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// The screen row whose first `width` columns contain `needle`.
fn row_with(app: &HeadlessApp<TestBackend>, needle: &str, width: u16) -> Option<u16> {
    let buf = app.terminal.backend().buffer();
    (0..buf.area.height).find(|&row| {
        let text: String = (0..width.min(buf.area.width))
            .map(|col| buf[(col, row)].symbol().to_string())
            .collect();
        text.contains(needle)
    })
}

#[test]
fn the_wheel_and_clicks_move_through_a_guided_review() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
    std::fs::write(repo.join("a.txt"), &body).unwrap();
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["checkout", "-b", "feat/x"]);
    std::fs::write(repo.join("a.txt"), body.replace("line 3\n", "line three\n")).unwrap();
    std::fs::write(repo.join("b.txt"), "bee\n").unwrap();
    git(&repo, &["commit", "-am", "change two files"]);
    // SAFETY: this is the only test in the binary, and nothing else runs yet.
    unsafe {
        std::env::set_var("LG_PREFERENCES_DIR", home.join("config"));
        std::env::set_var("LG_SETTINGS_DIR", home.join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", home.join("model"));
    }
    lg::git::set_active_repo(&repo);

    let mut app = HeadlessApp::new(TestBackend::new(140, 40)).unwrap();
    app.state.ai_assist = false;
    app.state.repo_root = Some(repo.to_string_lossy().into_owned());
    app.state.branch = Some("feat/x".into());
    app.send_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.state.modal, Modal::GuidedReview);
    let deadline = Instant::now() + Duration::from_secs(20);
    while app
        .state
        .guided
        .as_ref()
        .is_some_and(|guided| guided.loading.is_some())
    {
        app.render().unwrap();
        assert!(Instant::now() < deadline, "the guided review never loaded");
        std::thread::sleep(Duration::from_millis(20));
    }
    app.render().unwrap();
    let current = |app: &HeadlessApp<TestBackend>| app.state.guided.as_ref().unwrap().current;
    let reviewed =
        |app: &HeadlessApp<TestBackend>| app.state.guided.as_ref().unwrap().reviewed_count();
    assert_eq!(current(&app), 0, "the walk opens on the overview");

    // The wheel over the step list steps through the walk.
    let overview = row_with(&app, "Overview", 40).expect("the overview row");
    app.send_mouse(mouse(MouseEventKind::ScrollDown, 4, overview))
        .unwrap();
    assert_eq!(current(&app), 1);
    app.send_mouse(mouse(MouseEventKind::ScrollDown, 4, overview))
        .unwrap();
    assert_eq!(current(&app), 2);
    app.send_mouse(mouse(MouseEventKind::ScrollUp, 4, overview))
        .unwrap();
    assert_eq!(current(&app), 1);
    assert_eq!(
        reviewed(&app),
        0,
        "scrolling past a step is not reviewing it"
    );

    // The wheel beside the list scrolls the pane there, not the steps.
    let before = app.state.guided.as_ref().unwrap().side_scroll;
    app.send_mouse(mouse(MouseEventKind::ScrollDown, 100, 20))
        .unwrap();
    assert_eq!(current(&app), 1);
    assert!(app.state.guided.as_ref().unwrap().side_scroll > before);

    // A click on a step in the list goes to it.
    app.render().unwrap();
    let b_row = row_with(&app, "b.txt", 40).expect("the second file's row");
    app.send_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 4, b_row + 1))
        .unwrap();
    assert_eq!(current(&app), 2, "the step under b.txt");
    assert_eq!(reviewed(&app), 0);
}
