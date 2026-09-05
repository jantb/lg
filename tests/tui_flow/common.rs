pub use lg::{
    git::{
        AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, NestedRepo,
        ReleaseBranches, ReleaseTargetStatus, RemoteBranch, ReviewNode, Worktree,
    },
    panel,
    session::SessionKind,
    state::{
        AppState, AuthorField, BranchView, ConflictResolveJob, DiffViewMode, FlowAction, FlowRun,
        Modal, Pane, PendingAction, ReleaseStatusJob, RepoTarget, ReviewChatRole,
        ReviewStyleFinding, ReviewStyleSeverity, TreeKind, WorkflowJob, build_tree_rows,
    },
};
pub use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    style::{Color, Modifier},
};
pub use std::{collections::HashSet, sync::mpsc};

/// The whole help overlay as text, scrolled from top to bottom, so asserting
/// that a binding is documented does not depend on where it happens to sit.
pub fn help_text(app: &mut lg::app::HeadlessApp<TestBackend>, area: Rect) -> String {
    app.state.help_offset = 0;
    let mut text = String::new();
    loop {
        app.render().expect("render help");
        text.push_str(&buffer_text(app));
        let before = app.state.help_offset;
        panel::help::scroll(&mut app.state, area, true, 1);
        if app.state.help_offset == before {
            return text;
        }
    }
}

/// A linked worktree with nothing unusual about it. Callers override the odd
/// field they care about: `Worktree { is_main: true, ..worktree(path, branch) }`.
pub fn worktree(path: &str, branch: &str) -> Worktree {
    Worktree {
        path: path.into(),
        branch: Some(branch.into()),
        head: "0123456789abcdef0123456789abcdef01234567".into(),
        is_main: false,
        bare: false,
        locked: None,
        prunable: None,
        has_changes: false,
        unmerged: Some(0),
    }
}

pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn left_click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

pub fn left_drag(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

pub fn make_state_with_files() -> AppState {
    let mut s = AppState::new();
    s.files = vec![
        FileEntry {
            path: "a.rs".into(),
            x: ' ',
            y: 'M',
        },
        FileEntry {
            path: "b.rs".into(),
            x: 'A',
            y: ' ',
        },
        FileEntry {
            path: "c.rs".into(),
            x: '?',
            y: '?',
        },
    ];
    s
}

pub fn add_flow_branches(state: &mut AppState) {
    state.branches = vec![
        Branch {
            name: "develop".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
        Branch {
            name: "test".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        },
    ];
}

pub fn buffer_text(app: &lg::app::HeadlessApp<TestBackend>) -> String {
    let buf = app.terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }
    text
}
