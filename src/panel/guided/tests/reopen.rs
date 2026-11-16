//! A review opened again picks up the reads it had, without asking again.

use crate::git::guided::TickedFinding;
use crate::state::AppState;

use crate::panel::guided::context::file_context;
use crate::panel::guided::load::apply_loaded;

use super::*;

const FOUND: &str = "the cache is never cleared, so a renamed entry is served stale.";

/// `walk_with_findings` saved as `branch`, with a.rs read afresh and saved.
fn read_and_saved(branch: &str) -> AppState {
    let mut state = walk_with_findings_saved_as(branch);
    answer(
        &mut state,
        1,
        &format!("Summary: Renames a line.\nL2 issue: {FOUND}\nVerdict: issue"),
    );
    state
}

/// The walk `state` has, opened again from what it saved, on a change of `steps`.
fn reopened(state: &AppState, steps: Vec<GuidedFile>) -> AppState {
    let guided = state.guided.as_deref().unwrap();
    let mut again = Guided::new(guided.source.clone());
    let loaded = Loaded {
        source: guided.source.clone(),
        steps,
        overview: Overview {
            text: "A rename.\n".into(),
            brief: "A rename.\n".into(),
        },
    };
    apply_loaded(&mut again, loaded, true);
    let mut state = AppState::new();
    state.guided = Some(Box::new(again));
    state
}

#[test]
fn a_review_opened_again_shows_the_same_findings_without_asking_the_model() {
    let state = read_and_saved("guided-test-reopen-same");
    let steps = state.guided.as_ref().unwrap().steps.clone();

    let state = reopened(&state, steps);

    let guided = state.guided.as_deref().unwrap();
    assert!(guided.is_read(&guided.steps[0]));
    assert!(
        guided.asks.is_empty(),
        "nothing was asked of the model to get it back"
    );
    assert!(
        shown_in(&state, "a.rs").contains(&(FOUND.to_string(), false)),
        "{:?}",
        shown_in(&state, "a.rs")
    );
}

#[test]
fn a_review_opened_again_after_a_file_changed_asks_only_what_became_of_its_findings() {
    let state = read_and_saved("guided-test-reopen-changed");
    let mut steps = state.guided.as_ref().unwrap().steps.clone();
    let line = steps[0]
        .lines
        .iter()
        .position(|line| line.text == "+new two")
        .unwrap();
    steps[0].lines[line].text = "+fixed two".into();

    let mut state = reopened(&state, steps);
    assert!(
        file_context(state.guided.as_deref().unwrap(), 1).contains(FOUND),
        "the read of a.rs is set against what was found before"
    );
    answer(
        &mut state,
        1,
        "Summary: Fixes the second rename.\nL2 issue: the cache is stale after a rename.\nVerdict: issue",
    );

    let shown = shown_in(&state, "a.rs");
    assert!(shown.contains(&(FOUND.to_string(), false)), "{shown:?}");
    assert!(
        !shown
            .iter()
            .any(|(text, _)| text == "the cache is stale after a rename."),
        "{shown:?}"
    );
}

#[test]
fn a_review_reset_opens_again_with_nothing_read() {
    use ratatui::crossterm::event::KeyCode::Char;
    let mut state = read_and_saved("guided-test-reopen-reset");
    let steps = state.guided.as_ref().unwrap().steps.clone();

    press(&mut state, Char('X'));
    press(&mut state, Char('X'));
    let state = reopened(&state, steps);

    let guided = state.guided.as_deref().unwrap();
    assert!(!guided.is_read(&guided.steps[0]));
}

/// Ticks were matched by their line too, once: one saved then, its finding
/// since moved to another line, still ticks it.
#[test]
fn a_tick_saved_on_a_line_its_finding_has_moved_from_still_ticks_it() {
    let mut state = walk_with_findings_saved_as("guided-test-reopen-old-tick");
    state
        .guided
        .as_mut()
        .unwrap()
        .progress
        .ticked
        .push(TickedFinding {
            path: "a.rs".into(),
            line: 7,
            text: "-1 stands in for missing and is used as an index.".into(),
        });

    assert!(shown_in(&state, "a.rs").contains(&(
        "-1 stands in for missing and is used as an index.".to_string(),
        true
    )));
}
