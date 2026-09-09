use std::{fs, path::Path, process::Command};

use lg::{
    app::HeadlessApp,
    git::MergeSnapshot,
    panel::conflict,
    state::{MergeEditor, Modal},
};
use ratatui::{
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};

fn git(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Merge Test")
        .env("GIT_AUTHOR_EMAIL", "merge@example.com")
        .env("GIT_COMMITTER_NAME", "Merge Test")
        .env("GIT_COMMITTER_EMAIL", "merge@example.com")
        .output()
        .unwrap()
}

fn ok(root: &Path, args: &[&str]) {
    let output = git(root, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const FILE: &str = "merge file.txt";

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init", "-b", "main"]);
    ok(root, &["config", "merge.conflictStyle", "merge"]);
    let middle = (0..20)
        .map(|i| format!("shared context {i}\n"))
        .collect::<String>();
    fs::write(
        root.join(FILE),
        format!("header\nbase first\n{middle}base last\nfooter\n"),
    )
    .unwrap();
    ok(root, &["add", "--", FILE]);
    ok(root, &["commit", "-m", "base"]);
    ok(root, &["checkout", "-b", "incoming"]);
    fs::write(
        root.join(FILE),
        format!("header\nincoming first\n{middle}incoming last\nfooter\n"),
    )
    .unwrap();
    ok(root, &["commit", "-am", "incoming"]);
    ok(root, &["checkout", "main"]);
    fs::write(
        root.join(FILE),
        format!("header\nlocal first\n{middle}local last\nfooter\n"),
    )
    .unwrap();
    ok(root, &["commit", "-am", "local"]);
    assert!(!git(root, &["merge", "incoming"]).status.success());
    dir
}

fn editor(root: &Path) -> MergeEditor {
    MergeEditor::new(MergeSnapshot::load(root, FILE).unwrap()).unwrap()
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn find_label(app: &HeadlessApp<TestBackend>, label: &str, occurrence: usize) -> (u16, u16) {
    let buffer = app.terminal.backend().buffer();
    let mut matches = Vec::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let text: String = (x..buffer.area.width)
                .map(|col| buffer[(col, y)].symbol())
                .collect();
            if text.starts_with(label) {
                matches.push((x, y));
            }
        }
    }
    *matches
        .get(occurrence)
        .unwrap_or_else(|| panic!("missing {label}"))
}

fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn screenshot(app: &HeadlessApp<TestBackend>, suffix: &str) {
    if let Ok(path) = std::env::var("LG_MERGE_SCREENSHOT") {
        let buffer = app.terminal.backend().buffer();
        let cells: Vec<_> = buffer.content.iter().map(|cell| serde_json::json!({"text": cell.symbol(), "fg": format!("{:?}", cell.fg), "bg": format!("{:?}", cell.bg)})).collect();
        fs::write(format!("{path}{suffix}"), serde_json::to_vec(&serde_json::json!({"width":buffer.area.width,"height":buffer.area.height,"cells":cells})).unwrap()).unwrap();
    }
}

#[test]
fn mouse_arrows_preview_without_mutation_then_apply_replace_insert_and_save() {
    let dir = fixture();
    let original = fs::read_to_string(dir.path().join(FILE)).unwrap();
    let mut app = HeadlessApp::new(TestBackend::new(160, 42)).unwrap();
    app.state.repo_root = Some(dir.path().to_string_lossy().into_owned());
    app.state.set_conflicts(vec![FILE.into()]);
    app.state.modal = Modal::Conflict;
    app.render().unwrap();
    let theirs = find_label(&app, "Theirs ←", 0);
    app.send_mouse(mouse(MouseEventKind::Moved, theirs))
        .unwrap();
    find_label(&app, "HOVER PREVIEW", 0);
    let editor = app
        .state
        .conflict_preview
        .as_ref()
        .unwrap()
        .editor
        .as_ref()
        .unwrap();
    assert_eq!(editor.current().result, "local first\n");
    assert!(!editor.dirty(), "hover must not accept or edit anything");
    assert_eq!(fs::read_to_string(dir.path().join(FILE)).unwrap(), original);
    screenshot(&app, ".hover");
    app.send_mouse(mouse(MouseEventKind::Moved, (0, 0)))
        .unwrap();
    assert!(
        app.state
            .conflict_preview
            .as_ref()
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .hovered
            .is_none()
    );
    app.send_mouse(mouse(MouseEventKind::Down(MouseButton::Left), theirs))
        .unwrap();
    assert_eq!(
        app.state
            .conflict_preview
            .as_ref()
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .current()
            .result,
        "incoming first\n"
    );
    // Click the result itself, then insert a side at that cursor.
    let result = find_label(&app, "incoming first", 0);
    app.send_mouse(mouse(MouseEventKind::Down(MouseButton::Left), result))
        .unwrap();
    let insert = find_label(&app, "+ Ours", 0);
    app.send_mouse(mouse(MouseEventKind::Moved, insert))
        .unwrap();
    find_label(&app, "Insert ours at the result cursor", 0);
    assert_eq!(
        app.state
            .conflict_preview
            .as_ref()
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .current()
            .result,
        "incoming first\n"
    );
    app.send_mouse(mouse(MouseEventKind::Down(MouseButton::Left), insert))
        .unwrap();
    assert_eq!(
        app.state
            .conflict_preview
            .as_ref()
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .current()
            .result,
        "local first\nincoming first\n"
    );
    let next = find_label(&app, "Next ›", 0);
    app.send_mouse(mouse(MouseEventKind::Down(MouseButton::Left), next))
        .unwrap();
    let ours = find_label(&app, "→ Ours", 0);
    app.send_mouse(mouse(MouseEventKind::Down(MouseButton::Left), ours))
        .unwrap();
    let save = find_label(&app, "[ Save ]", 0);
    app.send_mouse(mouse(MouseEventKind::Down(MouseButton::Left), save))
        .unwrap();
    let result = fs::read_to_string(dir.path().join(FILE)).unwrap();
    assert!(result.contains("local first\nincoming first\n") && result.contains("local last\n"));
    assert!(!lg::git::holds_conflict_marker(&result));
}

#[test]
fn manual_merge_reads_real_stages_and_preserves_already_merged_text() {
    let dir = fixture();
    let original = fs::read_to_string(dir.path().join(FILE)).unwrap();
    let mut editor = editor(dir.path());
    assert_eq!(editor.hunks.len(), 2);
    assert_eq!(
        editor.current().source.base.as_deref(),
        Some("base first\n")
    );
    assert!(!editor.dirty());
    assert!(
        editor
            .save()
            .unwrap_err()
            .to_string()
            .contains("every conflict")
    );
    editor.current_mut().choose('1');
    editor.navigate(true);
    editor.current_mut().choose('2');
    assert_eq!(
        fs::read_to_string(dir.path().join(FILE)).unwrap(),
        original,
        "decisions stay in memory"
    );
    editor.save().unwrap();
    let result = fs::read_to_string(dir.path().join(FILE)).unwrap();
    assert!(result.contains("local first\n"));
    assert!(result.contains("incoming last\n"));
    for i in 0..20 {
        assert!(result.contains(&format!("shared context {i}\n")));
    }
    assert!(result.starts_with("header\n") && result.ends_with("footer\n"));
    assert!(!lg::git::holds_conflict_marker(&result));
    assert!(!editor.dirty());
    assert!(
        !git(dir.path(), &["ls-files", "-u"]).stdout.is_empty(),
        "saving must not stage or commit"
    );
}

#[test]
fn saving_refuses_external_file_or_index_changes_without_losing_the_draft() {
    for change_index in [false, true] {
        let dir = fixture();
        let mut editor = editor(dir.path());
        for hunk in &mut editor.hunks {
            hunk.choose('3');
        }
        if change_index {
            ok(dir.path(), &["add", "--", FILE]);
        } else {
            fs::write(dir.path().join(FILE), "external editor's result\n").unwrap();
        }
        let before = fs::read(dir.path().join(FILE)).unwrap();
        assert!(
            editor
                .save()
                .unwrap_err()
                .to_string()
                .contains("changed outside")
        );
        assert_eq!(fs::read(dir.path().join(FILE)).unwrap(), before);
        assert!(editor.dirty());
        assert!(
            editor
                .current()
                .result
                .contains("local first\nincoming first")
        );
    }
}

#[test]
fn manual_text_editing_and_undo_preserve_unicode_and_line_endings() {
    let dir = fixture();
    let mut editor = editor(dir.path());
    let hunk = editor.current_mut();
    hunk.insert("λ😀\r\n");
    hunk.edit(key(KeyCode::Backspace));
    assert!(hunk.result.starts_with("λ😀local"));
    hunk.edit(key(KeyCode::Backspace));
    assert!(hunk.result.starts_with("λlocal"));
    hunk.undo();
    assert!(hunk.result.starts_with("λ😀local"));
    hunk.undo();
    assert!(hunk.result.starts_with("λ😀\r\nlocal"));
    hunk.undo();
    assert!(!hunk.accepted);
    assert_eq!(hunk.result, hunk.source.ours);
}

#[test]
fn inline_editor_routes_typing_paste_save_and_navigation_without_triggering_merge_actions() {
    let dir = fixture();
    let mut app = HeadlessApp::new(TestBackend::new(160, 42)).unwrap();
    app.state.repo_root = Some(dir.path().to_string_lossy().into_owned());
    app.state.set_conflicts(vec![FILE.into()]);
    app.state.modal = Modal::Conflict;
    app.render().unwrap();
    app.send_key(key(KeyCode::Enter)).unwrap();
    assert!(conflict::handle_paste(
        &mut app.state,
        "// custom resolution\n"
    ));
    // These letters are text in the result, never abort/agent/validate commands.
    for c in "acvl".chars() {
        app.send_key(key(KeyCode::Char(c))).unwrap();
    }
    assert!(app.state.pending_action.is_none());
    app.send_key(key(KeyCode::Esc)).unwrap();
    app.send_key(key(KeyCode::Char('j'))).unwrap();
    assert!(app.state.status.as_ref().unwrap().text.contains("unsaved"));
    app.send_key(key(KeyCode::Char(']'))).unwrap();
    app.send_key(key(KeyCode::Char('2'))).unwrap();
    app.send_key(ctrl('s')).unwrap();
    assert!(app.state.conflict_resolved.contains(FILE));
    let text = fs::read_to_string(dir.path().join(FILE)).unwrap();
    assert!(text.contains("// custom resolution\nacvllocal first"));
    assert!(text.contains("incoming last"));
}

#[test]
fn three_way_view_renders_all_sides_and_ancestor_at_normal_and_tiny_sizes() {
    let dir = fixture();
    let mut app = HeadlessApp::new(TestBackend::new(160, 42)).unwrap();
    app.state.repo_root = Some(dir.path().to_string_lossy().into_owned());
    app.state.set_conflicts(vec![FILE.into()]);
    app.state.modal = Modal::Conflict;
    app.render().unwrap();
    let text = |app: &HeadlessApp<TestBackend>| {
        app.terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    };
    let rendered = text(&app);
    for expected in [
        "Ours",
        "Theirs",
        "Result",
        "local first",
        "incoming first",
        "Conflict 1/2",
        "Ctrl-s save",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
    screenshot(&app, "");
    app.send_key(key(KeyCode::Char('b'))).unwrap();
    assert!(text(&app).contains("base first"));
    app.send_key(key(KeyCode::Char('b'))).unwrap();
    for (w, h) in [(100, 30), (80, 24), (40, 12), (12, 4), (1, 1)] {
        app.terminal.backend_mut().resize(w, h);
        app.terminal
            .resize(ratatui::layout::Rect::new(0, 0, w, h))
            .unwrap();
        app.render().unwrap();
        screenshot(&app, &format!(".{w}x{h}"));
        app.send_key(key(KeyCode::Enter)).unwrap();
        app.send_key(key(KeyCode::End)).unwrap();
        app.send_key(key(KeyCode::Esc)).unwrap();
    }
}

#[test]
fn malformed_markers_and_binary_files_use_the_external_editor_fallback() {
    let dir = fixture();
    fs::write(dir.path().join(FILE), "<<<<<<< HEAD\nbroken\n").unwrap();
    assert!(MergeEditor::new(MergeSnapshot::load(dir.path(), FILE).unwrap()).is_err());
    fs::write(dir.path().join(FILE), b"binary\0data").unwrap();
    assert!(
        MergeSnapshot::load(dir.path(), FILE)
            .unwrap_err()
            .to_string()
            .contains("binary")
    );
}

#[test]
fn saving_preserves_crlf_context_and_rejects_markers_in_manual_results() {
    let dir = fixture();
    fs::write(dir.path().join(FILE), "before\r\n<<<<<<< HEAD\r\nours\r\n=======\r\ntheirs\r\n>>>>>>> incoming\r\nafter without newline").unwrap();
    let mut editor = editor(dir.path());
    editor.current_mut().choose('2');
    editor.current_mut().insert("<<<<<<< unresolved\r\n");
    assert!(
        editor
            .save()
            .unwrap_err()
            .to_string()
            .contains("conflict markers")
    );
    editor.current_mut().undo();
    editor.save().unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join(FILE)).unwrap(),
        "before\r\ntheirs\r\nafter without newline"
    );
}

#[test]
fn a_merge_draft_survives_closing_and_prevents_accidental_quit() {
    let dir = fixture();
    let mut app = HeadlessApp::new(TestBackend::new(140, 32)).unwrap();
    app.state.repo_root = Some(dir.path().to_string_lossy().into_owned());
    app.state.set_conflicts(vec![FILE.into()]);
    app.state.modal = Modal::Conflict;
    app.render().unwrap();
    app.send_key(key(KeyCode::Char('1'))).unwrap();
    app.send_key(key(KeyCode::Esc)).unwrap();
    app.state.request_quit();
    assert!(!app.state.should_quit);
    assert_eq!(app.state.modal, Modal::Conflict);
    assert!(
        app.state
            .conflict_preview
            .as_ref()
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .dirty()
    );
    app.send_key(ctrl('r')).unwrap();
    assert!(
        !app.state
            .conflict_preview
            .as_ref()
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .dirty()
    );
}

#[cfg(unix)]
#[test]
fn saving_preserves_executable_mode_and_refuses_a_replaced_symlink() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = fixture();
    let file = dir.path().join(FILE);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    let mut editor = editor(dir.path());
    for hunk in &mut editor.hunks {
        hunk.choose('1');
    }
    editor.save().unwrap();
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let outside = dir.path().join("outside.txt");
    fs::write(&outside, "do not change").unwrap();
    fs::remove_file(&file).unwrap();
    symlink(&outside, &file).unwrap();
    editor.current_mut().insert("new edit");
    assert!(editor.save().is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "do not change");
}
