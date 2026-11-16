//! The delete-branch modal offers to delete a branch's remote only when that
//! remote branch is the one the local branch tracks and nobody else's, and
//! says which remote branch it is.
use lg::git::Branch;
use lg::panel;
use lg::state::{AppState, Modal, Pane, PendingAction};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

fn branch(name: &str, upstream: Option<&str>) -> Branch {
    Branch {
        name: name.into(),
        is_current: false,
        upstream: upstream.map(str::to_owned),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Open the delete modal on `name` the way `D` in the branch list does.
fn open_delete(branches: Vec<Branch>, name: &str) -> AppState {
    let mut state = AppState::new();
    state.focus = Pane::Branches;
    state.branches_list.idx = branches
        .iter()
        .position(|b| b.name == name)
        .expect("branch is listed");
    state.branches = branches;
    let shift_d = KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT);
    panel::branches::handle_key(&mut state, shift_d).unwrap();
    assert_eq!(state.modal, Modal::DeleteBranch);
    state
}

fn modal_text(state: &AppState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
    terminal
        .draw(|frame| panel::delete_branch::render(state, frame.area(), frame))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

/// Whatever the user ticks, confirming never asks to delete a remote branch.
fn no_remote_deletion_is_reachable(state: &mut AppState) {
    for _ in 0..3 {
        panel::delete_branch::handle_key(state, key(KeyCode::Char(' '))).unwrap();
        panel::delete_branch::handle_key(state, key(KeyCode::Char('j'))).unwrap();
    }
    assert!(!state.delete_branch.remote, "no remote deletion is ticked");
}

#[test]
fn a_branch_started_from_origin_main_never_offers_to_delete_origin_main() {
    let mut state = open_delete(
        vec![
            branch("main", Some("origin/main")),
            branch("my-fix", Some("origin/main")),
        ],
        "my-fix",
    );

    let text = modal_text(&state);
    assert!(!text.contains("delete remote"), "{text}");
    no_remote_deletion_is_reachable(&mut state);
}

#[test]
fn an_upstream_another_local_branch_owns_is_not_offered() {
    let mut state = open_delete(
        vec![
            branch("colleague", Some("origin/colleague")),
            branch("my-fix", Some("origin/colleague")),
        ],
        "my-fix",
    );

    let text = modal_text(&state);
    assert!(!text.contains("delete remote"), "{text}");
    no_remote_deletion_is_reachable(&mut state);
}

#[test]
fn the_modal_names_the_remote_branch_the_branch_really_tracks() {
    let state = open_delete(
        vec![
            branch("main", Some("origin/main")),
            branch("local-name", Some("fork/feature/remote-name")),
        ],
        "local-name",
    );

    let text = modal_text(&state);
    assert!(text.contains("fork/feature/remote-name"), "{text}");
}

#[test]
fn confirming_deletes_the_remote_branch_the_branch_really_tracks() {
    let mut state = open_delete(
        vec![
            branch("main", Some("origin/main")),
            branch("local-name", Some("fork/feature/remote-name")),
        ],
        "local-name",
    );

    panel::delete_branch::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::delete_branch::handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    panel::delete_branch::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    let Some(PendingAction::DeleteBranch {
        delete_remote: Some(remote),
        ..
    }) = &state.pending_action
    else {
        panic!("a remote deletion is asked for: {:?}", state.pending_action);
    };
    assert_eq!(
        (remote.remote.as_str(), remote.local_name.as_str()),
        ("fork", "feature/remote-name")
    );
}
