//! What the merge view draws after a change is the change: an edit, an undo
//! and a taken side each show in the result pane on the very next frame.

use std::{fs, path::Path, process::Command};

use lg::{
    app::HeadlessApp,
    panel::conflict,
    state::{MergeAction, Modal},
};
use ratatui::{
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

const FILE: &str = "redraw.txt";

fn ok(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Merge Test")
        .env("GIT_AUTHOR_EMAIL", "merge@example.com")
        .env("GIT_COMMITTER_NAME", "Merge Test")
        .env("GIT_COMMITTER_EMAIL", "merge@example.com")
        .output()
        .unwrap();
    assert!(
        output.status.success() || args[0] == "merge",
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A file with two conflicts, so settling one leaves the editor open.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let write = |first: &str, last: &str| {
        fs::write(
            root.join(FILE),
            format!("header\n{first} first\nshared\n{last} last\nfooter\n"),
        )
        .unwrap()
    };
    ok(root, &["init", "-b", "main"]);
    ok(root, &["config", "merge.conflictStyle", "merge"]);
    write("base", "base");
    ok(root, &["add", "--", FILE]);
    ok(root, &["commit", "-m", "base"]);
    ok(root, &["checkout", "-b", "incoming"]);
    write("incoming", "incoming");
    ok(root, &["commit", "-am", "incoming"]);
    ok(root, &["checkout", "main"]);
    write("local", "local");
    ok(root, &["commit", "-am", "local"]);
    ok(root, &["merge", "incoming"]);
    dir
}

fn open(dir: &Path) -> HeadlessApp<TestBackend> {
    let mut app = HeadlessApp::new(TestBackend::new(160, 42)).unwrap();
    app.state.repo_root = Some(dir.to_string_lossy().into_owned());
    app.state.set_conflicts(vec![FILE.into()]);
    app.state.modal = Modal::Conflict;
    app.render().unwrap();
    app
}

/// The text drawn inside the result pane, row by row.
fn result_pane(app: &HeadlessApp<TestBackend>) -> String {
    let buffer = app.terminal.backend().buffer();
    let row = |y: u16| -> Vec<&str> {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    };
    let column_of = |label: &str| {
        (0..buffer.area.height).find_map(|y| {
            let cells = row(y);
            (0..cells.len()).find_map(|x| {
                cells[x..]
                    .concat()
                    .starts_with(label)
                    .then_some((x as u16, y))
            })
        })
    };
    let (left, top) = column_of("Result ·").expect("the result pane is drawn");
    let (right, _) = column_of("Theirs ·").expect("the theirs pane is drawn");
    (top + 1..buffer.area.height)
        .map(|y| row(y)[left as usize..right.saturating_sub(1) as usize].concat())
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn an_edit_shows_in_the_result_on_the_next_frame() {
    let dir = fixture();
    let mut app = open(dir.path());
    app.send_key(key(KeyCode::Enter)).unwrap();
    app.render().unwrap();
    assert!(!result_pane(&app).contains("typed here"));

    assert!(conflict::handle_paste(&mut app.state, "typed here\n"));
    app.render().unwrap();

    assert!(
        result_pane(&app).contains("typed here"),
        "{}",
        result_pane(&app)
    );
}

#[test]
fn an_undo_shows_in_the_result_on_the_next_frame() {
    let dir = fixture();
    let mut app = open(dir.path());
    app.send_key(key(KeyCode::Enter)).unwrap();
    assert!(conflict::handle_paste(&mut app.state, "typed here\n"));
    app.render().unwrap();
    assert!(result_pane(&app).contains("typed here"));

    app.send_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    app.render().unwrap();

    assert!(
        !result_pane(&app).contains("typed here"),
        "{}",
        result_pane(&app)
    );
}

#[test]
fn a_taken_side_shows_in_the_result_on_the_next_frame() {
    let dir = fixture();
    let mut app = open(dir.path());
    assert!(!result_pane(&app).contains("incoming first"));

    app.send_key(key(KeyCode::Char('2'))).unwrap();
    app.render().unwrap();

    assert!(
        result_pane(&app).contains("incoming first"),
        "{}",
        result_pane(&app)
    );
}

#[test]
fn a_resolution_applied_from_outside_the_keyboard_shows_on_the_next_frame() {
    let dir = fixture();
    let mut app = open(dir.path());
    assert!(!result_pane(&app).contains("local first"));

    let editor = app
        .state
        .conflict
        .preview
        .as_mut()
        .and_then(|preview| preview.editor.as_mut().ok())
        .expect("the editor opened");
    editor.apply(0, MergeAction::AcceptOurs);
    app.render().unwrap();

    assert!(
        result_pane(&app).contains("local first"),
        "{}",
        result_pane(&app)
    );
}
