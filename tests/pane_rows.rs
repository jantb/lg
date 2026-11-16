//! The diff pane, the branch log and the error popup draw exactly the rows
//! they scroll over: what is counted to bound scrolling and place the cursor
//! is what ends up on screen.

use lg::panel;
use lg::state::{AppState, DiffSource, DiffViewMode, Pane};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    text::{Line, Span},
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The main pane drawn `width` by `height` cells, frame included, as text.
fn draw_main(state: &AppState, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| panel::main::render(state, frame.area(), frame, true))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|row| {
            (0..width)
                .map(|col| buffer[(col, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A diff whose context lines are wide characters, each wider on screen than
/// it is long in characters.
fn wide_character_diff() -> String {
    let mut lines = vec![
        "diff --git a/ja.txt b/ja.txt".to_string(),
        "--- a/ja.txt".to_string(),
        "+++ b/ja.txt".to_string(),
        "@@ -1,11 +1,11 @@".to_string(),
    ];
    for _ in 0..10 {
        lines.push(format!(" {}", "日本語のテキスト".repeat(4)));
    }
    lines.push(" LAST LINE".to_string());
    lines.join("\n")
}

#[test]
fn unified_rows_with_wide_characters_fit_the_pane() {
    for row in lg::ui::highlight_diff_text_wrapped(&wide_character_diff(), 40) {
        assert!(
            row.width() <= 40,
            "{:?} is {} wide",
            line_text(&row),
            row.width()
        );
    }
}

#[test]
fn the_side_by_side_separator_stays_put_beside_wide_characters() {
    let diff = [
        "diff --git a/ja.txt b/ja.txt",
        "--- a/ja.txt",
        "+++ b/ja.txt",
        "@@ -1,2 +1,2 @@",
        "-plain ascii line",
        "-完了しました ✅ 🚀",
        "+plain ascii line too",
        "+日本語のテキストをここに",
    ]
    .join("\n");
    let rows = lg::ui::highlight_side_by_side_diff_text(&diff, 60);
    let columns: Vec<usize> = rows
        .iter()
        .filter_map(|row| {
            let text = line_text(row);
            let at = text.find(" | ")?;
            Some(Span::raw(&text[..at]).width())
        })
        .collect();
    assert!(columns.len() >= 2, "{columns:?}");
    assert!(columns.iter().all(|c| *c == columns[0]), "{columns:?}");
    for row in rows.iter().filter(|row| line_text(row).contains(" | ")) {
        assert_eq!(row.width(), 60, "{:?}", line_text(row));
    }
}

#[test]
fn the_end_of_a_diff_with_wide_characters_can_be_scrolled_into_view() {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = DiffSource::File("ja.txt".into());
    state.diff_view_mode = DiffViewMode::Unified;
    state.set_diff_text(wide_character_diff());
    state.diff_viewport_width = 40;
    state.diff_viewport_height = 8;

    panel::main::handle_key(&mut state, key(KeyCode::Char('G'))).unwrap();

    let screen = draw_main(&state, 42, 10);
    assert!(screen.contains("LAST LINE"), "{screen}");
}

/// What the diff pane shows for a file with a staged and an unstaged hunk.
const SECTIONED_DIFF: &str = r#"== staged (--cached) ==
diff --git a/notes.txt b/notes.txt
index 1111111..2222222 100644
--- a/notes.txt
+++ b/notes.txt
@@ -1,3 +1,3 @@
-line 1
+line one
 line 2
 line 3

== worktree ==
diff --git a/notes.txt b/notes.txt
index 2222222..3333333 100644
--- a/notes.txt
+++ b/notes.txt
@@ -20,3 +20,3 @@ fn twenty()
 line 19
-line 20
+line twenty
 line 21
"#;

/// A row that shows line numbers and nothing else.
fn numbered_but_empty(row: &str) -> bool {
    row.chars().any(|ch| ch.is_ascii_digit())
        && row
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == ' ' || ch == '|')
}

#[test]
fn a_blank_line_outside_a_hunk_gets_no_line_numbers() {
    let unified = lg::ui::highlight_diff_text_wrapped(SECTIONED_DIFF, 80);
    let side_by_side = lg::ui::highlight_side_by_side_diff_text(SECTIONED_DIFF, 80);
    for row in unified.iter().chain(&side_by_side).map(line_text) {
        assert!(!numbered_but_empty(&row), "{row:?}");
    }
}

#[test]
fn the_last_row_of_a_scrolled_to_the_end_diff_is_its_last_line() {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = DiffSource::File("notes.txt".into());
    state.diff_view_mode = DiffViewMode::Unified;
    state.set_diff_text(SECTIONED_DIFF.to_string());
    state.diff_viewport_width = 60;
    state.diff_viewport_height = 5;

    panel::main::handle_key(&mut state, key(KeyCode::Char('G'))).unwrap();

    let screen = draw_main(&state, 62, 7);
    let last_row = screen.lines().nth(5).unwrap_or_default();
    assert!(last_row.contains("line 21"), "{screen}");
}

/// A branch log of ten commits whose message lines break at a space well
/// before the pane's edge, so each takes more rows than its length alone says.
fn word_wrapping_log() -> String {
    let body = format!("| {} {} {}", "a".repeat(15), "b".repeat(15), "c".repeat(6));
    let mut log = String::new();
    for at in 0..10 {
        log.push_str(&format!("* commit {at}abcdef\n{body}\n"));
    }
    log.push_str("  THE END\n");
    log
}

fn log_state() -> AppState {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.branch = Some("main".into());
    state.diff_source = DiffSource::Branch("feature".into());
    state.set_diff_text(word_wrapping_log());
    state.diff_viewport_width = 20;
    state.diff_viewport_height = 6;
    state
}

#[test]
fn the_end_of_a_word_wrapped_branch_log_can_be_scrolled_into_view() {
    let mut state = log_state();

    panel::main::handle_key(&mut state, key(KeyCode::Char('G'))).unwrap();

    let screen = draw_main(&state, 22, 8);
    assert!(screen.contains("THE END"), "{screen}");
}

#[test]
fn the_log_cursor_stays_on_screen_on_the_commit_it_names() {
    let mut state = log_state();

    for _ in 0..9 {
        panel::main::handle_key(&mut state, key(KeyCode::Char(']'))).unwrap();
    }

    let mut terminal = Terminal::new(TestBackend::new(22, 8)).unwrap();
    terminal
        .draw(|frame| panel::main::render(&state, frame.area(), frame, true))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let selection_bg = lg::ui::palette::selection().bg.unwrap();
    let marked: Vec<String> = (0..8)
        .filter(|row| (1..21).any(|col| buffer[(col, *row)].bg == selection_bg))
        .map(|row| (1..21).map(|col| buffer[(col, row)].symbol()).collect())
        .collect();
    assert!(
        marked.iter().any(|row| row.contains("9abcdef")),
        "{marked:?}"
    );
}

fn opened_file(state: &AppState) -> Option<String> {
    match &state.pending_action {
        Some(lg::state::PendingAction::OpenFile(path)) => Some(path.clone()),
        _ => None,
    }
}

fn commit_state(diff: String, mode: DiffViewMode) -> AppState {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = DiffSource::Commit("abc1234".into());
    state.diff_view_mode = mode;
    state.set_diff_text(diff);
    state.diff_viewport_width = 40;
    state.diff_viewport_height = 6;
    state
}

fn file_header(path: &str) -> [String; 3] {
    [
        format!("diff --git a/{path} b/{path}"),
        format!("--- a/{path}"),
        format!("+++ b/{path}"),
    ]
}

#[test]
fn o_opens_the_file_on_screen_when_its_lines_wrap() {
    let mut diff: Vec<String> = file_header("src/a.rs").into();
    diff.push("@@ -1,6 +1,6 @@".into());
    for at in 0..6 {
        diff.push(format!(" let wrapped_{at} = \"{}\";", "x".repeat(60)));
    }
    diff.extend(file_header("src/b.rs"));
    diff.push("@@ -1,1 +1,1 @@".into());
    diff.push("-old".into());
    diff.push("+new".into());
    let mut state = commit_state(diff.join("\n"), DiffViewMode::Unified);

    for _ in 0..12 {
        panel::main::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    }
    let screen = draw_main(&state, 42, 8);
    assert!(
        !screen.contains("src/b.rs"),
        "a.rs alone is on screen: {screen}"
    );
    panel::main::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(opened_file(&state).as_deref(), Some("src/a.rs"));
}

#[test]
fn o_opens_the_file_on_screen_in_the_side_by_side_view() {
    let mut diff: Vec<String> = file_header("src/a.rs").into();
    diff.push("@@ -1,10 +1,10 @@".into());
    diff.extend((0..10).map(|at| format!("-old {at}")));
    diff.extend((0..10).map(|at| format!("+new {at}")));
    diff.extend(file_header("src/b.rs"));
    diff.push("@@ -1,6 +1,6 @@".into());
    diff.extend((0..6).map(|at| format!(" kept {at}")));
    let mut state = commit_state(diff.join("\n"), DiffViewMode::SideBySide);

    panel::main::handle_key(&mut state, key(KeyCode::Char('G'))).unwrap();
    let screen = draw_main(&state, 42, 8);
    assert!(
        !screen.contains("src/a.rs"),
        "b.rs alone is on screen: {screen}"
    );
    panel::main::handle_key(&mut state, key(KeyCode::Char('o'))).unwrap();

    assert_eq!(opened_file(&state).as_deref(), Some("src/b.rs"));
}
