//! The issues walk over every open finding.

use super::*;

/// Reads of three files and the final review, each with findings of
/// their own, walked by issue.
fn walk_with_findings_read() -> AppState {
    let mut state = walk_with_plan();
    let guided = state.guided.as_mut().unwrap();
    let key =
        |guided: &Guided, path: &str| guided.steps.iter().find(|s| s.path == path).unwrap().key();
    let reads = [
        (
            key(guided, "src/cache.rs"),
            "L1 nit: evict could say what it evicts.\nVerdict: nit",
        ),
        (
            key(guided, "src/lookup.rs"),
            "L1 question: is the cache warm here?\nVerdict: ok",
        ),
        (
            key(guided, "tests/cache.rs"),
            "L1 issue: the test never fills the cache.\nVerdict: issue",
        ),
    ];
    for (key, text) in reads {
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
    guided.progress.walk = WalkMode::Issues;
    state
}

#[test]
fn issues_lists_every_open_finding_worst_first_and_the_arrows_walk_them() {
    use ratatui::crossterm::event::KeyCode::{Char, Right};
    let mut state = walk_with_findings_read();

    let rows = list_text(&state);
    // The list is narrow: a finding shows where it is and how it starts.
    assert!(row_of(&rows, "Issues (2)") < row_of(&rows, "cache.rs:1 the test"));
    assert!(row_of(&rows, "Issues (2)") < row_of(&rows, "lookup.rs:1 lookup and"));
    assert!(row_of(&rows, "cache.rs:1 the test") < row_of(&rows, "Nits (1)"));
    assert!(row_of(&rows, "Nits (1)") < row_of(&rows, "Questions (1)"));

    let on = |state: &AppState| {
        let guided = state.guided.as_deref().unwrap();
        let step = guided.step().unwrap();
        (step.path.clone(), step.lines[guided.cursor].text.clone())
    };
    // The cache concern, lookup.rs in it, comes before the tests.
    press(&mut state, Right);
    assert_eq!(
        on(&state),
        ("src/lookup.rs".into(), "+lookup calls the cache".into())
    );
    press(&mut state, Right);
    assert_eq!(on(&state).0, "tests/cache.rs");

    press(&mut state, Char('t'));
    let guided = state.guided.as_deref().unwrap();
    assert!(
        guided
            .open_findings()
            .iter()
            .all(|open| open.finding.text != "the test never fills the cache.")
    );
    press(&mut state, Right);
    assert_eq!(
        on(&state).0,
        "src/cache.rs",
        "the nit, once the issues are done"
    );
}

#[test]
fn a_click_on_a_finding_opens_its_file_on_its_line() {
    let mut state = walk_with_findings_read();
    let (list, _) = panes(&state);
    let rows = list_text(&state);
    let row = list.y + row_of(&rows, "cache.rs:1 the test") as u16;

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        list.x + 6,
        row,
    );

    let guided = state.guided.as_deref().unwrap();
    let step = guided.step().unwrap();
    assert_eq!(step.path, "tests/cache.rs");
    assert_eq!(step.lines[guided.cursor].text, "+evicts the oldest");
}
