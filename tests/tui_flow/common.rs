pub use lg::{
    git::{
        AssistedReview, Branch, BranchReleaseStatus, Commit, FileEntry, ReleaseTargetStatus,
        RemoteBranch, ReviewNode,
    },
    panel,
    state::{
        AppState, AuthorField, BranchView, Modal, Pane, PendingAction, ReleaseStatusJob, TreeKind,
        WorkflowJob, build_tree_rows,
    },
};
pub use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    style::{Color, Modifier},
};
pub use std::{collections::HashSet, sync::mpsc};

pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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
            last_commit_unix: None,
        },
        Branch {
            name: "release/next".into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
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
