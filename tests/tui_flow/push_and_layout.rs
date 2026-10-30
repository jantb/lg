//! The push modal while it runs, and how the panels share the screen.

use super::common::*;

#[test]
fn push_modal_renders_spinner_when_job_running() {
    use lg::state::{PushJob, PushMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut state = AppState::new();
    state.modal = Modal::Push;
    state.branch = Some("feature/x".into());
    let (_tx, rx) = std::sync::mpsc::channel::<PushMsg>();
    state.push_job = Some(PushJob {
        rx,
        handle: None,
        spinner: 0,
        branch: "feature/x".into(),
        remote: "origin".into(),
    });

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::push::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }
    assert!(text.contains("Pushing"), "expected 'Pushing' label: {text}");
    assert!(
        text.contains("feature/x"),
        "expected branch name in modal: {text}"
    );
}

#[test]
fn push_modal_handle_key_is_noop_while_running() {
    use lg::state::{PushJob, PushMsg};
    use ratatui::crossterm::event::KeyCode;
    let mut state = AppState::new();
    state.modal = Modal::Push;
    let (_tx, rx) = std::sync::mpsc::channel::<PushMsg>();
    state.push_job = Some(PushJob {
        rx,
        handle: None,
        spinner: 0,
        branch: "main".into(),
        remote: "origin".into(),
    });

    panel::push::handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    // Esc must not close the modal while push is running.
    assert_eq!(state.modal, Modal::Push);
    panel::push::handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(state.pending_action.is_none());
}

#[test]
fn push_modal_enter_merges_when_branch_diverged() {
    let mut state = AppState::new();
    state.modal = Modal::Push;
    state.branch = Some("feature/diverged".into());
    state.ahead_behind = Some((1, 6));

    panel::push::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(state.pending_action, Some(PendingAction::MergeUpstream));
}

#[test]
fn layout_renders_all_panel_borders() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(80, 24)).unwrap();
    app.state = make_state_with_files();
    app.state.repo_root = Some("/tmp/work/lg".into());
    app.state.branch = Some("main".into());
    add_flow_branches(&mut app.state);
    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut all_text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            all_text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(all_text.contains("lg"), "missing project header");
    assert!(all_text.contains("Status"), "missing Status panel title");
    assert!(
        all_text.contains("Branch inclusion"),
        "missing Branch inclusion panel title"
    );
    assert!(all_text.contains("Files"), "missing Files panel title");
    assert!(
        all_text.contains("Branches"),
        "missing Branches panel title"
    );
    assert!(all_text.contains("Commits"), "missing Commits panel title");
}

#[test]
fn layout_gives_files_panel_environment_space_when_flow_is_hidden() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let with_flow = lg::ui::split_layout_with_environments(area, true);
    let without_flow = lg::ui::split_layout_with_environments(area, false);

    assert_eq!(without_flow.environments.height, 0);
    assert_eq!(
        without_flow.files.y,
        without_flow
            .status
            .y
            .saturating_add(without_flow.status.height)
    );
    assert!(
        without_flow.files.height > with_flow.files.height,
        "hidden environment height should be given to files"
    );
    assert_eq!(
        without_flow.branches.y,
        without_flow
            .files
            .y
            .saturating_add(without_flow.files.height)
    );
    assert_eq!(
        without_flow.commits.y,
        without_flow
            .branches
            .y
            .saturating_add(without_flow.branches.height)
    );
}

#[test]
fn layout_accepts_resized_left_column_width() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 24,
    };
    let resized = lg::ui::split_layout_with_width(area, true, Some(40));

    assert_eq!(resized.status.width, 40);
    assert_eq!(resized.main.x, 40);
    assert_eq!(resized.main.width, 80);
}

#[test]
fn layout_accepts_resized_left_panel_heights() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 24,
    };
    let resized = lg::ui::split_layout_with_sizes(area, true, Some(40), Some([4, 6, 5, 4, 4]));

    assert_eq!(resized.status.height, 4);
    assert_eq!(resized.environments.height, 6);
    assert_eq!(resized.files.height, 5);
    assert_eq!(resized.branches.height, 4);
    assert_eq!(resized.commits.height, 3);
}

#[test]
fn hidden_environment_layout_merges_saved_environment_height_into_files() {
    let area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 24,
    };
    let resized = lg::ui::split_layout_with_sizes(area, false, Some(40), Some([4, 6, 5, 4, 4]));

    assert_eq!(resized.environments.height, 0);
    assert_eq!(resized.files.height, 11);
}
