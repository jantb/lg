//! The overview: where the problems are, each leading to its place, and the
//! chat opened beside it.

use crate::state::AppState;
use ratatui::crossterm::event::{MouseButton, MouseEventKind};

use super::*;

/// The plan, the reads of its three files and the final review, on the
/// overview.
fn overview_with_findings() -> AppState {
    let mut state = walk_with_plan();
    state.ai_assist = true;
    let guided = state.guided.as_mut().unwrap();
    let key =
        |guided: &Guided, path: &str| guided.steps.iter().find(|s| s.path == path).unwrap().key();
    let reads = [
        (
            "src/cache.rs",
            "L1 nit: evict could say what it evicts.\nVerdict: nit",
        ),
        ("src/lookup.rs", "Summary: Fine.\nVerdict: ok"),
        (
            "tests/cache.rs",
            "L1 issue: the test never fills the cache.\nVerdict: issue",
        ),
    ];
    for (path, text) in reads {
        let key = key(guided, path);
        guided.commentary.insert(key, commentary(text));
    }
    let final_key = guided.final_key();
    guided.commentary.insert(
        final_key,
        commentary(
            "src/lookup.rs:L1 issue: lookup and evict take the lock in opposite order.\n\
             Verdict: issue",
        ),
    );
    guided.current = OVERVIEW;
    state
}

/// Press and release on the first screen row showing `text`, in the side pane.
fn click_on(state: &mut AppState, text: &str) {
    let (_, side) = panes(state);
    let row = screen(state)
        .iter()
        .position(|row| row.contains(text))
        .unwrap_or_else(|| panic!("{text} is not on screen")) as u16;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        mouse(state, kind, side.x + 4, row);
    }
}

fn on(state: &AppState) -> String {
    let guided = state.guided.as_deref().unwrap();
    guided.step().map(|s| s.path.clone()).unwrap_or_default()
}

#[test]
fn the_overview_lists_every_open_finding_worst_first() {
    let state = overview_with_findings();
    let rows = screen(&state);
    let at = |text: &str| row_of(&rows, text);

    assert!(at("Where the problems are") < at("the test never fills"));
    assert!(at("the test never fills") < at("evict could say"));
    assert!(at("take the lock in opposite") < at("evict could say"));
}

#[test]
fn a_click_on_a_finding_on_the_overview_opens_its_file_on_its_line() {
    let mut state = overview_with_findings();

    click_on(&mut state, "the test never fills");

    assert_eq!(on(&state), "tests/cache.rs");
    let guided = state.guided.as_deref().unwrap();
    assert_eq!(
        guided.step().unwrap().lines[guided.cursor].text,
        "+evicts the oldest"
    );
}

#[test]
fn a_click_on_a_risky_file_on_the_overview_opens_it() {
    let mut state = overview_with_findings();

    click_on(&mut state, "a new call path");

    assert_eq!(on(&state), "src/lookup.rs");
}

#[test]
fn a_click_on_plain_overview_text_stays_on_the_overview() {
    let mut state = overview_with_findings();

    click_on(&mut state, "Adds a cache in front of lookup");

    assert_eq!(state.guided.as_deref().unwrap().current, OVERVIEW);
}

#[test]
fn the_chat_on_the_overview_hears_every_open_finding_across_the_change() {
    let state = overview_with_findings();
    let guided = state.guided.as_deref().unwrap();

    assert_eq!(guided.chat_subject().as_deref(), Some(WHOLE_CHANGE));
    let brief = guided.chat_brief(WHOLE_CHANGE);
    assert!(
        brief.contains("tests/cache.rs:L1 issue: the test never fills"),
        "{brief}"
    );
    assert!(
        brief.contains("src/cache.rs:L1 nit: evict could say"),
        "{brief}"
    );
    assert!(
        brief.contains("src/lookup.rs:L1 issue (from the review of the whole change)"),
        "{brief}"
    );
}

#[test]
fn claude_opens_beside_a_review_with_a_model_and_a_checkout() {
    let mut state = overview_with_findings();
    let guided = state.guided.as_mut().unwrap();

    assert!(opens_chat_beside(guided, true, true));
    assert!(
        !opens_chat_beside(guided, false, true),
        "no model, nothing to fix"
    );
    assert!(
        !opens_chat_beside(guided, true, false),
        "no checkout to fix in"
    );
    guided.source = Source::PullRequest {
        number: 7,
        title: "t".into(),
        head: "h".into(),
        base: "main".into(),
        commit: "c".into(),
        local: false,
    };
    assert!(
        !opens_chat_beside(guided, true, true),
        "not checked out here"
    );
}

/// The overview with a chat beside it, as far as the layout knows.
fn overview_with_chat() -> AppState {
    let mut state = overview_with_findings();
    state.guided.as_mut().unwrap().chat = Some(crate::session::SessionId::unstarted(1));
    state
}

fn chat_screen(state: &AppState) -> ratatui::layout::Rect {
    let (_, side) = panes(state);
    draw::chat_screen_area(state.guided.as_deref().unwrap(), side, state.diff_view_mode).unwrap()
}

#[test]
fn dragging_the_divider_left_of_the_chat_sizes_it_for_the_next_review_too() {
    let mut state = overview_with_chat();
    let before = chat_screen(&state);
    let divider = before.x - 1;

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        divider,
        before.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        divider - 10,
        before.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        divider - 10,
        before.y,
    );

    let after = chat_screen(&state);
    assert_eq!(after.width, before.width + 10);
    assert_eq!(
        after.x + after.width,
        before.x + before.width,
        "the right edge stays"
    );
    assert_eq!(state.guided_chat_width, Some(after.width));
    assert_eq!(
        state.guided.as_deref().unwrap().current,
        OVERVIEW,
        "a drag goes nowhere"
    );
}

#[test]
fn angle_brackets_widen_and_narrow_the_chat() {
    use ratatui::crossterm::event::KeyCode::Char;
    let mut state = overview_with_chat();
    let (_, side) = panes(&state);
    state.guided.as_ref().unwrap().chat_room.set(side.width);
    let before = chat_screen(&state).width;

    press(&mut state, Char('<'));
    assert_eq!(chat_screen(&state).width, before + 4);
    press(&mut state, Char('>'));
    press(&mut state, Char('>'));
    assert_eq!(chat_screen(&state).width, before - 4);
}

#[test]
fn the_chat_and_the_read_beside_it_each_keep_a_minimum() {
    assert_eq!(draw::chat_width(None, 100), 60);
    assert_eq!(draw::chat_width(Some(5), 100), 24);
    assert_eq!(
        draw::chat_width(Some(95), 100),
        79,
        "the read keeps 20 and the divider"
    );
    assert_eq!(
        draw::chat_width(Some(40), 30),
        9,
        "too narrow for both: the read wins"
    );
}
