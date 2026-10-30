//! The status, files and current branch panels, and the workspace pane of a
//! lone project.

use super::common::*;

#[test]
fn files_panel_renders_xy_codes_with_color() {
    let state = make_state_with_files();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::files::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();

    // Find the 'M' cell (y-side modification for a.rs) and check it's yellow.
    let found_yellow_m = (0..buf.area.height).any(|row| {
        (0..buf.area.width).any(|col| {
            let cell = &buf[(col, row)];
            cell.symbol() == "M" && cell.style().fg == Some(Color::Yellow)
        })
    });
    assert!(
        found_yellow_m,
        "expected a yellow 'M' cell for modified file"
    );

    // Untracked files render as an unstaged marker rather than raw "??".
    let found_unstaged = (0..buf.area.height).any(|row| {
        (0..buf.area.width.saturating_sub("unstaged".len() as u16)).any(|col| {
            (0.."unstaged".len()).all(|offset| {
                let cell = &buf[((col + offset as u16), row)];
                cell.symbol() == &"unstaged"[offset..offset + 1]
                    && cell.style().fg == Some(Color::DarkGray)
            })
        })
    });
    assert!(
        found_unstaged,
        "expected a grey unstaged marker for untracked file"
    );
}

#[test]
fn status_panel_renders_change_counts() {
    let state = make_state_with_files();
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::status::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("3 files"), "missing total file count: {text}");
    assert!(text.contains("S1"), "missing staged count: {text}");
    assert!(text.contains("U1"), "missing unstaged count: {text}");
    assert!(text.contains("?1"), "missing untracked count: {text}");
}

#[test]
fn status_panel_shows_active_generation() {
    let mut state = make_state_with_files();
    let (_tx, rx) = std::sync::mpsc::channel::<lg::state::GenMsg>();
    state.commit_drafts = vec![lg::state::CommitDraft {
        dir: state.commit_dir(),
        generation: Some(lg::state::Generation::new(
            rx,
            std::thread::spawn(|| {}),
            Default::default(),
        )),
        text: String::new(),
        ready: false,
    }];

    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            panel::status::render(&state, frame.area(), frame, false);
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
        text.contains("generating"),
        "expected active generation cue: {text}"
    );
}

/// A project on its own is still a checkout you start sessions in. The pane
/// used to be hidden outright for one, which left nowhere to press s.
#[test]
fn a_lone_project_still_gets_the_workspace_pane() {
    let mut state = AppState::new();
    state.repo_root = Some("/workspace".into());

    assert!(
        state.environments_visible(),
        "a single checkout has sessions to list and start"
    );
}

/// And the pane shows the tree rather than only the deployment box, so the
/// checkout and whatever is running in it can be selected.
#[test]
fn a_lone_project_lists_its_checkout_and_its_sessions() {
    let mut state = AppState::new();
    state.workspace_root = Some("/workspace".into());
    state.repo_root = Some("/workspace".into());
    state.branch = Some("main".into());
    add_flow_branches(&mut state);
    let spawn = lg::term::Spawn {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 30".into()],
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        env_remove: Vec::new(),
    };
    state
        .sessions
        .start_with(
            lg::session::SessionSpec {
                label: "main".into(),
                cwd: "/workspace".into(),
                sandboxed: false,
                kind: SessionKind::Codex,
                prompt: None,
            },
            &spawn,
            (24, 80),
        )
        .expect("start session");

    let backend = TestBackend::new(90, 14);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, true);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("workspace"), "the checkout row: {text}");
    assert!(text.contains("codex"), "the session running in it: {text}");
    assert!(
        text.contains("Branch inclusion"),
        "and the deployment box still fits underneath: {text}"
    );
}

#[test]
fn current_branch_panel_renders_environment_history() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("feature/released".into());
    state.current_branch_releases = BranchReleaseStatus {
        environments: Default::default(),
        main: None,
        develop: Some(ReleaseTargetStatus {
            released_at: "2026-04-29 14:20".into(),
            missing_commits: 2,
        }),
        test: Some(ReleaseTargetStatus {
            released_at: "2026-04-29 14:25".into(),
            missing_commits: 0,
        }),
    };

    let backend = TestBackend::new(90, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, false);
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
        text.contains("Branch inclusion"),
        "missing deployment status panel: {text}"
    );
    assert!(
        text.contains("feature/released"),
        "missing branch name: {text}"
    );
    assert!(text.contains("develop"), "missing develop badge: {text}");
    assert!(text.contains("test"), "missing test badge: {text}");
    assert!(
        text.contains("2026-04-29 14:20"),
        "missing release timestamp: {text}"
    );
    assert!(text.contains("+2"), "missing pending count: {text}");
}

#[test]
fn deployment_status_lists_only_the_deploy_branches_the_repo_has() {
    let mut state = AppState::new();
    state.release_branches = ReleaseBranches::new(None, Some("test".into()));
    state.branch = Some("feature/box-adjustments".into());
    state.current_branch_releases = BranchReleaseStatus {
        environments: Default::default(),
        main: Some(ReleaseTargetStatus {
            released_at: String::new(),
            missing_commits: 3,
        }),
        develop: None,
        test: Some(ReleaseTargetStatus {
            released_at: "2026-04-29 14:25".into(),
            missing_commits: 0,
        }),
    };

    let backend = TestBackend::new(90, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, false);
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
        text.contains("Branch inclusion"),
        "missing deployment status panel: {text}"
    );
    assert!(text.contains("test"), "missing test badge: {text}");
    assert!(
        text.contains("2026-04-29 14:25"),
        "missing test release timestamp: {text}"
    );
    assert!(
        !text.contains("develop"),
        "a repo without a develop branch should not get a develop row: {text}"
    );
}

#[test]
fn current_branch_panel_shows_deployment_status_loading() {
    let mut state = AppState::new();
    add_flow_branches(&mut state);
    state.branch = Some("feature/loading".into());
    let (_tx, rx) = mpsc::channel();
    state.release_status_job = Some(ReleaseStatusJob {
        rx,
        handle: None,
        spinner: 0,
        branch: "feature/loading".into(),
    });

    let backend = TestBackend::new(90, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, false);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let mut text = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            text.push_str(buf[(col, row)].symbol());
        }
    }

    assert!(text.contains("checking"), "missing loading state: {text}");
}

#[test]
fn current_branch_panel_hides_environment_history_without_release_branches() {
    let state = AppState::new();
    let backend = TestBackend::new(90, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            panel::environments::render(&state, frame.area(), frame, false);
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
        !text.contains("Branch inclusion"),
        "environment box should be hidden: {text}"
    );
    assert!(!text.contains("dev"), "dev status should be hidden: {text}");
    assert!(
        !text.contains("test"),
        "test status should be hidden: {text}"
    );
}
