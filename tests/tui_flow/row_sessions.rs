//! Agents and terminals started from a row of the repository panel.

use super::common::*;
use lg::panel::environments::menu::{self, RepoAction};

/// Where a started agent lands and how: the checkout, its label, which agent,
/// and whether it runs confined. A profile's confinement is the sandbox choice.
fn started_agent(state: &AppState) -> Option<(String, String, SessionKind, bool)> {
    match &state.pending_action {
        Some(PendingAction::StartAgent {
            path,
            label,
            profile,
        }) => Some((
            path.clone(),
            label.clone(),
            lg::agents::kind(profile),
            profile.confinement == lg::preferences::Confinement::Terrarium,
        )),
        _ => None,
    }
}

/// An agent runs under its own permission harness unless the sandbox is asked
/// for; the terminal never is.
#[test]
fn s_asks_for_a_session_under_the_agents_own_harness_in_the_selected_worktree() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace.worktrees/feat-x", "feat/x"),
    ];

    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::environments::handle_key(&mut state, key(KeyCode::Char('s'))).unwrap();
    panel::agent::handle_key(&mut state, key(KeyCode::Char('c'))).unwrap();

    assert_eq!(
        started_agent(&state),
        Some((
            "/workspace.worktrees/feat-x".into(),
            "feat/x".into(),
            SessionKind::Claude,
            false,
        ))
    );

    // Space in the picker asks for the same agent inside the Terrarium sandbox.
    state.pending_action = None;
    panel::environments::handle_key(&mut state, key(KeyCode::Char('s'))).unwrap();
    panel::agent::handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    panel::agent::handle_key(&mut state, key(KeyCode::Char('c'))).unwrap();
    assert_eq!(
        started_agent(&state),
        Some((
            "/workspace.worktrees/feat-x".into(),
            "feat/x".into(),
            SessionKind::Claude,
            true,
        ))
    );
}

/// Every agent lands in the same checkout the picker was opened on, and a
/// checkout can hold one of each at once.
#[test]
fn each_agent_starts_in_the_checkout_the_picker_was_opened_on() {
    for (pressed, expected) in [
        ('c', SessionKind::Claude),
        ('x', SessionKind::Codex),
        ('p', SessionKind::Pi),
    ] {
        let mut state = AppState::new();
        state.workspace_root = Some("/workspace".into());
        state.repo_root = Some("/workspace".into());
        state.branch = Some("main".into());

        panel::environments::handle_key(&mut state, key(KeyCode::Char('s'))).unwrap();
        panel::agent::handle_key(&mut state, key(KeyCode::Char(pressed))).unwrap();

        assert_eq!(
            started_agent(&state),
            Some(("/workspace".into(), "main".into(), expected, false)),
            "{pressed} should start {}",
            expected.label()
        );
    }
}

/// The terminal key is the session key with a different program on the end:
/// same checkout, and never a sandbox, from the key or the menu alike.
#[test]
fn t_asks_for_a_terminal_in_the_selected_worktree() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace.worktrees/feat-x", "feat/x"),
    ];

    panel::environments::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::environments::handle_key(&mut state, key(KeyCode::Char('t'))).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::StartSession {
            path: "/workspace.worktrees/feat-x".into(),
            label: "feat/x".into(),
            sandboxed: false,
            kind: SessionKind::Terminal,
            prompt: None,
        })
    );

    state.pending_action = None;
    menu::run(&mut state, RepoAction::Terminal);
    assert_eq!(
        state.pending_action,
        Some(PendingAction::StartSession {
            path: "/workspace.worktrees/feat-x".into(),
            label: "feat/x".into(),
            sandboxed: false,
            kind: SessionKind::Terminal,
            prompt: None,
        })
    );
}

#[test]
fn a_session_on_the_root_row_uses_the_checked_out_branch_as_its_name() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.branch = Some("main".into());

    panel::environments::handle_key(&mut state, key(KeyCode::Char('s'))).unwrap();
    panel::agent::handle_key(&mut state, key(KeyCode::Char('c'))).unwrap();

    assert_eq!(
        started_agent(&state),
        Some((
            "/workspace".into(),
            "main".into(),
            SessionKind::Claude,
            false
        ))
    );
}
