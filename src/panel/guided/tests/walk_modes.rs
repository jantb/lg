//! The plan's walk modes and its concerns.

use super::*;

#[test]
fn the_plan_is_read_first_and_a_file_only_once_the_walk_reaches_it() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();

    guided.current = 1;
    assert_eq!(steps_to_read(guided), vec![OVERVIEW, 1]);
    guided.current = OVERVIEW;
    assert_eq!(steps_to_read(guided), vec![OVERVIEW]);
    guided.current = guided.final_step();
    assert_eq!(steps_to_read(guided), vec![OVERVIEW, guided.final_step()]);

    guided.owe_chat("a.rs".into(), ChatNews::FileRead);
    guided.owe_chat("a.rs".into(), ChatNews::Redone);
    guided.owe_chat("a.rs".into(), ChatNews::FileRead);
    assert_eq!(
        guided.chat_owed,
        vec![("a.rs".to_string(), ChatNews::Redone)]
    );
    guided.current = 2;
    assert_eq!(steps_to_read(guided), vec![OVERVIEW, 2, 1]);
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
fn on_the_map_the_walk_follows_the_plans_reading_order() {
    let mut state = walk_with_plan();
    state.guided.as_mut().unwrap().progress.walk = WalkMode::Map;

    assert_eq!(
        walked(&state),
        [
            "tests/cache.rs",
            "src/cache.rs",
            "src/lookup.rs",
            "Cargo.lock"
        ]
    );
}

#[test]
fn triage_walks_the_riskiest_first_and_s_marks_the_files_to_skim_reviewed() {
    use ratatui::crossterm::event::KeyCode::Char;
    let mut state = walk_with_plan();
    state.guided.as_mut().unwrap().progress.walk = WalkMode::Triage;

    assert_eq!(
        walked(&state),
        [
            "src/lookup.rs",
            "src/cache.rs",
            "tests/cache.rs",
            "Cargo.lock"
        ]
    );

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
    for walk in [WalkMode::Concerns, WalkMode::Map, WalkMode::Triage] {
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
    assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Map);
    assert!(screen(&state)[badge.y as usize].contains("MAP"));
    click(&mut state);
    assert_eq!(state.guided.as_ref().unwrap().walk(), WalkMode::Triage);
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

#[test]
fn triage_lists_the_files_in_risk_bands_and_the_map_numbers_them() {
    let mut state = walk_with_plan();
    state.guided.as_mut().unwrap().progress.walk = WalkMode::Triage;
    let rows = list_text(&state);
    assert!(row_of(&rows, "High risk") < row_of(&rows, "lookup.rs"));
    assert!(row_of(&rows, "lookup.rs") < row_of(&rows, "Medium risk"));
    assert!(row_of(&rows, "Medium risk") < row_of(&rows, "cache.rs"));
    assert!(row_of(&rows, "Safe to skim") < row_of(&rows, "Cargo.lock"));

    state.guided.as_mut().unwrap().progress.walk = WalkMode::Map;
    let rows = list_text(&state);
    assert!(!rows.iter().any(|row| row.contains("risk")));
    // Numbered in the plan's order: the tests first, the lockfile last.
    assert!(row_of(&rows, "tests/") < row_of(&rows, "1 cache.rs"));
    assert!(rows.iter().any(|row| row.contains("4 Cargo.lock")));
}

/// The button reads the whole concern; a click on the heading's title
/// does nothing, so reading is always chosen.
#[test]
fn a_concerns_button_reads_all_of_its_files_and_only_the_button_does() {
    let mut state = walk_with_plan();
    state.ai_assist = true;
    let (list, _) = panes(&state);
    let rows = list_text(&state);
    let heading = list.y + row_of(&rows, "The cache") as u16;
    assert!(rows[row_of(&rows, "The cache")].contains("read all"));
    let click = |state: &mut AppState, column: u16| {
        mouse(
            state,
            MouseEventKind::Down(MouseButton::Left),
            column,
            heading,
        )
    };

    click(&mut state, list.x + 3);
    assert!(!state.guided.as_ref().unwrap().wants_read("src/cache.rs"));

    click(&mut state, list.x + list.width - 2);
    let guided = state.guided.as_deref().unwrap();
    assert!(guided.wants_read("src/cache.rs") && guided.wants_read("src/lookup.rs"));
    assert!(!guided.wants_read("tests/cache.rs"));
    let number = |path: &str| guided.steps.iter().position(|s| s.path == path).unwrap() + 1;
    let wanted = steps_to_read(guided);
    assert!(wanted.contains(&number("src/cache.rs")));
    assert!(wanted.contains(&number("src/lookup.rs")));
    assert!(!wanted.contains(&number("tests/cache.rs")));
}

#[test]
fn a_reads_the_concern_of_the_file_on_screen() {
    let mut state = walk_with_plan();
    state.ai_assist = true;
    let guided = state.guided.as_mut().unwrap();
    guided.current = guided
        .steps
        .iter()
        .position(|s| s.path == "tests/cache.rs")
        .unwrap()
        + 1;

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('A'));

    let guided = state.guided.as_deref().unwrap();
    assert!(guided.wants_read("tests/cache.rs"));
    assert!(!guided.wants_read("src/cache.rs"));
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
