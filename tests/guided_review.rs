//! A guided review over a real branch: notes are kept across leaving and
//! coming back, and an edit made mid-review shows up in the walk.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use lg::app::HeadlessApp;
use lg::state::Modal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn screen(app: &HeadlessApp<TestBackend>) -> String {
    let buf = app.terminal.backend().buffer();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
        text.push('\n');
    }
    text
}

/// Draw frames until the walk has finished reading the change.
fn wait_loaded(app: &mut HeadlessApp<TestBackend>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.render().expect("render");
        let loading = app
            .state
            .guided
            .as_ref()
            .is_some_and(|guided| guided.loading.is_some());
        if !loading {
            return;
        }
        assert!(Instant::now() < deadline, "the guided review never loaded");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn type_text(app: &mut HeadlessApp<TestBackend>, text: &str) {
    for ch in text.chars() {
        app.send_key(key(KeyCode::Char(ch))).unwrap();
    }
}

#[test]
fn a_guided_review_keeps_its_notes_sees_edits_and_on_main_walks_the_changes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(
        repo.join("calc.rs"),
        "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "add calc"]);
    git(&repo, &["checkout", "-b", "feat/sub"]);
    std::fs::write(
        repo.join("calc.rs"),
        "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn sub(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();
    git(&repo, &["commit", "-am", "add sub"]);
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
    app.state.branch = Some("feat/sub".into());

    app.send_key(key(KeyCode::Char('V'))).unwrap();
    assert_eq!(app.state.modal, Modal::GuidedReview);
    wait_loaded(&mut app);
    assert!(screen(&app).contains("calc.rs"), "{}", screen(&app));

    // Onto the hunk, down to the line that is wrong, and a note on it.
    app.send_key(key(KeyCode::Right)).unwrap();
    for _ in 0..10 {
        app.render().unwrap();
        if screen(&app).contains("fn sub") {
            break;
        }
    }
    let wrong = app
        .state
        .guided
        .as_ref()
        .and_then(|guided| guided.step())
        .and_then(|step| step.lines.iter().rposition(|l| l.text == "+    a + b"))
        .expect("the hunk shows the new body");
    while app.state.guided.as_ref().unwrap().cursor < wrong {
        app.send_key(key(KeyCode::Char('j'))).unwrap();
    }
    app.send_key(key(KeyCode::Char('c'))).unwrap();
    type_text(&mut app, "sub adds instead of subtracting");
    app.send_key(key(KeyCode::Enter)).unwrap();
    app.render().unwrap();
    assert!(
        screen(&app).contains("sub adds instead of subtracting"),
        "the note is shown on its line: {}",
        screen(&app)
    );

    // The fix lands in the file while the review is open, and a reload shows it.
    std::fs::write(
        repo.join("calc.rs"),
        "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n",
    )
    .unwrap();
    app.send_key(key(KeyCode::Char('R'))).unwrap();
    wait_loaded(&mut app);
    assert!(
        screen(&app).contains("a - b"),
        "the walk shows the edited code: {}",
        screen(&app)
    );

    // Leave, drop everything held in memory, and come back: the note is still
    // part of the review.
    app.send_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.state.modal, Modal::None);
    app.state.guided = None;
    app.send_key(key(KeyCode::Char('V'))).unwrap();
    wait_loaded(&mut app);
    app.send_key(key(KeyCode::Char('s'))).unwrap();
    app.render().unwrap();
    assert!(
        screen(&app).contains("sub adds instead of subtracting"),
        "the note survives leaving the review: {}",
        screen(&app)
    );

    // Help is one key away from inside the walk, and comes back to it.
    app.send_key(key(KeyCode::Esc)).unwrap();
    app.send_key(key(KeyCode::Char('?'))).unwrap();
    assert_eq!(app.state.modal, Modal::Help);
    app.render().unwrap();
    assert!(
        screen(&app).contains("Write or edit a note on the cursor line"),
        "help opens on the guided review's keys: {}",
        screen(&app)
    );
    app.send_key(key(KeyCode::Char('q'))).unwrap();
    assert_eq!(app.state.modal, Modal::GuidedReview);

    // On main there is no branch to review, so the walk is over what has not
    // been committed yet.
    app.send_key(key(KeyCode::Esc)).unwrap();
    git(&repo, &["checkout", "-f", "main"]);
    std::fs::write(repo.join("notes.md"), "remember to test overflow\n").unwrap();
    app.state.branch = Some("main".into());
    app.send_key(key(KeyCode::Char('V'))).unwrap();
    wait_loaded(&mut app);
    let text = screen(&app);
    assert!(text.contains("uncommitted changes on main"), "{text}");
    assert!(
        text.contains("notes.md"),
        "the untracked file is in the walk: {text}"
    );
    assert!(
        !text.contains("calc.rs"),
        "committed history is not part of the walk: {text}"
    );
}
