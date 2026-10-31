//! The plan's walk modes and its concerns.

use crate::state::AppState;
use ratatui::crossterm::event::{MouseButton, MouseEventKind};

use crate::panel::guided::context::file_context;

use super::*;

/// The step number of `path` in the walk.
fn number(state: &AppState, path: &str) -> usize {
    let guided = state.guided.as_deref().unwrap();
    guided.steps.iter().position(|s| s.path == path).unwrap() + 1
}

#[test]
fn the_plan_is_read_first_then_the_file_on_screen_then_every_file_but_the_skims() {
    let mut state = walk_with_plan();
    let lookup = number(&state, "src/lookup.rs");
    let guided = state.guided.as_mut().unwrap();

    guided.current = OVERVIEW;
    let in_order = |state: &AppState| {
        let guided = state.guided.as_deref().unwrap();
        steps_to_read(guided)
            .into_iter()
            .map(|step| match step {
                OVERVIEW => "overview".to_string(),
                step => guided.steps[step - 1].path.clone(),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        in_order(&state),
        [
            "overview",
            "src/cache.rs",
            "src/lookup.rs",
            "tests/cache.rs"
        ]
    );

    state.guided.as_mut().unwrap().current = lookup;
    assert_eq!(
        in_order(&state),
        [
            "overview",
            "src/lookup.rs",
            "src/cache.rs",
            "tests/cache.rs"
        ]
    );
    // A skim is read only when the walk is on it.
    state.guided.as_mut().unwrap().current = number(&state, "Cargo.lock");
    assert_eq!(in_order(&state)[1], "Cargo.lock");
}

#[test]
fn the_final_review_waits_until_every_file_is_read_and_its_questions_settled() {
    let mut state = walk_with_plan();
    let guided = state.guided.as_mut().unwrap();
    let last = guided.final_step();
    guided.current = last;
    assert!(!steps_to_read(guided).contains(&last));
    assert_eq!(guided.final_progress(), Some(Progress::Queued { ahead: 3 }));

    for path in ["src/cache.rs", "src/lookup.rs", "tests/cache.rs"] {
        let key = guided.steps.iter().find(|s| s.path == path).unwrap().key();
        guided
            .commentary
            .insert(key, commentary("Summary: Fine.\nVerdict: ok"));
    }
    let lookup = guided
        .steps
        .iter()
        .find(|s| s.path == "src/lookup.rs")
        .unwrap()
        .key();
    guided.check_queue.push(lookup.clone());
    assert!(!guided.ready_for_final());
    guided.check_queue.clear();
    guided.commentary.get_mut(&lookup).unwrap().checking = true;
    assert_eq!(guided.files_settled(), (2, 3));
    assert!(!steps_to_read(guided).contains(&last));

    guided.commentary.get_mut(&lookup).unwrap().checking = false;
    assert!(guided.ready_for_final());
    assert_eq!(steps_to_read(guided).last(), Some(&last));
}

#[test]
fn a_file_the_chat_is_owed_is_read_before_the_rest() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.owe_chat("a.rs".into(), ChatNews::FileRead);
    guided.owe_chat("a.rs".into(), ChatNews::Redone);
    guided.owe_chat("a.rs".into(), ChatNews::FileRead);
    assert_eq!(
        guided.chat_owed,
        vec![("a.rs".to_string(), ChatNews::Redone)]
    );
    guided.current = 2;
    assert_eq!(steps_to_read(guided)[..3], [OVERVIEW, 2, 1]);
}

fn walked(state: &AppState) -> Vec<String> {
    let guided = state.guided.as_deref().unwrap();
    guided
        .walk_order()
        .into_iter()
        .map(|step| guided.steps[step - 1].path.clone())
        .collect()
}

#[test]
fn by_concern_the_walk_goes_concern_by_concern_with_each_listed_over_its_files() {
    use ratatui::crossterm::event::KeyCode::Right;
    let mut state = walk_with_plan();

    assert_eq!(
        walked(&state),
        [
            "src/cache.rs",
            "src/lookup.rs",
            "tests/cache.rs",
            "Cargo.lock"
        ]
    );
    press(&mut state, Right);
    let guided = state.guided.as_deref().unwrap();
    assert_eq!(guided.step().map(|s| s.path.as_str()), Some("src/cache.rs"));

    let (list, _) = panes(&state);
    let rows: Vec<String> = screen(&state)
        .into_iter()
        .skip(list.y as usize)
        .take(list.height as usize)
        .map(|row| {
            row.chars()
                .skip(list.x as usize)
                .take(list.width as usize)
                .collect()
        })
        .collect();
    let row_of = |text: &str| {
        rows.iter()
            .position(|row| row.contains(text))
            .unwrap_or_else(|| panic!("{text} is not listed: {rows:#?}"))
    };
    assert!(row_of("The cache") < row_of("cache.rs"));
    assert!(row_of("cache.rs") < row_of("lookup.rs"));
    assert!(row_of("lookup.rs") < row_of("Tests"));
    assert!(row_of("Tests") < row_of("tests/"));
    assert!(row_of("tests/") < row_of(plan::EVERYTHING_ELSE));
    assert!(row_of(plan::EVERYTHING_ELSE) < row_of("Cargo.lock"));
}

#[test]
fn s_marks_the_files_to_skim_reviewed() {
    use ratatui::crossterm::event::KeyCode::Char;
    let mut state = walk_with_plan();

    press(&mut state, Char('S'));
    let guided = state.guided.as_deref().unwrap();
    let reviewed: Vec<&str> = guided
        .steps
        .iter()
        .filter(|step| guided.is_reviewed(step))
        .map(|step| step.path.as_str())
        .collect();
    assert_eq!(reviewed, ["Cargo.lock"]);
}

#[test]
fn without_a_plan_every_mode_walks_the_file_tree() {
    let mut state = walk_with_plan();
    let guided = state.guided.as_mut().unwrap();
    guided.commentary.clear();
    for walk in [WalkMode::Concerns, WalkMode::Issues] {
        state.guided.as_mut().unwrap().progress.walk = walk;
        assert_eq!(
            walked(&state),
            [
                "Cargo.lock",
                "src/cache.rs",
                "src/lookup.rs",
                "tests/cache.rs"
            ]
        );
    }
}

/// One click, no confirm: every mode orders the same files, so switching
/// loses nothing.
#[test]
fn a_click_on_the_walk_badge_switches_mode_and_keeps_what_was_done() {
    let mut state = walk_with_plan();
    let guided = state.guided.as_mut().unwrap();
    // It reads back what it saved, which the other walks with the plan
    // would overwrite under the shared key.
    guided.source = Source::Branch {
        branch: "guided-test-walk-badge".into(),
        base_ref: "main".into(),
    };
    let reviewed = guided.steps[0].key();
    guided.progress.reviewed.push(reviewed.clone());
    let badge = draw::walk_badge_area(state.guided.as_deref().unwrap(), AREA).unwrap();
    let click = |state: &mut AppState| {
        mouse(
            state,
            MouseEventKind::Down(MouseButton::Left),
            badge.x + 1,
            badge.y,
        )
    };

    click(&mut state);
    assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Issues);
    assert!(screen(&state)[badge.y as usize].contains("ISSUES"));
    click(&mut state);
    assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Concerns);
    press(&mut state, ratatui::crossterm::event::KeyCode::Char('M'));
    assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Issues);
    press(&mut state, ratatui::crossterm::event::KeyCode::Char('M'));
    assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Concerns);

    let guided = state.guided.as_ref().unwrap();
    assert!(guided.progress.reviewed.contains(&reviewed));
    assert_eq!(
        crate::git::guided::load_progress(&guided.source.progress_key()).walk,
        WalkMode::Concerns
    );
}

#[test]
fn a_file_is_read_with_the_rest_of_its_concern() {
    let state = walk_with_plan();
    let guided = state.guided.as_deref().unwrap();
    let lookup = guided
        .steps
        .iter()
        .position(|step| step.path == "src/lookup.rs")
        .unwrap();

    let context = file_context(guided, lookup + 1);

    assert!(context.contains("The cache"), "{context}");
    assert!(context.contains("+fn evict()"), "{context}");
    assert!(!context.contains("+evicts the oldest"), "{context}");
}

#[test]
fn files_wait_for_the_plan_while_it_is_read() {
    let mut state = walk_with_plan();
    let guided = state.guided.as_mut().unwrap();
    guided
        .commentary
        .insert(OVERVIEW_KEY.to_string(), Commentary::default());
    guided.current = 1;

    start_wanted_commentary(guided);

    assert!(!guided.commentary.contains_key(&guided.steps[0].key()));
    assert!(guided.asks.is_empty());
}

/// A concern's heading counts its files in as they are read, then
/// tallies what is open in them.
#[test]
fn a_concerns_heading_counts_its_files_in_then_tallies_them() {
    let mut state = walk_with_plan();
    let rows = list_text(&state);
    assert!(
        rows[row_of(&rows, "The cache")].contains("0/2"),
        "{rows:#?}"
    );

    let guided = state.guided.as_mut().unwrap();
    for path in ["src/cache.rs", "src/lookup.rs"] {
        let key = guided.steps.iter().find(|s| s.path == path).unwrap().key();
        guided.commentary.insert(
            key,
            commentary("Summary: Fine.\nL1 nit: a name.\nVerdict: nit"),
        );
    }
    let rows = list_text(&state);
    assert!(
        rows[row_of(&rows, "The cache")].contains("\u{25cb}2"),
        "{rows:#?}"
    );
}

#[test]
fn the_plan_shows_how_far_it_has_got() {
    let mut state = walk_with_plan();
    let guided = state.guided.as_mut().unwrap();
    guided
        .commentary
        .insert(OVERVIEW_KEY.to_string(), Commentary::default());
    assert!(matches!(guided.planning(), Some(Progress::Reading { .. })));

    guided.commentary.get_mut(OVERVIEW_KEY).unwrap().text =
        "Summary: Adds a cache.\nConcern: The cache\nFiles: src/cache.rs, src/lookup.rs\n".into();
    assert_eq!(
        guided.planning(),
        Some(Progress::Placing { named: 2, total: 4 })
    );
    assert!(
        screen(&state)
            .iter()
            .any(|row| row.contains("2/4 files placed"))
    );
}
