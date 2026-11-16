//! Keys that do the same thing are documented as doing the same thing, so the
//! help and the footer describe what pressing the key really does.
use lg::panel::{self, keys};
use lg::state::{AppState, Modal};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn help(section: &str, key: &str) -> &'static str {
    keys::section(section)
        .and_then(|s| s.bindings.iter().find(|b| b.key == key))
        .unwrap_or_else(|| panic!("{section} documents {key}"))
        .help
}

fn footer_label(section: &str, key: &str) -> &'static str {
    keys::footer_entry(keys::section(section).unwrap(), key)
        .unwrap_or_else(|| panic!("{section} footer shows {key}"))
        .1
}

fn author_action(key: KeyEvent) -> Option<lg::state::PendingAction> {
    let mut state = AppState::new();
    state.modal = Modal::Author;
    state.author.path.text = "services/api".into();
    state.author.name.text = "Ada".into();
    state.author.email.text = "ada@example.com".into();
    panel::author::handle_key(&mut state, key).unwrap();
    state.pending_action
}

#[test]
fn enter_in_the_author_modal_is_documented_as_the_save_it_does() {
    let enter = author_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let ctrl_l = author_action(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(enter.is_some());
    assert_eq!(enter, ctrl_l, "Enter saves what Ctrl+L saves");

    assert_eq!(
        help("Author settings", "Enter"),
        help("Author settings", "Ctrl+L")
    );
    assert_eq!(
        footer_label("Author settings", "Enter"),
        footer_label("Author settings", "Ctrl+L")
    );
}

fn commit_cursor_after(key: KeyEvent) -> usize {
    let mut state = AppState::new();
    state.modal = Modal::Commit;
    state.commit_message = "feat: first line\n\nsecond paragraph".into();
    state.commit_cursor = "feat: first line\n\nsecond".chars().count();
    panel::commit::handle_key(&mut state, key).unwrap();
    state.commit_cursor
}

#[test]
fn ctrl_a_and_ctrl_e_in_the_commit_editor_are_documented_as_what_they_move_to() {
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        commit_cursor_after(ctrl('a')),
        commit_cursor_after(plain(KeyCode::Home))
    );
    assert_eq!(
        commit_cursor_after(ctrl('e')),
        commit_cursor_after(plain(KeyCode::End))
    );

    assert_eq!(
        help("Commit modal", "Ctrl+A / Ctrl+E"),
        help("Commit modal", "Home / End")
    );
}
