//! The flow modal while a workflow runs, the prompts before one, and the
//! conflict modal it can stop on.

use super::common::*;
use lg::panel::environments::menu::{self, RepoAction};

#[test]
fn flow_modal_renders_running_workflow_steps() {
    let mut state = AppState::new();
    let (_tx, rx) = std::sync::mpsc::channel();
    state.workflow_job = Some(WorkflowJob {
        rx,
        handle: None,
        spinner: 1,
        label: "Release current branch into test".into(),
        steps: vec![
            "push feature/demo".into(),
            "merge origin/feature/demo".into(),
            "push test".into(),
        ],
        current_step: Some(1),
        flow: None,
    });

    // The marker's frame follows the animation clock, not the job.
    state.animation_tick = 1;
    let backend = TestBackend::new(90, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::flow::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(
        text.contains("[x] push feature/demo"),
        "missing completed step: {text}"
    );
    assert!(
        text.contains(">/< merge origin/feature/demo"),
        "missing active step marker: {text}"
    );
    assert!(
        text.contains("[ ] push test"),
        "missing pending step: {text}"
    );
}

/// A list of eleven git operations does not say what is being done to the
/// repository, so a running flow draws the same graph the menu drew, with the
/// marker moved to the step in progress.
#[test]
fn flow_modal_draws_the_graph_beside_the_running_steps() {
    let mut state = AppState::new();
    state.branch = Some("feature/demo".into());
    state.release_branches = ReleaseBranches::new(Some("develop".into()), Some("test".into()));
    let (_tx, rx) = mpsc::channel();
    state.workflow_job = Some(WorkflowJob {
        rx,
        handle: None,
        spinner: 0,
        label: "Release current branch into test".into(),
        steps: vec![
            "stash current changes".into(),
            "create safety backup".into(),
            "push feature/demo".into(),
            "fetch origin".into(),
            "sync test from origin/test".into(),
            "checkout test".into(),
            "merge origin/main".into(),
            "merge origin/feature/demo".into(),
            "push HEAD to origin/test".into(),
            "checkout feature/demo".into(),
            "restore stashed changes".into(),
        ],
        // The flow is merging main into the deploy branch, which the graph draws.
        current_step: Some(6),
        flow: Some(FlowRun {
            action: FlowAction::ReleaseTest,
            branch: "feature/demo".into(),
            target: Some("test".into()),
            input: None,
        }),
    });

    let backend = TestBackend::new(90, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::flow::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(
        text.contains(">|< merge origin/main"),
        "the step in progress should still be marked in the list: {text}"
    );
    assert!(text.contains("Where it is"), "missing graph pane: {text}");
    for lane in ["origin/main", "origin/test"] {
        assert!(
            text.contains(lane),
            "graph is missing the {lane} lane: {text}"
        );
    }
    assert!(
        text.contains('\u{25c9}'),
        "the marker should be on the graph: {text}"
    );
}

#[test]
fn conflict_modal_exposes_inline_and_external_resolution() {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.conflict.files = vec!["src/conflict.rs".into()];
    state.conflict.log = "merge failed".into();

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::conflict::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("Git conflict detected"), "{text}");
    assert!(
        text.contains("Resolve here with Enter"),
        "modal should expose inline resolution: {text}"
    );
    assert!(
        text.contains("v validate"),
        "modal should expose validate action: {text}"
    );
    assert!(
        text.contains("open"),
        "modal should expose open action: {text}"
    );
    assert!(!text.contains("ours/theirs/both"), "{text}");
    assert!(!text.contains("LLM"), "{text}");
    assert!(!text.contains("stage + continue"), "{text}");
}

#[test]
fn the_conflict_modal_separates_files_to_read_from_files_to_resolve() {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.conflict.files = vec!["src/easy.rs".into(), "src/hard.rs".into()];
    state.conflict.resolved.insert("src/easy.rs".into());

    let backend = TestBackend::new(140, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::conflict::render(&state, frame.area(), frame);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("src/easy.rs"), "{text}");
    assert!(text.contains("src/hard.rs"), "{text}");
    assert!(
        text.contains("resolved in lg"),
        "the modal has to say a file was rewritten before v commits it: {text}"
    );
    assert!(
        text.contains('l'),
        "the local pass has to be offered somewhere: {text}"
    );
}

#[test]
fn only_the_irreversible_prompt_warns_that_it_cannot_be_undone() {
    let screen_after = |code| {
        let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
        app.state.workspace_root = Some("/workspace".into());
        app.state.repo_root = Some("/workspace".into());
        app.state.worktrees = vec![
            Worktree {
                is_main: true,
                ..worktree("/workspace", "main")
            },
            worktree("/workspace.worktrees/feat-x", "feat/x"),
        ];
        app.state.focus = Pane::Status;
        app.state.nested_repo_tree_idx = 1;
        menu::run(&mut app.state, code);
        app.render().unwrap();
        buffer_text(&app)
    };

    let landing = screen_after(RepoAction::LandWorktree);
    assert!(
        landing.contains("This cannot be undone."),
        "landing deletes a branch off the remote: {landing}"
    );

    let coming_home = screen_after(RepoAction::BranchHome);
    assert!(
        !coming_home.contains("cannot be undone"),
        "moving a branch between checkouts is not a one-way door: {coming_home}"
    );
    assert!(
        coming_home.contains("y confirm"),
        "it is still confirmed, just not warned about: {coming_home}"
    );
}

#[test]
fn a_running_operation_says_which_step_it_is_on() {
    let mut state = AppState::new();
    let (tx, rx) = mpsc::channel();
    state.operation_job = Some(lg::state::OperationJob {
        rx,
        handle: None,
        spinner: 0,
        label: "landing worktree",
        kind: lg::state::OperationKind::WorkingTree,
        step: Some("pushing main".into()),
    });
    state.set_status("landing worktree\u{2026}", false);
    drop(tx);

    let backend = TestBackend::new(60, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| panel::status::render(&state, frame.area(), frame, false))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(
        text.contains("landing worktree: pushing main"),
        "the spinner should say what it is doing: {text}"
    );
}

/// Resolving a merge is reading two versions and deciding what the result
/// should be — work lg does not do itself, so the modal offers something that
/// can, in the checkout the conflict is in.
#[test]
fn c_hands_the_conflict_to_a_claude_session_in_the_checkout() {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.repo_root = Some("/workspace/alv-no".into());
    state.branch = Some("feature/send-cv".into());
    state.conflict.files = vec!["src/a.rs".into(), "src/b.rs".into()];

    panel::conflict::handle_key(&mut state, key(KeyCode::Char('c'))).unwrap();

    let Some(PendingAction::StartSession {
        path,
        label,
        sandboxed,
        kind,
        prompt,
    }) = state.pending_action.clone()
    else {
        panic!("c should ask for a session: {:?}", state.pending_action);
    };
    assert_eq!(path, "/workspace/alv-no");
    assert_eq!(label, "feature/send-cv");
    assert!(
        !sandboxed,
        "c runs the agent under its own harness, as elsewhere in lg; C sandboxes it"
    );
    assert_eq!(kind, SessionKind::Claude);

    let prompt = prompt.expect("the session should open on the conflict");
    assert!(
        prompt.contains("src/a.rs") && prompt.contains("src/b.rs"),
        "the prompt should name the conflicted files: {prompt}"
    );
    // lg settles the merge either way now, so the session must not be told to
    // leave it uncommitted.
    assert!(
        !prompt.to_lowercase().contains("do not commit"),
        "committing is the session's choice: {prompt}"
    );
    assert_eq!(
        state.modal,
        Modal::None,
        "the modal steps aside so the session can be seen"
    );

    // Shift is the Terrarium-sandboxed one.
    state.pending_action = None;
    state.modal = Modal::Conflict;
    panel::conflict::handle_key(&mut state, key(KeyCode::Char('C'))).unwrap();
    let Some(PendingAction::StartSession { sandboxed, .. }) = state.pending_action.clone() else {
        panic!("C should ask for a session too");
    };
    assert!(sandboxed);
}

/// Leaving the modal — to resolve the conflict in a session, or by mistake —
/// must not strand the flow that is still waiting on it.
#[test]
fn f_reopens_a_conflict_that_is_still_unfinished() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(100, 30)).unwrap();
    app.state.repo_root = Some("/workspace".into());
    app.state.conflict.files = vec!["src/a.rs".into()];
    app.state.modal = Modal::Conflict;

    app.send_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.state.modal, Modal::None);

    app.send_key(key(KeyCode::Char('F'))).unwrap();
    assert_eq!(
        app.state.modal,
        Modal::Conflict,
        "the flow key should go back to the conflict holding the flow up"
    );

    // With nothing conflicted, F is the branch action menu it has always been.
    app.state.conflict.files.clear();
    app.state.modal = Modal::None;
    app.send_key(key(KeyCode::Char('F'))).unwrap();
    assert_ne!(app.state.modal, Modal::Conflict);
}

/// A conflict stopped a sync part-way through its branches, so settling it is
/// where finishing starts: the branch just merged is still unpushed and the
/// branches after it are untouched.
fn stopped_sync() -> AppState {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.conflict.files = vec!["src/a.rs".into()];
    state.conflict.followup = Some(lg::state::ConflictFollowup::for_branch_sync(Some(
        "main".to_string(),
    )));
    state
}

/// The sync stashed the work on the branch it was launched from and the
/// conflict left the checkout on another branch, so settling it has to come
/// back before the stash does.
#[test]
fn a_stopped_sync_comes_back_to_the_branch_it_was_launched_from() {
    let state = stopped_sync();

    assert_eq!(
        state
            .conflict
            .followup
            .as_ref()
            .and_then(|followup| followup.return_branch.as_deref()),
        Some("main"),
        "the work is stashed on the branch the sync started on, not the conflicted one"
    );
}

#[test]
fn continuing_a_conflict_resumes_the_sync_that_stopped_on_it() {
    let mut state = stopped_sync();

    state.settle_conflict(true);

    assert_eq!(
        state.pending_action,
        Some(PendingAction::MergeMainAllBranches),
        "the sync should run again, which is what pushes the branch just merged"
    );
    assert_eq!(state.modal, Modal::None);
    assert!(state.conflict.files.is_empty());
    assert_eq!(state.conflict.followup, None, "the followup is spent");
}

/// Aborting is giving up on the flow, so it must not start it again.
#[test]
fn aborting_a_conflict_does_not_resume_anything() {
    let mut state = stopped_sync();

    state.settle_conflict(false);

    assert_eq!(
        state.pending_action, None,
        "abort must not queue the flow it is giving up on"
    );
    assert_eq!(state.modal, Modal::None);
    assert_eq!(state.conflict.followup, None);
}

/// A conflict from something with nothing left to do settles without queueing
/// anything at all.
#[test]
fn continuing_a_conflict_with_nothing_to_resume_queues_nothing() {
    let mut state = AppState::new();
    state.modal = Modal::Conflict;
    state.conflict.files = vec!["src/a.rs".into()];
    state.conflict.followup = Some(lg::state::ConflictFollowup::default());

    state.settle_conflict(true);

    assert_eq!(state.pending_action, None);
    assert_eq!(state.modal, Modal::None);
}
