//! The step list and the panes beside it, under the mouse.

use super::*;

#[test]
fn each_file_is_one_row_that_opens_it() {
    let mut state = walk_on_a_long_hunk();
    let two_hunk_file = |name: &str| {
        format!(
            "diff --git a/{name} b/{name}\n--- a/{name}\n+++ b/{name}\n@@ -1 +1 @@ fn {name}\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n"
        )
    };
    let guided = state.guided.as_mut().unwrap();
    guided.steps = crate::git::guided::parse_files(&format!(
        "{}{}",
        two_hunk_file("first.rs"),
        two_hunk_file("second.rs")
    ));
    guided.current = OVERVIEW;
    let (list, _) = panes(&state);

    // The overview, first.rs, second.rs: the third row is the second file.
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        list.x + 2,
        list.y + 2,
    );

    let guided = state.guided.as_deref().unwrap();
    assert_eq!(
        guided.step().map(|step| step.path.as_str()),
        Some("second.rs")
    );
}

#[test]
fn the_step_list_shows_the_file_tree_and_a_directory_row_opens_nothing() {
    let mut state = walk_on_a_long_hunk();
    let file = |path: &str| {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-a\n+b\n")
    };
    let guided = state.guided.as_mut().unwrap();
    guided.steps =
        crate::git::guided::parse_files(&format!("{}{}", file("src/b.rs"), file("src/a/x.rs")));
    guided.current = OVERVIEW;
    let (list, _) = panes(&state);
    let rows = screen(&state);
    let row = |y: u16| rows[y as usize][..].to_string();
    assert!(row(list.y + 1).contains("src/"), "{}", row(list.y + 1));
    assert!(row(list.y + 2).contains("a/"), "{}", row(list.y + 2));
    assert!(row(list.y + 3).contains("x.rs"), "{}", row(list.y + 3));
    assert!(row(list.y + 4).contains("b.rs"), "{}", row(list.y + 4));
    let click = |state: &mut AppState, y: u16| {
        mouse(
            state,
            MouseEventKind::Down(MouseButton::Left),
            list.x + 4,
            y,
        )
    };

    click(&mut state, list.y + 1);
    assert_eq!(state.guided.as_ref().unwrap().current, OVERVIEW);

    click(&mut state, list.y + 3);
    let guided = state.guided.as_deref().unwrap();
    assert_eq!(guided.step().map(|s| s.path.as_str()), Some("src/a/x.rs"));
}

#[test]
fn the_step_list_is_as_wide_as_its_divider_is_dragged() {
    let mut state = walk_on_a_long_hunk();
    let (list, _) = panes(&state);
    let divider = list.x + list.width;
    let row = list.y + 2;

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        divider,
        row,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        divider + 10,
        row,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        divider + 10,
        row,
    );
    let (wider, side) = panes(&state);
    assert_eq!(wider.x + wider.width, divider + 10);
    assert!(side.x > divider + 10, "the hunk moves over for it");

    // A drag that did not start on the divider leaves the list be.
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        divider + 20,
        row,
    );
    assert_eq!(panes(&state).0, wider);
}

/// The verdict sits in the list's first column, where a list dragged
/// narrow cannot cut it off.
#[test]
fn a_steps_verdict_shows_at_the_left_edge_of_a_narrow_list() {
    let mut state = walk_on_a_long_hunk();
    state.guided_list_width = Some(12);
    let guided = state.guided.as_mut().unwrap();
    let key = guided.steps[0].key();
    guided
        .commentary
        .insert(key, commentary("Drops a check.\nVerdict: issue"));
    let (list, _) = panes(&state);

    let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(&state, AREA, frame)).unwrap();
    let buf = terminal.backend().buffer();

    assert!(
        (list.y..list.y + list.height).any(|y| buf[(list.x, y)].symbol() == "\u{25cf}"),
        "no verdict at the list's left edge"
    );
}

#[test]
fn the_wheel_over_the_hunk_scrolls_the_hunk_and_over_the_commentary_the_commentary() {
    let mut state = walk_on_a_long_hunk();
    let (_, side) = panes(&state);
    let guided = state.guided.as_deref().unwrap();
    let (parts, _) = draw::step_areas(guided, guided.step().unwrap(), side, state.diff_view_mode);
    let (hunk, commentary) = (parts[0], parts[1]);

    mouse(
        &mut state,
        MouseEventKind::ScrollDown,
        hunk.x + 5,
        hunk.y + 3,
    );
    let cursor = state.guided.as_ref().unwrap().cursor;
    assert!(cursor > 0, "the hunk's cursor moves down it");

    mouse(
        &mut state,
        MouseEventKind::ScrollDown,
        commentary.x + 5,
        commentary.y + 1,
    );
    let guided = state.guided.as_ref().unwrap();
    assert_eq!(guided.cursor, cursor, "the hunk stays where it was");
    assert!(guided.side_scroll > 0, "the commentary scrolls");
}

#[test]
fn the_overview_lists_each_file_once_with_its_totals() {
    let steps = crate::git::guided::parse_files(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,2 @@\n x\n+y\n@@ -9,1 +10,1 @@\n-p\n+q\n",
    );
    let Overview { text, brief } = overview_of("Branch b", "", &["abc fix it".into()], &steps);

    assert!(text.contains("- abc fix it"));
    assert_eq!(text.matches("a.rs").count(), 1, "{text}");
    assert!(text.contains("+2 -1"), "{text}");
    assert!(brief.contains("- a.rs\n"), "{brief}");
    assert!(!brief.contains("+2 -1"), "{brief}");
}

/// Scrolling the list opens nothing, so nothing is read; a click opens
/// the row under it, and the list stays where it was scrolled.
#[test]
fn the_wheel_scrolls_the_step_list_and_only_a_click_opens_a_step() {
    let mut state = walk_on_a_long_hunk();
    let diff: String = (0..60)
        .map(|n| {
            format!("diff --git a/f{n:02}.rs b/f{n:02}.rs\n--- a/f{n:02}.rs\n+++ b/f{n:02}.rs\n@@ -1 +1 @@\n-a\n+b\n")
        })
        .collect();
    let guided = state.guided.as_mut().unwrap();
    guided.steps = crate::git::guided::parse_files(&diff);
    guided.current = OVERVIEW;
    let (list, _) = panes(&state);
    let wheel =
        |state: &mut AppState| mouse(state, MouseEventKind::ScrollDown, list.x + 2, list.y + 2);

    for _ in 0..4 {
        wheel(&mut state);
    }
    assert_eq!(state.guided.as_ref().unwrap().current, OVERVIEW);
    assert!(state.guided.as_ref().unwrap().asks.is_empty());
    let rows = list_text(&state);
    assert!(
        !rows[0].contains("Overview"),
        "the list scrolled: {rows:#?}"
    );
    let row = row_of(&rows, "f20.rs");

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        list.x + 4,
        list.y + row as u16,
    );

    let guided = state.guided.as_deref().unwrap();
    assert_eq!(guided.step().map(|s| s.path.as_str()), Some("f20.rs"));
    assert_eq!(
        row_of(&list_text(&state), "f20.rs"),
        row,
        "the list stays put"
    );
}
