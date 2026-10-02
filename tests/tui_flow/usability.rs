//! Errors that can be read, jobs that wait instead of vanishing, filters,
//! confirmations and mouse support in modals.

use super::common::*;

fn headless() -> lg::app::HeadlessApp<TestBackend> {
    lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap()
}

fn footer_row(app: &lg::app::HeadlessApp<TestBackend>) -> String {
    let buf = app.terminal.backend().buffer().clone();
    let row = buf.area.height - 1;
    (0..buf.area.width)
        .map(|col| buf[(col, row)].symbol().to_string())
        .collect()
}

/// A refused push is a dozen lines of git output, and the line that says
/// why is the last. The bar says there is more, `!` shows all of it, and it
/// is still there after the error itself was dismissed.
#[test]
fn bang_opens_the_whole_of_an_error_the_status_bar_cut_short() {
    let mut app = headless();
    app.state.focus = Pane::Files;
    let error = "git push origin failed: To github.com:me/repo.git\n ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs to 'github.com:me/repo.git'\nhint: Updates were rejected because the remote contains work that you do not have locally";
    app.state.set_status(error, true);
    app.render().unwrap();
    assert!(
        footer_row(&app).contains("! details"),
        "{}",
        footer_row(&app)
    );

    // Dismissed, it is still what `!` shows.
    app.send_key(key(KeyCode::Esc)).unwrap();
    app.send_key(key(KeyCode::Char('!'))).unwrap();

    assert_eq!(app.state.modal, Modal::StatusDetails);
    let screen = buffer_text(&app);
    assert!(screen.contains("Updates were rejected"), "{screen}");

    app.send_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.state.modal, Modal::None);
}

/// A diff of `hunks` unstaged hunks to one file, four lines each.
fn many_hunks(hunks: usize) -> String {
    let mut text = String::from(
        "== worktree ==\ndiff --git a/big.txt b/big.txt\nindex 1111111..2222222 100644\n--- a/big.txt\n+++ b/big.txt\n",
    );
    for i in 0..hunks {
        let at = i * 10 + 1;
        text.push_str(&format!(
            "@@ -{at},2 +{at},2 @@\n-old {i}\n+new {i}\n context {i}\n"
        ));
    }
    text
}

fn big_diff_app(text: String) -> lg::app::HeadlessApp<TestBackend> {
    let mut app = headless();
    app.state.files = vec![FileEntry {
        path: "big.txt".into(),
        x: ' ',
        y: 'M',
    }];
    app.state.diff_source = lg::state::DiffSource::File("big.txt".into());
    app.state.diff_view_mode = DiffViewMode::Unified;
    app.state.set_diff_text(text);
    app.state.focus = Pane::Main;
    app.render().unwrap();
    app
}

/// A diff far past the cap shows its start, says how much it left out, and
/// the hunks it shows can still be staged, whole.
#[test]
fn a_huge_diff_is_cut_short_and_its_shown_hunks_still_stage() {
    let mut app = big_diff_app(many_hunks(8_000));

    assert!(
        app.state
            .diff_text
            .contains(lg::state::DIFF_TRUNCATED_MARKER),
        "the cut says so"
    );
    assert!(app.state.diff_text.lines().count() <= 20_001);

    app.send_key(key(KeyCode::Char('G'))).unwrap();
    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("more lines not shown"),
        "the end of the pane says what is missing"
    );

    app.send_key(key(KeyCode::Char(' '))).unwrap();
    match &app.state.pending_action {
        Some(PendingAction::ApplyHunk { hunk, .. }) => {
            assert!(hunk.hunk.is_complete(), "only whole hunks are staged")
        }
        other => panic!("expected a hunk to be staged, got {other:?}"),
    }
}

/// One hunk longer than the cap is cut through, and what is left of it is
/// not offered for staging.
#[test]
fn a_hunk_the_cap_cuts_through_cannot_be_staged() {
    let mut text = String::from(
        "== worktree ==\ndiff --git a/big.txt b/big.txt\nindex 1111111..2222222 100644\n--- a/big.txt\n+++ b/big.txt\n@@ -0,0 +1,30000 @@\n",
    );
    for i in 0..30_000 {
        text.push_str(&format!("+line {i}\n"));
    }
    let mut app = big_diff_app(text);

    app.send_key(key(KeyCode::Char(' '))).unwrap();

    assert!(
        !matches!(
            app.state.pending_action,
            Some(PendingAction::ApplyHunk { .. })
        ),
        "a partial hunk is never applied"
    );
}

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn nested(path: &str) -> NestedRepo {
    NestedRepo {
        path: path.into(),
        branch: Some("main".into()),
        detached_at: None,
        has_changes: false,
        worktree_of: None,
    }
}

/// Expanding a repository reads its branches in the background. The cursor
/// moved on while they were read, and it stays on the row it was moved to
/// when the rows under the expanded repository appear above it.
#[test]
fn expanding_a_repository_reads_its_branches_without_moving_the_cursor() {
    let workspace = tempfile::tempdir().unwrap();
    for name in ["api", "web"] {
        let repo = workspace.path().join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git_in(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git_in(&repo, &["add", "a.txt"]);
        git_in(&repo, &["commit", "-m", "init"]);
    }
    git_in(&workspace.path().join("api"), &["branch", "feature/x"]);

    let mut app = headless();
    app.state.focus = Pane::Status;
    app.state.workspace_root = Some(workspace.path().to_string_lossy().into_owned());
    app.state.nested_repositories = vec![nested("api"), nested("web")];
    app.state.nested_repo_tree_idx = 1;

    app.send_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.state.nested_repo_detail_path.as_deref(), Some("api"));
    app.state.pending_action = None;
    app.send_key(key(KeyCode::Char('j'))).unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.state.nested_detail_job.is_some() && std::time::Instant::now() < deadline {
        lg::panel::environments::poll_nested_repo_detail(&mut app.state);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let names: Vec<&str> = app
        .state
        .nested_repo_branches
        .iter()
        .map(|branch| branch.name.as_str())
        .collect();
    assert!(names.contains(&"feature/x"), "{names:?}");

    // Enter acts on the row the cursor is on: still web.
    app.send_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.state.nested_repo_detail_path.as_deref(), Some("web"));
}

fn start_session(
    app: &mut lg::app::HeadlessApp<TestBackend>,
    script: &str,
    dir: &str,
    kind: SessionKind,
) {
    let spawn = lg::term::Spawn {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: std::env::temp_dir(),
        env: vec![("TERM".into(), "xterm-256color".into())],
        env_remove: Vec::new(),
    };
    let spec = lg::session::SessionSpec {
        label: "feat/x".into(),
        cwd: dir.into(),
        sandboxed: false,
        kind,
        prompt: None,
    };
    let id = app
        .state
        .sessions
        .start_with(spec, &spawn, (24, 80))
        .expect("start session");
    app.state.show_session(id);
}

fn workspace_with_worktree(app: &mut lg::app::HeadlessApp<TestBackend>) {
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    app.state.worktrees = vec![
        Worktree {
            is_main: true,
            ..worktree("/workspace", "main")
        },
        worktree("/workspace.worktrees/feat-x", "feat/x"),
    ];
}

/// `x` on a running agent's row used to kill it on the spot, mid-turn.
#[test]
fn x_on_a_running_agent_asks_before_stopping_it() {
    let mut app = headless();
    workspace_with_worktree(&mut app);
    start_session(
        &mut app,
        "sleep 30",
        "/workspace.worktrees/feat-x",
        SessionKind::Claude,
    );
    app.state.show_diff();
    app.state.pending_action = None;
    app.state.focus = Pane::Status;
    // Row 0 root, row 1 the worktree, row 2 its session.
    app.state.nested_repo_tree_idx = 2;

    app.send_key(key(KeyCode::Char('x'))).unwrap();
    assert_eq!(app.state.modal, Modal::ConfirmDestructive);
    app.send_key(key(KeyCode::Char('n'))).unwrap();
    assert!(!app.state.sessions.is_empty(), "n leaves the agent running");

    app.send_key(key(KeyCode::Char('x'))).unwrap();
    app.send_key(key(KeyCode::Char('y'))).unwrap();
    assert!(app.state.sessions.is_empty(), "y stops it");
}

/// A shell sitting at its prompt has nothing to lose, so it just closes.
#[test]
fn x_on_an_idle_shell_closes_it_without_asking() {
    let mut app = headless();
    workspace_with_worktree(&mut app);
    start_session(
        &mut app,
        "read line",
        "/workspace.worktrees/feat-x",
        SessionKind::Terminal,
    );
    app.state.session_capture = false;
    app.state.focus = Pane::Main;

    app.send_key(key(KeyCode::Char('x'))).unwrap();

    assert_eq!(app.state.modal, Modal::None);
    assert!(app.state.sessions.is_empty());
}

/// "set aside · c reopens it" was said of a message that had just been
/// thrown away, and `c` then asked the model for a new one.
#[test]
fn a_draft_set_aside_with_x_comes_back_on_c() {
    let mut app = headless();
    app.state.workspace_root = Some("/workspace".into());
    app.state.repo_root = Some("/workspace".into());
    app.state.files = vec![FileEntry {
        path: "a.rs".into(),
        x: 'M',
        y: ' ',
    }];
    app.state.commit_drafts = vec![lg::state::CommitDraft {
        dir: "/workspace".into(),
        generation: None,
        text: "feat: the message the model wrote".into(),
        ready: true,
    }];
    app.state.focus = Pane::Status;
    // Row 0 the root, row 1 its draft.
    app.state.nested_repo_tree_idx = 1;

    app.send_key(key(KeyCode::Char('x'))).unwrap();
    assert!(app.state.commit_drafts.is_empty(), "the row is gone");
    app.render().unwrap();
    assert!(!buffer_text(&app).contains("the message the model wrote"));

    app.send_key(key(KeyCode::Char('c'))).unwrap();

    assert_eq!(app.state.modal, Modal::Commit);
    assert_eq!(
        app.state.commit_message,
        "feat: the message the model wrote"
    );
    assert!(
        !matches!(
            app.state.pending_action,
            Some(PendingAction::GenerateMessage)
        ),
        "the kept message is not replaced by a new one"
    );
}

fn branch(name: &str) -> Branch {
    Branch {
        name: name.into(),
        is_current: false,
        upstream: None,
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        behind_main: 0,
        last_commit_unix: None,
    }
}

fn type_keys(app: &mut lg::app::HeadlessApp<TestBackend>, text: &str) {
    for c in text.chars() {
        app.send_key(key(KeyCode::Char(c))).unwrap();
    }
}

/// `/` narrows Branches as it is typed, the title says so, the selection
/// moves among what is left, and what acts on the selection acts on the row
/// highlighted. Typed letters go into the filter, not to lg's own keys.
#[test]
fn slash_filters_branches_and_actions_follow_the_visible_selection() {
    let mut app = headless();
    app.state.branches = ["main", "feature/a", "fix/c", "feature/b"]
        .into_iter()
        .map(branch)
        .collect();
    app.state.focus = Pane::Branches;

    app.send_key(key(KeyCode::Char('/'))).unwrap();
    // `c` would open the commit modal, `f` would fetch.
    type_keys(&mut app, "feat");
    assert_eq!(app.state.modal, Modal::None);
    assert_eq!(app.state.selected_branch_ref(), Some("feature/a"));
    app.send_key(key(KeyCode::Enter)).unwrap();

    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(
        screen.contains("Branches /feat"),
        "the title names the filter"
    );
    assert!(!screen.contains("fix/c"), "{screen}");

    app.send_key(key(KeyCode::Char('j'))).unwrap();
    assert_eq!(app.state.selected_branch_ref(), Some("feature/b"));

    app.send_key(key(KeyCode::Esc)).unwrap();
    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("fix/c"),
        "Esc brings every branch back"
    );
    assert_eq!(
        app.state.selected_branch_ref(),
        Some("feature/b"),
        "and the selection stays on the branch it was on"
    );
}

/// With nothing left, there is no branch to act on, and none is acted on.
#[test]
fn a_filter_that_leaves_nothing_acts_on_nothing() {
    let mut app = headless();
    app.state.branches = ["main", "feature/a"].into_iter().map(branch).collect();
    app.state.focus = Pane::Branches;

    app.send_key(key(KeyCode::Char('/'))).unwrap();
    type_keys(&mut app, "zzz");
    app.send_key(key(KeyCode::Enter)).unwrap();
    app.send_key(key(KeyCode::Char('D'))).unwrap();

    assert_eq!(
        app.state.modal,
        Modal::None,
        "no delete prompt for a hidden branch"
    );
    assert!(app.state.selected_branch_ref().is_none());
}

#[test]
fn slash_filters_commits_by_author_and_revert_takes_the_visible_one() {
    let mut app = headless();
    let commit = |sha: &str, author: &str, subject: &str| Commit {
        sha: sha.into(),
        author: author.into(),
        author_short: author[..2].into(),
        parents: vec!["p".into()],
        is_first_parent: true,
        subject: subject.into(),
    };
    app.state.commits = vec![
        commit("aaaaaaa", "Ada", "first"),
        commit("bbbbbbb", "Bob", "second"),
        commit("ccccccc", "Ada", "third"),
    ];
    app.state.focus = Pane::Commits;

    app.send_key(key(KeyCode::Char('/'))).unwrap();
    type_keys(&mut app, "bob");
    app.send_key(key(KeyCode::Enter)).unwrap();
    app.send_key(key(KeyCode::Char('t'))).unwrap();

    let confirm = app.state.confirm.as_ref().expect("revert asks first");
    assert_eq!(
        confirm.action,
        PendingAction::RevertCommit {
            sha: "bbbbbbb".into()
        }
    );
}

#[test]
fn slash_filters_files_by_path_and_space_stages_the_one_shown() {
    let mut app = headless();
    app.state.files = vec![
        FileEntry {
            path: "src/app.rs".into(),
            x: ' ',
            y: 'M',
        },
        FileEntry {
            path: "docs/guide.md".into(),
            x: ' ',
            y: 'M',
        },
    ];
    app.state.focus = Pane::Files;

    app.send_key(key(KeyCode::Char('/'))).unwrap();
    type_keys(&mut app, "guide");
    app.send_key(key(KeyCode::Enter)).unwrap();
    app.render().unwrap();
    let screen = buffer_text(&app);
    assert!(screen.contains("Files /guide"), "{screen}");
    assert!(!screen.contains("app.rs"), "{screen}");

    // The first row under "(all changes)" is the folder; the file is below it.
    app.send_key(key(KeyCode::Char('j'))).unwrap();
    app.send_key(key(KeyCode::Char(' '))).unwrap();
    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::StagePath("docs/guide.md".into()))
    );
}

/// The palette lists the pane keys of the table and runs one by focusing its
/// pane and pressing it there.
#[test]
fn the_palette_runs_a_pane_key_from_the_table() {
    let mut app = headless();
    app.state.files = vec![FileEntry {
        path: "a.rs".into(),
        x: ' ',
        y: 'M',
    }];
    app.state.focus = Pane::Commits;
    app.state.files_list.idx = 1;

    app.send_key(key(KeyCode::Char(':'))).unwrap();
    type_keys(&mut app, "delete file");
    app.send_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.state.focus, Pane::Files);
    assert_eq!(app.state.modal, Modal::ConfirmDestructive, "d asks first");
}

/// The screen row with `needle` on it.
fn screen_row(app: &lg::app::HeadlessApp<TestBackend>, needle: &str) -> Option<(u16, u16)> {
    let buf = app.terminal.backend().buffer().clone();
    (0..buf.area.height).find_map(|row| {
        let line: String = (0..buf.area.width)
            .map(|col| buf[(col, row)].symbol().to_string())
            .collect();
        line.find(needle)
            .map(|at| (line[..at].chars().count() as u16, row))
    })
}

/// The agent picker is a list: a click picks the row it lands on.
#[test]
fn a_click_selects_an_agent_in_the_picker() {
    let mut app = headless();
    app.state.modal = Modal::Agent;
    app.state.agent_profiles.clear();
    app.state.agent_pick_idx = 0;
    app.render().unwrap();
    let (column, row) = screen_row(&app, "codex").expect("codex is listed");

    app.send_mouse(left_click(column, row)).unwrap();

    assert_eq!(app.state.agent_pick_idx, 1);
    assert_eq!(app.state.modal, Modal::Agent, "one click only selects");
}

/// The confirm prompt's buttons do what their keys do.
#[test]
fn clicking_the_confirm_buttons_answers_the_prompt() {
    let mut app = headless();
    app.state.confirm_action(
        "Stage",
        "Stage everything?",
        "every change",
        PendingAction::StageAll,
    );
    app.render().unwrap();
    let (column, row) = screen_row(&app, "n/Esc cancel").expect("the cancel button");
    app.send_mouse(left_click(column + 1, row)).unwrap();
    assert_eq!(app.state.modal, Modal::None);
    assert_eq!(app.state.pending_action, None, "cancelled");

    app.state.confirm_action(
        "Stage",
        "Stage everything?",
        "every change",
        PendingAction::StageAll,
    );
    app.render().unwrap();
    let (column, row) = screen_row(&app, "y confirm").expect("the confirm button");
    app.send_mouse(left_click(column + 2, row)).unwrap();
    assert_eq!(app.state.pending_action, Some(PendingAction::StageAll));
}

/// The palette is a long list: the wheel moves through it and a click picks
/// the entry under it.
#[test]
fn the_palette_takes_the_wheel_and_clicks() {
    let mut app = headless();
    app.send_key(key(KeyCode::Char(':'))).unwrap();
    app.render().unwrap();
    let (column, row) = screen_row(&app, "Help \u{2014} all shortcuts").expect("listed");

    app.send_mouse(left_click(column, row)).unwrap();
    app.send_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.state.modal, Modal::Help, "the clicked entry ran");

    app.send_key(key(KeyCode::Char('q'))).unwrap();
    app.send_key(key(KeyCode::Char(':'))).unwrap();
    let wheel = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    app.send_mouse(wheel).unwrap();
    assert!(
        app.state.commands.selected > 0,
        "the wheel moved the selection"
    );
}
