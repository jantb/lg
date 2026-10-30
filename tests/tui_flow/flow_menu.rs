//! The branch actions behind `f`: what the flow menu offers, and the preview
//! beside it.

use super::common::*;

#[test]
fn pressing_f_opens_branch_actions_from_branches_pane() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    add_flow_branches(&mut app.state);
    app.state.focus = Pane::Branches;
    app.send_key(key(KeyCode::Char('F'))).unwrap();
    assert_eq!(app.state.modal, Modal::Flow);

    let text = buffer_text(&app);

    assert!(
        text.contains("Branch Actions"),
        "missing branch actions title: {text}"
    );
    // Nothing but a feature branch is released: with no feature checked out
    // the menu is trunk housekeeping, and develop is not offered as a target.
    assert!(
        !text.contains("Release current branch into develop"),
        "release offered without a feature branch: {text}"
    );
    assert!(
        text.contains("Start new feature from origin/main"),
        "missing new feature action: {text}"
    );
}

#[test]
fn flow_menu_offers_release_actions_for_a_test_only_repository() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.branches = vec![Branch {
        name: "test".into(),
        is_current: false,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }];
    app.state.release_branches = ReleaseBranches::new(None, Some("test".into()));
    app.state.branch = Some("feature/box-adjustments".into());
    app.state.focus = Pane::Branches;
    app.send_key(key(KeyCode::Char('F'))).unwrap();
    assert_eq!(app.state.modal, Modal::Flow);

    let text = buffer_text(&app);

    assert!(
        text.contains("Release current branch into test"),
        "missing test release action: {text}"
    );
    assert!(
        !text.contains("Reset test from origin/main"),
        "resetting test is the trunk's business, not a feature branch's: {text}"
    );

    // Standing on test, the reset of test is offered.
    app.state.modal = Modal::None;
    app.state.branch = Some("test".into());
    app.send_key(key(KeyCode::Char('F'))).unwrap();
    let text = buffer_text(&app);
    assert!(
        text.contains("Reset test from origin/main"),
        "missing test reset action: {text}"
    );
    assert!(
        !text.contains("develop"),
        "a repo without a develop branch should not offer develop actions: {text}"
    );
}

#[test]
fn flow_menu_names_the_dev_branch_the_repository_uses() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.release_branches = ReleaseBranches::new(Some("dev".into()), None);
    app.state.branch = Some("feature/box-adjustments".into());
    app.state.focus = Pane::Branches;
    app.send_key(key(KeyCode::Char('F'))).unwrap();

    let text = buffer_text(&app);

    assert!(
        text.contains("Release current branch into dev"),
        "missing dev release action: {text}"
    );
    assert!(
        !text.contains("into test"),
        "a repo without a test branch should not offer test actions: {text}"
    );
}

#[test]
fn pressing_f_does_not_open_branch_actions_outside_branches_pane() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.send_key(key(KeyCode::Char('F'))).unwrap();

    assert_eq!(app.state.modal, Modal::None);
}

fn render_flow_text(state: &AppState) -> String {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::flow::render(state, frame.area(), frame);
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

#[test]
fn flow_modal_hides_merge_main_on_main() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("main".into());
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        !text.contains("Merge origin/main into current branch"),
        "merge-main should be hidden on main: {text}"
    );
    assert!(
        text.contains("Start new feature from origin/main"),
        "other flow actions should remain visible: {text}"
    );
}

#[test]
fn flow_modal_shows_merge_main_on_develop_when_behind_main() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("develop".into());
    if let Some(branch) = state
        .branches
        .iter_mut()
        .find(|branch| branch.name == "develop")
    {
        branch.is_current = true;
        branch.behind_main = 2;
    }
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        text.contains("Merge origin/main into current branch"),
        "merge-main should be shown on stale develop: {text}"
    );
}

#[test]
fn flow_modal_hides_merge_main_on_release_next_when_not_behind_main() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("test".into());
    if let Some(branch) = state
        .branches
        .iter_mut()
        .find(|branch| branch.name == "test")
    {
        branch.is_current = true;
    }
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        !text.contains("Merge origin/main into current branch"),
        "merge-main should be hidden on up-to-date test: {text}"
    );
}

/// Moving a diff onto a new branch is trunk housekeeping: it is offered from
/// the trunk with a feature branch selected, not from the feature branch.
#[test]
fn branch_actions_show_transfer_diff_for_selected_feature_branch() {
    let mut state = AppState::new();
    state.branch = Some("main".into());
    state.focus = Pane::Branches;
    state.branches = vec![
        Branch {
            name: "main".into(),
            is_current: true,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "feature/current".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
    ];
    state.branches_list.idx = 1;
    state.modal = Modal::Flow;

    let text = render_flow_text(&state);

    assert!(
        text.contains("Move selected diff to a new branch"),
        "missing transfer action: {text}"
    );
}

/// A branch name gets typed the way it is spoken, and git will not take a
/// space in a ref. Typing one is a dash rather than a name the create rejects.
#[test]
fn a_space_typed_into_a_branch_name_becomes_a_dash() {
    let mut state = flow_menu_state();
    state.flow_input = Some(FlowAction::NewFeature);
    state.flow_text.clear();

    for typed in "feature/update cv s".chars() {
        panel::flow::handle_key(&mut state, key(KeyCode::Char(typed))).unwrap();
    }

    assert_eq!(state.flow_text, "feature/update-cv-s");
}

/// A checkout where every branch action applies, for exercising the menu.
fn flow_menu_state() -> AppState {
    let mut state = AppState::new();
    state.branch = Some("feature/send-cv".into());
    state.branches = vec![Branch {
        name: "feature/send-cv".into(),
        is_current: true,
        upstream: Some("origin/feature/send-cv".into()),
        upstream_gone: false,
        ahead: 1,
        behind: 0,
        behind_main: 2,
        last_commit_unix: None,
    }];
    state.branches_list.idx = 0;
    state.release_branches =
        lg::git::ReleaseBranches::new(Some("develop".into()), Some("test".into()));
    state.modal = Modal::Flow;
    state
}

fn flow_menu_text(state: &AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| panel::flow::render(state, frame.area(), frame))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    (0..buf.area.height)
        .map(|row| {
            (0..buf.area.width)
                .map(|col| buf[(col, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// "Release" and "reset" both end in a force-push somewhere and only one throws
/// history away, so the menu shows what the highlighted action would do rather
/// than leaving its name to carry the whole meaning.
#[test]
fn the_flow_menu_previews_the_highlighted_action() {
    let mut state = flow_menu_state();
    state.flow_list.idx = FlowAction::ALL
        .iter()
        .position(|action| *action == FlowAction::ReleaseTest)
        .unwrap();

    let text = flow_menu_text(&state, 140, 40);

    assert!(text.contains("What it does"), "{text}");
    assert!(
        text.contains("origin/main") && text.contains("origin/test"),
        "the graph should name the branches it moves between: {text}"
    );
    assert!(
        text.contains("merge into test"),
        "and say it in words too: {text}"
    );
    assert!(
        text.contains("merge origin/feature/send-cv"),
        "the steps it would run belong there as well: {text}"
    );
}

/// The graph is coloured by branch, and the same branch is the same colour
/// wherever lg draws it — a diagram in one colour would be a diagram nobody can
/// follow.
#[test]
fn the_flow_preview_is_coloured_by_branch() {
    let mut state = flow_menu_state();
    // Resetting an environment is offered on that environment's branch.
    state.branch = Some("test".into());
    state.branches[0].name = "test".into();
    state.flow_list.idx = panel::flow::available_actions(&state)
        .iter()
        .position(|action| *action == FlowAction::ResetTest)
        .expect("the test branch offers its reset");

    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| panel::flow::render(&state, frame.area(), frame))
        .unwrap();
    let buf = terminal.backend().buffer().clone();

    let mut seen = HashSet::new();
    let mut marker = HashSet::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            let cell = &buf[(col, row)];
            if cell.symbol().contains('\u{25c9}') {
                marker.insert(cell.fg);
            } else if cell
                .symbol()
                .chars()
                .any(|c| "\u{25cf}\u{25c6}\u{2717}".contains(c))
            {
                seen.insert(cell.fg);
            }
        }
    }

    assert!(
        seen.contains(&lg::ui::palette::LANE_MAIN),
        "main should keep its colour: {seen:?}"
    );
    assert!(
        seen.contains(&lg::ui::palette::LANE_TEST),
        "test should keep its colour: {seen:?}"
    );
    assert!(
        seen.contains(&lg::ui::palette::LANE_LOST),
        "a reset should draw the history it drops in red: {seen:?}"
    );
    assert!(
        !marker.is_empty() && marker.is_disjoint(&seen),
        "the travelling marker should stand out from every lane: {marker:?} vs {seen:?}"
    );
}

#[test]
fn the_flow_preview_animates_as_the_clock_ticks() {
    let mut state = flow_menu_state();
    state.flow_list.idx = FlowAction::ALL
        .iter()
        .position(|action| *action == FlowAction::ReleaseTest)
        .unwrap();

    let first = flow_menu_text(&state, 140, 40);
    let moved = (1..40).any(|tick| {
        state.animation_ms = tick * 120;
        flow_menu_text(&state, 140, 40) != first
    });
    assert!(moved, "the preview should be moving: {first}");
    assert!(
        state.wants_animation(),
        "and the loop should be redrawing fast enough to show it"
    );
}

/// The preview pane is far bigger than the few rows and columns a graph needs
/// at its smallest, and a diagram drawn at its smallest in the corner of it
/// wastes the room that would make it readable. So the picture is drawn to the
/// pane it has: a bigger pane, a bigger picture, across and down.
#[test]
fn the_flow_preview_is_drawn_to_the_size_of_its_pane() {
    let mut state = flow_menu_state();
    state.flow_list.idx = FlowAction::ALL
        .iter()
        .position(|action| *action == FlowAction::ReleaseTest)
        .unwrap();

    /// How far the drawn graph reaches: the dashes along one lane's track, and
    /// the rows from the first lane to the last.
    fn extent(text: &str) -> (usize, usize) {
        let rows: Vec<&str> = text.lines().collect();
        let across = rows
            .iter()
            .find(|row| row.contains("origin/main"))
            .map_or(0, |row| row.matches('\u{2500}').count());
        let lanes: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.contains('\u{25cf}'))
            .map(|(idx, _)| idx)
            .collect();
        let down = match (lanes.first(), lanes.last()) {
            (Some(first), Some(last)) => last - first,
            _ => 0,
        };
        (across, down)
    }

    let (wide, tall) = extent(&flow_menu_text(&state, 160, 50));
    let (narrow, short) = extent(&flow_menu_text(&state, 100, 30));

    assert!(narrow > 0 && short > 0, "both panes should draw a graph");
    assert!(
        wide > narrow,
        "a wider pane should draw a wider graph: {wide} vs {narrow}"
    );
    assert!(
        tall > short,
        "a taller pane should spread the lanes further apart: {tall} vs {short}"
    );
}

/// A terminal with no room for the graph still gets the menu; the preview is
/// what gives way, not the thing being chosen from.
#[test]
fn a_narrow_flow_menu_drops_the_preview_and_keeps_the_actions() {
    let state = flow_menu_state();
    let text = flow_menu_text(&state, 64, 24);

    assert!(text.contains("Release current branch into test"), "{text}");
    assert!(!text.contains("What it does"), "{text}");
}
