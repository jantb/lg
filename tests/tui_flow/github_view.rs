//! GitHub in the main pane: it fills the pane, the panes beside it stay in
//! view and keep working, and the focus moves between them as it does for the
//! diff.

use super::common::*;

fn github_app() -> lg::app::HeadlessApp<TestBackend> {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(160, 40)).unwrap();
    app.state = make_state_with_files();
    app.state.show_github();
    app
}

/// It used to float over the screen as a modal, centred and capped in
/// width, hiding the panes and leaving a strip of the main pane unused.
#[test]
fn github_fills_the_main_pane_beside_the_left_panes() {
    let mut app = github_app();
    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(screen.contains("GitHub"), "{screen}");
    assert!(screen.contains("Pull requests"), "{screen}");
    assert!(
        screen.contains("Files"),
        "the left panes stay in view: {screen}"
    );
    assert_eq!(app.state.modal, Modal::None);
}

#[test]
fn the_pane_keys_move_the_focus_out_of_github_and_back() {
    let mut app = github_app();
    app.send_key(key(KeyCode::Char('1'))).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    assert!(app.state.github_view(), "GitHub stays on screen");

    app.send_key(key(KeyCode::Char('0'))).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
    assert!(app.state.github_has_keyboard());

    app.send_key(key(KeyCode::Esc)).unwrap();
    assert!(
        !app.state.github_view(),
        "Esc gives the pane back to the diff"
    );
}

/// Picking a file asks for its diff, and the diff is drawn where GitHub is.
#[test]
fn focusing_the_files_puts_github_in_the_background() {
    let mut app = github_app();
    app.send_key(key(KeyCode::Char('2'))).unwrap();
    assert_eq!(app.state.focus, Pane::Files);
    assert!(!app.state.github_view());
}

/// The repository search takes every letter, digits included.
#[test]
fn digits_typed_into_the_repository_search_stay_there() {
    let mut app = github_app();
    app.state.github.tab = panel::github::Tab::Repositories;
    app.send_key(key(KeyCode::Char('1'))).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
    assert_eq!(app.state.github.query.as_str(), "1");
}

#[test]
fn clicking_beside_github_focuses_that_pane_and_clicking_it_takes_the_focus_back() {
    let mut app = github_app();
    app.render().unwrap();
    app.send_mouse(left_click(3, 2)).unwrap();
    assert_eq!(app.state.focus, Pane::Status);
    assert!(app.state.github_view());

    app.send_mouse(left_click(120, 20)).unwrap();
    assert_eq!(app.state.focus, Pane::Main);
    assert!(app.state.github_has_keyboard());
}
