use super::common::*;

// ── Commit input ──────────────────────────────────────────────────────────────

#[test]
fn backspace_removes_last_char_of_commit_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "hello".into();
    state.commit_cursor = state.commit_message.chars().count();

    panel::commit::handle_key(&mut state, key(KeyCode::Backspace)).unwrap();

    assert_eq!(state.commit_message, "hell");
    assert_eq!(state.commit_cursor, 4);
}

#[test]
fn commit_input_edits_at_cursor_with_arrow_keys() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "helo".into();
    state.commit_cursor = state.commit_message.chars().count();

    panel::commit::handle_key(&mut state, key(KeyCode::Left)).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Char('l'))).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Right)).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Char('!'))).unwrap();

    assert_eq!(state.commit_message, "hello!");
    assert_eq!(state.commit_cursor, 6);
}

#[test]
fn commit_input_moves_cursor_vertically_and_with_control_shortcuts() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "one\ntwo\nthree".into();
    state.commit_cursor = 5;

    panel::commit::handle_key(&mut state, key(KeyCode::Up)).unwrap();
    assert_eq!(state.commit_cursor, 1);

    panel::commit::handle_key(&mut state, key(KeyCode::Down)).unwrap();
    assert_eq!(state.commit_cursor, 5);

    panel::commit::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(state.commit_cursor, 0);

    panel::commit::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(state.commit_cursor, state.commit_message.chars().count());
}

#[test]
fn commit_input_ctrl_u_clears_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "subject\n\nbody".into();
    state.commit_cursor = state.commit_message.chars().count();
    state.commit_scroll_offset = 3;

    panel::commit::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(state.commit_message, "");
    assert_eq!(state.commit_cursor, 0);
    assert_eq!(state.commit_scroll_offset, 0);
}

#[test]
fn commit_input_mouse_click_places_cursor() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "one\ntwo".into();

    let area = ratatui::layout::Rect::new(0, 0, 100, 30);
    assert!(panel::commit::place_cursor_at(&mut state, area, 12, 6));

    assert_eq!(state.commit_cursor, 5);
}

#[test]
fn commit_input_accepts_long_multiline_message() {
    let mut state = make_state_with_files();
    state.modal = Modal::Commit;
    state.commit_message = "x".repeat(2_048);
    state.commit_cursor = state.commit_message.chars().count();

    panel::commit::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    panel::commit::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();

    assert_eq!(state.commit_message, format!("{}\ny", "x".repeat(2_048)));
}

#[test]
fn commit_modal_uses_terminal_cursor_for_multiline_message() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state = make_state_with_files();
    app.state.modal = Modal::Commit;
    app.state.commit_message = "one\ntwo".into();
    app.state.commit_cursor = app.state.commit_message.chars().count();

    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(all_text.contains("one"), "missing first message line");
    assert!(all_text.contains("two"), "missing second message line");
    assert!(
        !all_text.contains('\u{2588}'),
        "commit input should not render a block character as the cursor"
    );
    app.terminal.backend_mut().assert_cursor_position((14, 6));
}

#[test]
fn commit_modal_scrolls_to_cursor_for_long_multiline_message() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state = make_state_with_files();
    app.state.modal = Modal::Commit;
    app.state.commit_message = (0..40)
        .map(|i| format!("line-{i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.state.commit_cursor = app.state.commit_message.chars().count();

    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(all_text.contains("line-39"), "missing final message line");
    assert!(
        !all_text.contains("line-00"),
        "oldest message line should scroll out of the editor viewport"
    );
    app.terminal.backend_mut().assert_cursor_position((18, 23));
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

#[test]
fn diff_view_v_toggles_side_by_side_mode() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::File("src/main.rs".into());

    assert_eq!(state.diff_view_mode, DiffViewMode::SideBySide);

    panel::main::handle_key(&mut state, key(KeyCode::Char('v'))).unwrap();

    assert_eq!(state.diff_view_mode, DiffViewMode::Unified);
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|status| status.text.contains("unified"))
    );

    panel::main::handle_key(&mut state, key(KeyCode::Char('v'))).unwrap();

    assert_eq!(state.diff_view_mode, DiffViewMode::SideBySide);
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|status| status.text.contains("side-by-side"))
    );
}

#[test]
fn side_by_side_diff_pairs_replaced_lines() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 20)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::File("src/main.rs".into());
    app.state.diff_view_mode = DiffViewMode::SideBySide;
    app.state.diff_text = [
        "diff --git a/src/main.rs b/src/main.rs",
        "--- a/src/main.rs",
        "+++ b/src/main.rs",
        "@@ -10,3 +20,3 @@",
        " context()",
        "-old_value()",
        "+new_value()",
        " after()",
    ]
    .join("\n");

    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let rows: Vec<String> = (0..buf.area.height)
        .map(|row| {
            (0..buf.area.width)
                .map(|col| buf[(col, row)].symbol())
                .collect::<String>()
        })
        .collect();
    let change_row = rows
        .iter()
        .find(|row| row.contains("old_value()"))
        .expect("removed line row");

    assert!(change_row.contains("  11 - old_value()"), "{change_row}");
    assert!(change_row.contains("  21 + new_value()"), "{change_row}");
    assert!(buffer_text(&app).contains("Diff: side-by-side"));
}

#[test]
fn side_by_side_diff_line_count_pairs_replacements() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::File("src/main.rs".into());
    state.diff_view_mode = DiffViewMode::SideBySide;
    state.diff_viewport_width = 80;
    state.diff_text = [
        "diff --git a/src/main.rs b/src/main.rs",
        "--- a/src/main.rs",
        "+++ b/src/main.rs",
        "@@ -10,3 +20,3 @@",
        " context()",
        "-old_value()",
        "+new_value()",
        " after()",
    ]
    .join("\n");

    assert_eq!(panel::main::rendered_line_count(&state), 7);
}

#[test]
fn branch_log_render_clamps_offset_to_rendered_lines() {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    state.diff_line_count = 500;
    state.diff_viewport_height = 4;
    state.diff_offset = 500;

    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::main::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains("message"),
        "missing log bottom: {rendered}"
    );
}

#[test]
fn branch_log_scroll_up_clamps_stale_offset_to_rendered_lines() {
    let mut state = AppState::new();
    state.focus = Pane::Main;
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    state.diff_line_count = 500;
    state.diff_viewport_height = 2;
    state.diff_offset = 500;

    panel::main::handle_key(&mut state, key(KeyCode::Char('k'))).unwrap();

    assert_eq!(state.diff_offset, 0);
}

#[test]
fn branch_log_max_scroll_offset_ignores_trailing_newline() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    state.diff_viewport_height = 2;

    assert_eq!(panel::main::max_scroll_offset(&state), 1);
}

#[test]
fn branch_log_scroll_bound_counts_wrapped_visual_rows() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = "0123456789abcdefghij\nshort\n".into();
    state.diff_viewport_height = 1;
    state.diff_viewport_width = 10;

    assert_eq!(panel::main::max_scroll_offset(&state), 2);
    assert_eq!(panel::main::rendered_line_count(&state), 3);
}

#[test]
fn branch_log_fast_mouse_scroll_bursts_stay_in_bounds() {
    let mut state = AppState::new();
    state.diff_source = lg::state::DiffSource::Branch("main".into());
    state.diff_text = (0..50)
        .map(|i| format!("* commit {i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    state.diff_text.push('\n');
    state.diff_viewport_height = 5;
    let max_offset = panel::main::max_scroll_offset(&state);

    for _ in 0..100 {
        panel::main::scroll(&mut state, true, 3);
        assert!(state.diff_offset <= max_offset);
    }
    assert_eq!(state.diff_offset, max_offset);

    for _ in 0..100 {
        panel::main::scroll(&mut state, false, 3);
        assert!(state.diff_offset <= max_offset);
    }
    assert_eq!(state.diff_offset, 0);
}

#[test]
fn render_clamps_stale_branch_log_offset_before_next_mouse_burst() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 24)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Branch("main".into());
    app.state.diff_text = "* commit abc1234\n| Author: Alice\n| message\n".into();
    app.state.diff_offset = 500;

    app.render().unwrap();

    assert_eq!(
        app.state.diff_offset,
        panel::main::max_scroll_offset(&app.state)
    );
    for _ in 0..100 {
        panel::main::scroll(&mut app.state, true, 3);
        assert!(app.state.diff_offset <= panel::main::max_scroll_offset(&app.state));
    }
}
