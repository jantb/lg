//! The review chat beside the walk.

use super::*;

#[test]
fn a_drag_started_in_the_chat_copies_only_the_chat() {
    let mut state = walk_with_findings();
    chatting(&mut state, "echo chat says hello; sleep 30", "hello");
    let (_, side) = panes(&state);
    let chat = draw::chat_screen_area(state.guided.as_deref().unwrap(), side, state.diff_view_mode)
        .unwrap();

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        chat.x,
        chat.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        0,
        AREA.height - 1,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        0,
        AREA.height - 1,
    );

    let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(&state, AREA, frame)).unwrap();
    state.sessions.close_all();
    let copied = state.selection.unwrap().text(terminal.backend().buffer());
    assert!(copied.contains("chat says hello"), "{copied:?}");
    assert!(
        !copied.contains("Renames"),
        "the read beside it: {copied:?}"
    );
    assert!(!copied.contains("new one"), "the diff above it: {copied:?}");
}

#[test]
fn a_running_chat_stays_on_screen_through_the_final_review() {
    let mut state = walk_with_findings();
    chatting(&mut state, "echo ready; sleep 30", "ready");

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('w'));

    assert!(state.guided.as_ref().unwrap().on_final());
    let rows = screen(&state);
    state.sessions.close_all();
    assert!(
        rows.iter().any(|row| row.contains("Claude \u{b7} a.rs")),
        "{rows:#?}"
    );
}

#[test]
fn the_chat_starts_on_what_is_open_in_the_file_on_screen() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.progress.ticked.push(TickedFinding {
        path: "a.rs".into(),
        line: 3,
        text: "tail could say what it ends.".into(),
    });

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('C'));

    assert_eq!(state.pending_action, Some(PendingAction::GuidedChat));
    let brief = state.guided.as_ref().unwrap().chat_brief("a.rs");
    assert!(
        brief.contains("L2 issue: -1 stands in for missing"),
        "{brief}"
    );
    assert!(!brief.contains("tail could say"), "{brief}");
    assert!(!brief.contains("y says less"), "{brief}");

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('Y'));
    let Some(PendingAction::CopyToClipboard { text, .. }) = &state.pending_action else {
        panic!("Y copies");
    };
    assert_eq!(*text, brief);
}
