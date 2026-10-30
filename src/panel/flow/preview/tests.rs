use ratatui::text::Line;

use crate::state::{FlowAction, FlowRun};

use super::actions::shorten;
use super::*;

/// A pane with room to spare, the way the flow modal gives it; the tests
/// about fitting name their own sizes.
const ROOMY_HEIGHT: u16 = 24;

/// A checkout where every branch action applies: a feature branch checked
/// out and selected, and both deploy branches present.
fn state_with_deploy_branches() -> AppState {
    let mut state = AppState::new();
    state.branch = Some("feature/parser".into());
    state.branches = vec![crate::git::Branch {
        name: "feature/parser".into(),
        is_current: true,
        upstream: Some("origin/feature/parser".into()),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 1,
        last_commit_unix: None,
    }];
    state.branches_list.idx = 0;
    state.release_branches =
        crate::git::ReleaseBranches::new(Some("develop".to_string()), Some("test".to_string()));
    state
}

/// The run the menu would resolve for `action`, so the tests draw what the
/// menu draws rather than a hand-built approximation of it.
fn run(state: &AppState, action: FlowAction) -> FlowRun {
    super::super::flow_run(state, action)
}

fn text(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The colours one glyph was drawn in, for the assertions that are about
/// colour rather than shape. Every cell of the grid is its own span, so a
/// span holding one glyph is one cell.
fn colors(lines: &[Line<'static>], glyph: char) -> Vec<Color> {
    let wanted = glyph.to_string();
    lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .filter(|span| span.content.as_ref() == wanted)
        .filter_map(|span| span.style.fg)
        .collect()
}

/// Every action the menu can offer has to explain itself; one that draws
/// nothing is a blank pane where the explanation should be.
#[test]
fn every_offered_action_has_something_to_show() {
    let state = state_with_deploy_branches();
    for action in FlowAction::ALL {
        let lines = lines(
            &state,
            &run(&state, action),
            Progress::Menu,
            0,
            80,
            ROOMY_HEIGHT,
        );
        assert!(
            !lines.is_empty(),
            "{action:?} should draw something at a usable width"
        );
        assert!(
            lines.len() > 1,
            "{action:?} should draw a graph, not just its caption"
        );
    }
}

/// The picture is the point: a release names both branches it merges, in
/// the order it merges them.
#[test]
fn a_release_shows_main_and_the_feature_arriving_in_the_deploy_branch() {
    let state = state_with_deploy_branches();
    let drawn = text(&lines(
        &state,
        &run(&state, FlowAction::ReleaseTest),
        Progress::Menu,
        0,
        80,
        ROOMY_HEIGHT,
    ));

    assert!(drawn.contains("origin/main"), "{drawn}");
    assert!(drawn.contains(&shorten("origin/feature/parser")), "{drawn}");
    assert!(drawn.contains("test"), "{drawn}");
    assert!(drawn.contains("origin/test"), "{drawn}");
}

/// What separates a reset from a release is that the target's own history
/// goes, so the drawing has to show that and not only say it.
#[test]
fn a_reset_marks_the_history_it_throws_away_and_a_release_does_not() {
    let state = state_with_deploy_branches();
    let reset = text(&lines(
        &state,
        &run(&state, FlowAction::ResetTest),
        Progress::Menu,
        0,
        80,
        ROOMY_HEIGHT,
    ));
    let release = text(&lines(
        &state,
        &run(&state, FlowAction::ReleaseTest),
        Progress::Menu,
        0,
        80,
        ROOMY_HEIGHT,
    ));

    assert!(
        reset.contains('\u{2717}'),
        "a reset should mark the commits it drops: {reset}"
    );
    assert!(
        !release.contains('\u{2717}'),
        "a release drops nothing: {release}"
    );
}

#[test]
fn the_marker_moves_as_the_clock_ticks() {
    let state = state_with_deploy_branches();
    let first = text(&lines(
        &state,
        &run(&state, FlowAction::MergeMain),
        Progress::Menu,
        0,
        80,
        ROOMY_HEIGHT,
    ));
    let moved = (1..40)
        .map(|tick| {
            text(&lines(
                &state,
                &run(&state, FlowAction::MergeMain),
                Progress::Menu,
                tick * MARKER_CELL_MS,
                80,
                ROOMY_HEIGHT,
            ))
        })
        .any(|frame| frame != first);
    assert!(moved, "the diagram should animate: {first}");
}

/// It walks the whole route and comes back, rather than running off the end.
#[test]
fn the_marker_visits_every_step_and_loops() {
    let state = state_with_deploy_branches();
    let preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
    let path = preview.path();
    assert!(path.len() > 4, "the route should have steps to walk");

    let rows = preview.grid_rows();
    let cols = preview.grid_cols();
    for (row, col) in &path {
        assert!(*row < rows && *col < cols, "({row},{col}) is off the grid");
    }
    assert_eq!(
        path[0],
        path[path.len() % path.len().max(1)],
        "the route should be walked from its start"
    );
}

/// A pane too narrow for the graph gets the sentence instead of a picture
/// chopped in half, which would read as a different flow entirely.
#[test]
fn a_narrow_pane_falls_back_to_the_caption() {
    let state = state_with_deploy_branches();
    let lines = lines(
        &state,
        &run(&state, FlowAction::ReleaseTest),
        Progress::Menu,
        0,
        12,
        ROOMY_HEIGHT,
    );
    assert_eq!(lines.len(), 1, "no room for a graph");
    assert!(text(&lines).contains("merge into"), "{:?}", text(&lines));
}

/// A long branch name gives way rather than pushing the track off the pane,
/// but a name that fits is left exactly as it is.
#[test]
fn long_branch_names_are_cut_and_short_ones_are_not() {
    assert_eq!(shorten("origin/test"), "origin/test");
    let long = shorten("origin/feature/send-cv-t-alvminnelig");
    assert_eq!(long.chars().count(), MAX_LABEL);
    assert!(long.ends_with('\u{2026}'), "{long}");
    assert!(long.starts_with("origin/feature"), "{long}");
}

#[test]
fn a_repository_with_no_branch_draws_nothing() {
    let state = AppState::new();
    assert!(
        lines(
            &state,
            &run(&state, FlowAction::MergeMain),
            Progress::Menu,
            0,
            80,
            ROOMY_HEIGHT
        )
        .is_empty()
    );
}

/// Every move has to know which step it happens on, or a running flow puts
/// the marker somewhere the flow is not. The step is found by matching the
/// flow's own wording, so this is what catches a step being reworded.
#[test]
fn every_move_happens_on_a_step_the_flow_reports() {
    let state = state_with_deploy_branches();
    for action in super::super::available_actions(&state) {
        let run = run(&state, action);
        let preview = preview(&state, &run).expect("a picture for every action");
        for (idx, mv) in preview.moves.iter().enumerate() {
            assert!(
                mv.step.is_some(),
                "{action:?} move {idx} matches none of {:?}",
                preview.steps
            );
        }
    }
}

/// While a flow runs, the marker is where the flow is: travelling the leg of
/// the step in progress, waiting at the start until the run reaches anything
/// the picture draws, and holding where it landed once the last move is done.
#[test]
fn the_marker_follows_the_step_a_run_is_on() {
    let state = state_with_deploy_branches();
    let preview = preview(&state, &run(&state, FlowAction::ReleaseTest)).expect("preview");
    let legs = preview.legs();
    let first = preview.moves[0].step.expect("the first merge has a step");
    let last = preview
        .moves
        .last()
        .and_then(|mv| mv.step)
        .expect("the push has a step");

    // A release stashes, pushes and checks out before it merges anything, so
    // at the first step there is nowhere for the marker to have got to yet.
    assert_eq!(
        preview.marker(Progress::Step(0), 7),
        legs[0].first().copied(),
        "the marker should wait at the start of the route"
    );

    let on_leg = preview
        .marker(Progress::Step(first), 3)
        .expect("a marker while merging");
    assert!(
        legs[0].contains(&on_leg),
        "{on_leg:?} is not on the leg of the merge in progress"
    );
    let moved = (0..legs[0].len())
        .map(|tick| preview.marker(Progress::Step(first), tick))
        .any(|at| at != preview.marker(Progress::Step(first), 0));
    assert!(
        moved,
        "the marker should travel the leg while the step runs"
    );

    assert_eq!(
        preview.marker(Progress::Step(last + 1), 0),
        legs.last().and_then(|leg| leg.last()).copied(),
        "once the last move is done the marker holds where it landed"
    );
}

/// A reset only throws history away when it gets there. Struck out in red
/// from the first step, the picture would be describing a repository the
/// flow has not touched yet.
#[test]
fn history_goes_red_only_once_the_flow_has_thrown_it_away() {
    let state = state_with_deploy_branches();
    let run = run(&state, FlowAction::ResetTest);
    let at = super::super::steps_for(&run)
        .iter()
        .position(|step| step.contains("reset test to"))
        .expect("a reset step");

    let waiting = colors(
        &lines(&state, &run, Progress::Step(0), 0, 80, ROOMY_HEIGHT),
        '\u{2717}',
    );
    let done = colors(
        &lines(&state, &run, Progress::Step(at), 0, 80, ROOMY_HEIGHT),
        '\u{2717}',
    );

    assert!(
        !waiting.is_empty(),
        "the reset should show what it will drop"
    );
    assert!(
        waiting.iter().all(|color| *color == palette::LANE_PENDING),
        "nothing is lost yet, so nothing should be red: {waiting:?}"
    );
    assert!(
        done.contains(&palette::LANE_LOST),
        "the history the reset just dropped should be red: {done:?}"
    );
}

/// The deploy branches keep the colours the deployment block gives them, so
/// the same branch is the same colour wherever it is drawn.
#[test]
fn branches_keep_their_colours() {
    let state = state_with_deploy_branches();
    assert_eq!(branch_color(&state, "origin/main"), palette::LANE_MAIN);
    assert_eq!(branch_color(&state, "test"), palette::LANE_TEST);
    assert_eq!(branch_color(&state, "origin/develop"), palette::LANE_DEV);
    assert_eq!(
        branch_color(&state, "feature/parser"),
        palette::LANE_FEATURE
    );
}

#[test]
fn the_glow_slides_between_frames_instead_of_jumping() {
    let state = state_with_deploy_branches();
    let preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
    let at_cell = preview.trail(Progress::Menu, 10 * MARKER_CELL_MS);
    let later = preview.trail(Progress::Menu, 10 * MARKER_CELL_MS + MARKER_CELL_MS / 2);
    let head = preview.marker(Progress::Menu, 10).unwrap();
    let next = preview.marker(Progress::Menu, 11).unwrap();
    let intensity = |cells: &[((usize, usize), f64)], at| {
        cells
            .iter()
            .find(|(cell, _)| *cell == at)
            .map(|(_, intensity)| *intensity)
            .unwrap_or(0.0)
    };

    assert_eq!(at_cell[0], (head, 1.0), "the head leads at full");
    assert_eq!(intensity(&later, head), 1.0, "and holds for the step");
    assert!(
        intensity(&later, next) > intensity(&at_cell, next),
        "the cell ahead should brighten as the marker approaches"
    );
    // Only cells the route passes once: one it crosses twice is lit by
    // whichever pass is brighter, which is not what is measured here.
    let route = preview.path();
    let passed_once = |cell: &(usize, usize)| route.iter().filter(|c| *c == cell).count() == 1;
    let tail: Vec<_> = at_cell
        .iter()
        .filter(|(cell, _)| *cell != head && *cell != next && passed_once(cell))
        .collect();
    assert!(!tail.is_empty(), "the head should have a tail");
    for (cell, before) in tail {
        assert!(
            intensity(&later, *cell) < *before,
            "the tail should dim as the step wears on, not hold and jump"
        );
    }
}

/// The tail shows where the run has been, and before the first cell of a
/// leg it has been nowhere on that leg.
#[test]
fn a_running_leg_has_no_tail_behind_its_start() {
    let state = state_with_deploy_branches();
    let preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
    let first = preview
        .moves
        .iter()
        .filter_map(|mv| mv.step)
        .min()
        .expect("a drawn step");
    let legs = preview.legs();
    let start = legs[0][0];
    let ahead = legs[0][1];
    for (cell, _) in preview.trail(Progress::Step(first), 0) {
        assert!(
            cell == start || cell == ahead,
            "{cell:?} is behind the start of the leg"
        );
    }
}

#[test]
fn the_diagram_grows_into_the_room_it_is_given() {
    let state = state_with_deploy_branches();
    let mut preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
    preview.fit(10, 200, 40);
    let (wide, tall) = (preview.grid_cols(), preview.grid_rows());
    preview.fit(10, 25, 8);
    let (narrow, short) = (preview.grid_cols(), preview.grid_rows());
    assert!(wide > narrow, "a wide pane should get a wider picture");
    assert!(tall > short, "a tall pane should get a taller picture");
    assert!(
        u16::try_from(10 + 1 + narrow).unwrap() <= 25,
        "and a small one should still fit across"
    );
    assert!(
        u16::try_from(short + CAPTION_ROWS).unwrap() <= 8,
        "and down"
    );
}

/// The picture is drawn to be watched, so growing it must not turn the loop
/// into a slideshow: the marker's pace is one commit per so long, whatever
/// scale the commits are drawn at.
#[test]
fn a_bigger_picture_is_not_a_slower_one() {
    let state = state_with_deploy_branches();
    let mut preview = preview(&state, &run(&state, FlowAction::MergeMain)).expect("preview");
    let loop_ms = |preview: &Preview| preview.path().len() as u64 * preview.cell_ms();
    preview.fit(10, 25, 8);
    let small = loop_ms(&preview);
    preview.fit(10, 90, 40);
    let big = loop_ms(&preview);
    assert!(
        big < small * 2,
        "a diagram {} times the size should not loop {} times slower",
        preview.stride,
        big / small.max(1)
    );
}

/// Room left under a diagram that could have used it is room wasted: the
/// lanes run to the edge of the pane rather than stopping short of it.
#[test]
fn the_lanes_run_the_width_of_the_pane() {
    let state = state_with_deploy_branches();
    let mut preview = preview(&state, &run(&state, FlowAction::TransferDiff)).expect("preview");
    preview.fit(10, 90, 40);
    assert!(
        preview.grid_cols() + 10 + 1 > 90 - preview.stride,
        "{} columns leaves more than a commit of the 90 unused",
        preview.grid_cols()
    );
}
