use lg::{
    git::FileEntry,
    panel,
    state::{AppState, Modal, Pane, TreeKind, build_tree_rows},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    style::Color,
};
use std::collections::HashSet;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn make_state_with_files() -> AppState {
    let mut s = AppState::new();
    s.files = vec![
        FileEntry {
            path: "a.rs".into(),
            x: ' ',
            y: 'M',
        },
        FileEntry {
            path: "b.rs".into(),
            x: 'A',
            y: ' ',
        },
        FileEntry {
            path: "c.rs".into(),
            x: '?',
            y: '?',
        },
    ];
    s
}

// ── Navigation ────────────────────────────────────────────────────────────────

#[test]
fn files_panel_navigation_moves_selection() {
    let mut state = make_state_with_files();
    assert_eq!(state.files_idx, 0);
    panel::files::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(state.files_idx, 1);
}

#[test]
fn files_panel_k_moves_selection_up() {
    let mut state = make_state_with_files();
    state.files_idx = 2;
    panel::files::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();
    assert_eq!(state.files_idx, 1);
}

// ── Focus cycling ─────────────────────────────────────────────────────────────

#[test]
fn focus_cycles_through_all_panes_with_tab() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    assert_eq!(app.state.focus, Pane::Files);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Branches);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Commits);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    app.send_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.state.focus, Pane::Files);
}

#[test]
fn numeric_keys_set_focus() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.send_key(key(KeyCode::Char('1'))).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    app.send_key(key(KeyCode::Char('3'))).unwrap();
    assert_eq!(app.state.focus, Pane::Branches);
    app.send_key(key(KeyCode::Char('0'))).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
}

#[test]
fn pressing_f_opens_flow_modal_with_deploy_map() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.send_key(key(KeyCode::Char('F'))).unwrap();
    assert_eq!(app.state.modal, Modal::Flow);

    let buf = app.terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(
        text.contains("production"),
        "missing production path: {text}"
    );
    assert!(text.contains("develop"), "missing develop branch: {text}");
    assert!(text.contains("dev"), "missing dev deployment: {text}");
    assert!(
        text.contains("release/next"),
        "missing release/next branch: {text}"
    );
    assert!(text.contains("test"), "missing test deployment: {text}");
}

// ── Panel transitions ─────────────────────────────────────────────────────────

#[test]
fn pressing_c_opens_commit_modal() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
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
    state.prev_focus = Pane::Files;
    state.modal = Modal::Help;

    panel::help::handle_key(&mut state, key(KeyCode::Char('?'))).unwrap();

    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.focus, Pane::Files);
}

// ── Commit input ──────────────────────────────────────────────────────────────

#[test]
fn backspace_removes_last_char_of_commit_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "hello".into();

    panel::commit::handle_key(&mut state, key(KeyCode::Backspace)).unwrap();

    assert_eq!(state.commit_message, "hell");
}

#[test]
fn commit_input_accepts_long_multiline_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "x".repeat(2_048);

    panel::commit::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();

    assert_eq!(state.commit_message, format!("{}\ny", "x".repeat(2_048)));
}

// ── Diff scroll clamping ──────────────────────────────────────────────────────

#[test]
fn diff_scroll_g_clamps_to_max_offset() {
    let mut state = AppState::new();
    // Seed 500 lines of content and set viewport metrics manually.
    state.diff_text = "line\n".repeat(500);
    state.diff_line_count = 500;
    state.diff_viewport_height = 10;

    // G should jump to max_offset = 500 - 10 = 490.
    panel::main::handle_key(&mut state, key(KeyCode::Char('G'))).unwrap();
    assert_eq!(state.diff_offset, 490, "G should set offset to max_offset");

    // j past the end must stay at 490.
    panel::main::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(
        state.diff_offset, 490,
        "j past end should stay at max_offset"
    );
    panel::main::handle_key(&mut state, key(KeyCode::Down)).unwrap();
    assert_eq!(
        state.diff_offset, 490,
        "Down past end should stay at max_offset"
    );
}

#[test]
fn diff_scroll_g_renders_non_blank_bottom_of_content() {
    // End-to-end: press `0`, seed a long diff, press `G`, render, then scan
    // the main-pane rect in the terminal buffer. Before the fix, `G` set the
    // offset to u16::MAX/2 and the pane rendered empty. This test would have
    // caught that.
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();

    // Focus the diff pane and render once so diff_viewport_height is set.
    app.send_key(key(KeyCode::Char('0'))).unwrap();
    app.render().unwrap();

    // Seed 200 numbered lines as the diff content.
    let body: String = (0..200).map(|i| format!("line {i:03}\n")).collect();
    app.state.diff_text = body;
    app.state.diff_line_count = 200;
    // viewport_height is already set from the previous render.

    // Press G — should jump to the end without blanking.
    app.send_key(key(KeyCode::Char('G'))).unwrap();

    let vh = app.state.diff_viewport_height;
    assert!(vh > 0, "viewport height should be set by render");
    assert_eq!(
        app.state.diff_offset,
        200 - vh,
        "G should land at line_count - viewport_height"
    );

    // Scan the main-pane rect in the buffer.
    let buf = app.terminal.backend().buffer().clone();
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: buf.area.width,
        height: buf.area.height,
    };
    let main = lg::ui::split_layout(area).main;

    // Collect the text inside the pane's inner area (skip the 1-cell border).
    let mut pane_text = String::new();
    for row in (main.y + 1)..(main.y + main.height - 1) {
        for col in (main.x + 1)..(main.x + main.width - 1) {
            pane_text.push_str(buf[(col, row)].symbol());
        }
        pane_text.push('\n');
    }

    // Sanity: the pane is not entirely whitespace.
    let non_space_cells = pane_text.chars().filter(|c| !c.is_whitespace()).count();
    assert!(
        non_space_cells > 0,
        "diff pane should not be blank after pressing G; got:\n{pane_text}"
    );

    // The last visible line in the buffer should be "line 199" (the final line
    // of content). If the offset were past the end, we'd see blanks instead.
    assert!(
        pane_text.contains("line 199"),
        "expected final line 'line 199' to be visible at bottom; got:\n{pane_text}"
    );

    // And pressing `j` further must not blank the pane.
    app.send_key(key(KeyCode::Char('j'))).unwrap();
    assert_eq!(
        app.state.diff_offset,
        200 - vh,
        "j past end should stay clamped"
    );

    let buf2 = app.terminal.backend().buffer().clone();
    let mut still_has_content = false;
    for row in (main.y + 1)..(main.y + main.height - 1) {
        for col in (main.x + 1)..(main.x + main.width - 1) {
            if !buf2[(col, row)].symbol().chars().all(char::is_whitespace) {
                still_has_content = true;
                break;
            }
        }
    }
    assert!(still_has_content, "pane blanked after j past end");
}

// ── XY-rendering smoke test ───────────────────────────────────────────────────

#[test]
fn files_panel_renders_xy_codes_with_color() {
    let state = make_state_with_files();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::files::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();

    // Find the 'M' cell (y-side modification for a.rs) and check it's yellow.
    let found_yellow_m = (0..buf.area.height).any(|row| {
        (0..buf.area.width).any(|col| {
            let cell = &buf[(col, row)];
            cell.symbol() == "M" && cell.style().fg == Some(Color::Yellow)
        })
    });
    assert!(
        found_yellow_m,
        "expected a yellow 'M' cell for modified file"
    );

    // Find the '?' cell (untracked) — should be cyan.
    let found_cyan_q = (0..buf.area.height).any(|row| {
        (0..buf.area.width).any(|col| {
            let cell = &buf[(col, row)];
            cell.symbol() == "?" && cell.style().fg == Some(Color::Cyan)
        })
    });
    assert!(found_cyan_q, "expected a cyan '?' cell for untracked file");
}

#[test]
fn status_panel_renders_change_counts() {
    let state = make_state_with_files();
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::status::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("3 files"), "missing total file count: {text}");
    assert!(text.contains("S1"), "missing staged count: {text}");
    assert!(text.contains("U1"), "missing unstaged count: {text}");
    assert!(text.contains("?1"), "missing untracked count: {text}");
}

#[test]
fn status_panel_shows_active_generation() {
    let mut state = make_state_with_files();
    let (_tx, rx) = std::sync::mpsc::channel::<lg::state::GenMsg>();
    state.generation = Some(lg::state::Generation {
        rx,
        output: String::new(),
        spinner: 0,
    });

    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::status::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(
        text.contains("generating"),
        "expected active generation cue: {text}"
    );
}

// ── Tree building ─────────────────────────────────────────────────────────────

#[test]
fn tree_flat_files_emit_all_changes_plus_each_file() {
    let files = vec![
        FileEntry {
            path: "a.rs".into(),
            x: ' ',
            y: 'M',
        },
        FileEntry {
            path: "b.rs".into(),
            x: 'A',
            y: ' ',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());
    assert_eq!(rows.len(), 3);
    assert!(matches!(rows[0].kind, TreeKind::AllChanges));
    assert!(matches!(rows[1].kind, TreeKind::File { entry_idx: 0 }));
    assert!(matches!(rows[2].kind, TreeKind::File { entry_idx: 1 }));
}

#[test]
fn tree_groups_nested_files_under_folders() {
    let files = vec![
        FileEntry {
            path: "src/lib.rs".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/util/mod.rs".into(),
            x: 'A',
            y: ' ',
        },
        FileEntry {
            path: "README.md".into(),
            x: ' ',
            y: 'M',
        },
    ];
    let rows = build_tree_rows(&files, &HashSet::new());
    // Interleaved alphabetical at each depth:
    //   root: README.md ('r') < src/ ('s')
    //   src/: lib.rs ('l') < util/ ('u')
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[1].path, "README.md");
    assert_eq!(rows[2].path, "src");
    match rows[2].kind {
        TreeKind::Folder {
            expanded,
            total,
            staged,
        } => {
            assert!(expanded);
            assert_eq!(total, 2);
            assert_eq!(staged, 2);
        }
        _ => panic!("expected folder row"),
    }
    assert_eq!(rows[3].path, "src/lib.rs");
    assert_eq!(rows[4].path, "src/util");
    assert_eq!(rows[5].path, "src/util/mod.rs");
}

#[test]
fn tree_collapsed_folder_hides_children() {
    let files = vec![
        FileEntry {
            path: "src/a.rs".into(),
            x: 'M',
            y: ' ',
        },
        FileEntry {
            path: "src/b.rs".into(),
            x: 'A',
            y: ' ',
        },
    ];
    let mut collapsed = HashSet::new();
    collapsed.insert("src".to_string());
    let rows = build_tree_rows(&files, &collapsed);
    assert_eq!(rows.len(), 2); // AllChanges + collapsed src folder only
    match rows[1].kind {
        TreeKind::Folder {
            expanded,
            total,
            staged,
        } => {
            assert!(!expanded);
            assert_eq!(total, 2);
            assert_eq!(staged, 2);
        }
        _ => panic!("expected folder row"),
    }
}

#[test]
fn files_panel_enter_toggles_folder_collapse() {
    use ratatui::crossterm::event::KeyCode;
    let mut state = AppState::new();
    state.files = vec![FileEntry {
        path: "src/lib.rs".into(),
        x: 'M',
        y: ' ',
    }];
    // Initial rows: AllChanges, src/, src/lib.rs. Move cursor to folder row.
    state.files_idx = 1;
    panel::files::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(
        state.collapsed_dirs.contains("src"),
        "Enter on expanded folder should collapse it"
    );
    // Now only AllChanges + collapsed src remain
    assert_eq!(state.tree_rows().len(), 2);

    // Enter again re-expands.
    panel::files::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(!state.collapsed_dirs.contains("src"));
    assert_eq!(state.tree_rows().len(), 3);
}

// ── Push animation ────────────────────────────────────────────────────────────

#[test]
fn push_modal_renders_spinner_when_job_running() {
    use lg::state::{PushJob, PushMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut state = AppState::new();
    state.modal = Modal::Push;
    state.branch = Some("feature/x".into());
    let (_tx, rx) = std::sync::mpsc::channel::<PushMsg>();
    state.push_job = Some(PushJob {
        rx,
        spinner: 0,
        branch: "feature/x".into(),
        remote: "origin".into(),
    });

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::push::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }
    assert!(text.contains("Pushing"), "expected 'Pushing' label: {text}");
    assert!(
        text.contains("feature/x"),
        "expected branch name in modal: {text}"
    );
}

#[test]
fn push_modal_handle_key_is_noop_while_running() {
    use lg::state::{PushJob, PushMsg};
    use ratatui::crossterm::event::KeyCode;
    let mut state = AppState::new();
    state.modal = Modal::Push;
    let (_tx, rx) = std::sync::mpsc::channel::<PushMsg>();
    state.push_job = Some(PushJob {
        rx,
        spinner: 0,
        branch: "main".into(),
        remote: "origin".into(),
    });

    panel::push::handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    // Esc must not close the modal while push is running.
    assert_eq!(state.modal, Modal::Push);
    panel::push::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(state.pending_action.is_none());
}

#[test]
fn layout_renders_all_panel_borders() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state = make_state_with_files();
    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(all_text.contains("Status"), "missing Status panel title");
    assert!(all_text.contains("Files"), "missing Files panel title");
    assert!(
        all_text.contains("Branches"),
        "missing Branches panel title"
    );
    assert!(all_text.contains("Commits"), "missing Commits panel title");
}
