use super::common::*;
use lg::git::hunk::{HunkOp, HunkSide};

// ── Hunks in the diff pane ───────────────────────────────────────────────────

/// What the diff pane shows for a file with one staged hunk and two unstaged.
const FILE_DIFF: &str = r#"== staged (--cached) ==
diff --git a/notes.txt b/notes.txt
index 1111111..2222222 100644
--- a/notes.txt
+++ b/notes.txt
@@ -1,3 +1,3 @@
-line 1
+line one
 line 2
 line 3

== worktree ==
diff --git a/notes.txt b/notes.txt
index 2222222..3333333 100644
--- a/notes.txt
+++ b/notes.txt
@@ -10,3 +10,3 @@ fn ten()
 line 9
-line 10
+line ten
 line 11
@@ -20,3 +20,3 @@ fn twenty()
 line 19
-line 20
+line twenty
 line 21
"#;

fn diff_pane_app() -> lg::app::HeadlessApp<TestBackend> {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    app.state.files = vec![FileEntry {
        path: "notes.txt".into(),
        x: 'M',
        y: 'M',
    }];
    app.state.diff_source = lg::state::DiffSource::File("notes.txt".into());
    app.state.diff_view_mode = DiffViewMode::Unified;
    app.state.set_diff_text(FILE_DIFF.to_string());
    app.state.focus = Pane::Main;
    app.render().unwrap();
    app
}

fn pending_hunk(state: &AppState) -> Option<(HunkOp, HunkSide, String)> {
    match &state.pending_action {
        Some(PendingAction::ApplyHunk { op, hunk }) => {
            Some((*op, hunk.side, hunk.hunk.header.clone()))
        }
        _ => None,
    }
}

#[test]
fn space_on_an_unstaged_hunk_stages_that_hunk() {
    let mut app = diff_pane_app();

    app.send_key(key(KeyCode::Char(' '))).unwrap();

    let (op, side, header) = pending_hunk(&app.state).expect("a hunk is applied");
    assert_eq!((op, side), (HunkOp::Stage, HunkSide::Worktree));
    assert!(header.starts_with("@@ -10,3"), "{header}");
}

#[test]
fn moving_to_the_next_hunk_moves_what_space_stages() {
    let mut app = diff_pane_app();

    app.send_key(key(KeyCode::Char(']'))).unwrap();
    app.render().unwrap();
    assert!(
        buffer_text(&app).contains("hunk 3/3 unstaged"),
        "the title says which hunk is current"
    );
    app.send_key(key(KeyCode::Char(' '))).unwrap();

    let (op, _, header) = pending_hunk(&app.state).expect("a hunk is applied");
    assert_eq!(op, HunkOp::Stage);
    assert!(header.starts_with("@@ -20,3"), "{header}");
}

#[test]
fn space_on_a_staged_hunk_unstages_it() {
    let mut app = diff_pane_app();

    // From the first unstaged hunk, one back is the staged one above it.
    app.send_key(key(KeyCode::Char('['))).unwrap();
    app.send_key(key(KeyCode::Char(' '))).unwrap();

    let (op, side, _) = pending_hunk(&app.state).expect("a hunk is applied");
    assert_eq!((op, side), (HunkOp::Unstage, HunkSide::Staged));
}

#[test]
fn discarding_a_hunk_asks_first_and_runs_only_once_confirmed() {
    let mut app = diff_pane_app();

    app.send_key(key(KeyCode::Char('d'))).unwrap();

    assert_eq!(app.state.modal, Modal::ConfirmDestructive);
    assert!(
        app.state.pending_action.is_none(),
        "nothing runs before yes"
    );
    let prompt = app.state.confirm.clone().expect("confirm prompt");
    assert!(prompt.question.contains("notes.txt"), "{}", prompt.question);
    app.send_key(key(KeyCode::Char('y'))).unwrap();
    let (op, side, _) = pending_hunk(&app.state).expect("confirmed discard");
    assert_eq!((op, side), (HunkOp::Discard, HunkSide::Worktree));
}

#[test]
fn a_staged_hunk_is_not_discarded() {
    let mut app = diff_pane_app();
    app.send_key(key(KeyCode::Char('['))).unwrap();

    app.send_key(key(KeyCode::Char('d'))).unwrap();

    assert_ne!(app.state.modal, Modal::ConfirmDestructive);
    assert!(app.state.pending_action.is_none());
}

#[test]
fn a_commits_hunks_can_be_walked_but_not_staged() {
    let mut app = diff_pane_app();
    app.state.diff_source = lg::state::DiffSource::Commit("abc1234".into());

    app.send_key(key(KeyCode::Char(']'))).unwrap();
    app.send_key(key(KeyCode::Char(' '))).unwrap();
    app.send_key(key(KeyCode::Char('d'))).unwrap();

    assert!(app.state.pending_action.is_none());
    assert_ne!(app.state.modal, Modal::ConfirmDestructive);
    assert!(
        app.state
            .status
            .as_ref()
            .is_some_and(|status| status.text.contains("history")),
        "{:?}",
        app.state.status
    );
}

#[test]
fn the_current_hunk_follows_the_scroll() {
    let mut app = diff_pane_app();
    app.state.diff_viewport_height = 4;

    app.send_key(key(KeyCode::Char('G'))).unwrap();
    app.send_key(key(KeyCode::Char(' '))).unwrap();

    let (_, _, header) = pending_hunk(&app.state).expect("a hunk is applied");
    assert!(
        header.starts_with("@@ -20,3"),
        "the hunk on screen, not one scrolled away: {header}"
    );
}

#[test]
fn the_diff_pane_help_and_footer_list_the_hunk_keys() {
    let mut app = diff_pane_app();
    app.terminal.backend_mut().resize(240, 40);
    app.render().unwrap();
    let footer = buffer_text(&app);
    assert!(footer.contains("stage hunk"), "{footer}");
    assert!(footer.contains("discard"), "{footer}");

    app.state.modal = Modal::Help;
    let area = Rect::new(0, 0, 120, 40);
    let help = help_text(&mut app, area);
    assert!(help.contains("Stage unstaged hunk / unstage staged one"));
    assert!(help.contains("Discard an unstaged hunk (confirms)"));
    assert!(help.contains("Staged hunks above, unstaged below"));
    assert!(help.contains("Log: cherry-pick commit here (confirms)"));
}

// ── Cherry-pick from a branch log ────────────────────────────────────────────

const BRANCH_LOG: &str = r#"* commit 1a2b3c4 (feature/other)
| Author: A <a@example.com>
| Date:   2 days ago
|
|     second feature commit
|
* commit 5d6e7f8
  Author: A <a@example.com>
  Date:   3 days ago

      first feature commit
"#;

fn log_app() -> lg::app::HeadlessApp<TestBackend> {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 40)).unwrap();
    app.state.branch = Some("main".into());
    app.state.diff_source = lg::state::DiffSource::Branch("feature/other".into());
    app.state.set_diff_text(BRANCH_LOG.to_string());
    app.state.focus = Pane::Main;
    app.render().unwrap();
    app
}

#[test]
fn cherry_picking_from_a_branch_log_confirms_the_commit_under_the_cursor() {
    let mut app = log_app();

    app.send_key(key(KeyCode::Char(']'))).unwrap();
    app.send_key(key(KeyCode::Char('C'))).unwrap();

    assert_eq!(app.state.modal, Modal::ConfirmDestructive);
    let prompt = app.state.confirm.clone().expect("confirm prompt");
    assert!(prompt.question.contains("5d6e7f8"), "{}", prompt.question);
    assert!(prompt.question.contains("main"), "{}", prompt.question);
    assert!(
        prompt.detail.contains("first feature commit"),
        "{}",
        prompt.detail
    );
    app.send_key(key(KeyCode::Char('y'))).unwrap();
    assert_eq!(
        app.state.pending_action,
        Some(PendingAction::CherryPick {
            sha: "5d6e7f8".into()
        })
    );
}

#[test]
fn the_checked_out_branchs_own_log_offers_no_cherry_pick() {
    let mut app = log_app();
    app.state.branch = Some("feature/other".into());

    app.send_key(key(KeyCode::Char('C'))).unwrap();

    assert_ne!(app.state.modal, Modal::ConfirmDestructive);
    assert!(app.state.pending_action.is_none());
}

// ── Commits pane: copy and revert ────────────────────────────────────────────

fn commits_state() -> AppState {
    let mut state = AppState::new();
    state.branch = Some("main".into());
    state.commits = vec![
        Commit {
            sha: "abc1234".into(),
            author: "a@b.c".into(),
            author_short: "ab".into(),
            subject: "fix the thing".into(),
            parents: vec!["def5678".into()],
            is_first_parent: true,
        },
        Commit {
            sha: "fedcba9".into(),
            author: "a@b.c".into(),
            author_short: "ab".into(),
            subject: "Merge branch 'x'".into(),
            parents: vec!["1111111".into(), "2222222".into()],
            is_first_parent: true,
        },
    ];
    state.focus = Pane::Commits;
    state
}

#[test]
fn y_copies_the_selected_commits_sha() {
    let mut state = commits_state();

    panel::commits::handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();

    match state.pending_action {
        Some(PendingAction::CopyToClipboard { label, text }) => {
            assert_eq!(label, "abc1234");
            assert!(text.starts_with("abc1234"), "{text}");
        }
        other => panic!("expected a copy, got {other:?}"),
    }
}

#[test]
fn reverting_a_commit_asks_first() {
    let mut state = commits_state();

    panel::commits::handle_key(&mut state, key(KeyCode::Char('t'))).unwrap();

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert!(state.pending_action.is_none());
    let prompt = state.confirm.clone().expect("confirm prompt");
    assert_eq!(
        prompt.action,
        PendingAction::RevertCommit {
            sha: "abc1234".into()
        }
    );
    assert!(prompt.detail.contains("fix the thing"));
}

#[test]
fn a_merge_commit_is_not_offered_for_revert() {
    let mut state = commits_state();
    state.commits_list.idx = 1;

    panel::commits::handle_key(&mut state, key(KeyCode::Char('t'))).unwrap();

    assert_ne!(state.modal, Modal::ConfirmDestructive);
    assert!(state.pending_action.is_none());
}

// ── Stash modal ──────────────────────────────────────────────────────────────

fn stash_state() -> AppState {
    let mut state = make_state_with_files();
    state.stash.entries = vec![
        lg::git::StashEntry {
            index: 0,
            sha: "1111111111111111111111111111111111111111".into(),
            subject: "On main: half done".into(),
            age: "2 minutes ago".into(),
            auto: false,
        },
        lg::git::StashEntry {
            index: 1,
            sha: "2222222222222222222222222222222222222222".into(),
            subject: "On main: lg: auto-stash before pull".into(),
            age: "3 days ago".into(),
            auto: true,
        },
    ];
    state.modal = Modal::Stash;
    state
}

#[test]
fn the_stash_list_marks_the_stashes_lg_made_itself() {
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state = stash_state();
    app.render().unwrap();

    let text = buffer_text(&app);
    assert!(text.contains("half done"), "{text}");
    assert!(text.contains("lg auto-stash"), "{text}");
}

#[test]
fn stash_keys_apply_pop_and_drop_the_selected_entry() {
    let mut state = stash_state();
    let second = state.stash.entries[1].sha.clone();

    panel::stash::handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    panel::stash::handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    assert_eq!(
        state.pending_action.take(),
        Some(PendingAction::StashApply {
            sha: second.clone()
        })
    );
    panel::stash::handle_key(&mut state, key(KeyCode::Char('g'))).unwrap();
    assert_eq!(
        state.pending_action.take(),
        Some(PendingAction::StashPop {
            sha: second.clone()
        })
    );

    panel::stash::handle_key(&mut state, key(KeyCode::Char('d'))).unwrap();
    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert!(state.pending_action.is_none(), "dropping waits for yes");
    let prompt = state.confirm.clone().expect("confirm prompt");
    assert_eq!(prompt.action, PendingAction::StashDrop { sha: second });
    assert!(
        prompt.detail.contains("lg made this one"),
        "an auto-stash says so before it is dropped: {}",
        prompt.detail
    );
}

#[test]
fn a_new_stash_takes_the_typed_message() {
    let mut state = stash_state();

    panel::stash::handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();
    for c in "wip".chars() {
        panel::stash::handle_key(&mut state, key(KeyCode::Char(c))).unwrap();
    }
    panel::stash::handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::StashPush {
            message: "wip".into()
        })
    );
}

// ── Against a real repository ────────────────────────────────────────────────

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "first commit"]);
    dir
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn amend_puts_the_last_message_in_the_editor_and_asks_before_replacing_it() {
    let dir = repo();
    let mut state = AppState::new();
    state.modal = Modal::Commit;
    state.commit_message = "a new commit".into();

    lg::git::with_repo(dir.path(), || {
        panel::commit::handle_key(&mut state, ctrl('t')).unwrap();
        assert!(state.commit_amend);
        assert_eq!(state.commit_message, "first commit");
        panel::commit::handle_key(&mut state, ctrl('s')).unwrap();
    });

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    let prompt = state.confirm.clone().expect("confirm prompt");
    assert_eq!(prompt.action, PendingAction::AmendCommit);
    assert!(
        prompt.detail.contains("not been pushed"),
        "{}",
        prompt.detail
    );

    // Turning amend off gives the message being written back.
    state.modal = Modal::Commit;
    panel::commit::handle_key(&mut state, ctrl('t')).unwrap();
    assert!(!state.commit_amend);
    assert_eq!(state.commit_message, "a new commit");
}

#[test]
fn amending_a_pushed_commit_warns_that_a_force_push_will_be_needed() {
    let dir = repo();
    let bare = tempfile::tempdir().expect("bare");
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git(dir.path(), &["push", "-q", "-u", "origin", "main"]);
    let mut state = AppState::new();
    state.focus = Pane::Commits;

    lg::git::with_repo(dir.path(), || {
        panel::commits::handle_key(&mut state, key(KeyCode::Char('A'))).unwrap();
        assert_eq!(state.modal, Modal::Commit);
        panel::commit::handle_key(&mut state, ctrl('s')).unwrap();
    });

    let prompt = state.confirm.clone().expect("confirm prompt");
    assert_eq!(prompt.action, PendingAction::AmendCommit);
    assert!(
        prompt.question.contains("origin/main"),
        "{}",
        prompt.question
    );
    assert!(prompt.detail.contains("force push"), "{}", prompt.detail);
}

#[test]
fn the_files_pane_opens_the_stash_on_the_real_stash() {
    let dir = repo();
    std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    git(dir.path(), &["stash", "push", "-q", "-m", "parked work"]);
    let mut state = make_state_with_files();
    state.focus = Pane::Files;

    lg::git::with_repo(dir.path(), || {
        panel::files::handle_key(&mut state, key(KeyCode::Char('S'))).unwrap();
    });

    assert_eq!(state.modal, Modal::Stash);
    assert_eq!(state.stash.entries.len(), 1);
    assert!(state.stash.entries[0].subject.contains("parked work"));
}

/// A feature branch pushed, then amended: diverged from its upstream by one
/// commit each way.
fn diverged_feature(branch: &str) -> (tempfile::TempDir, tempfile::TempDir) {
    let dir = repo();
    let bare = tempfile::tempdir().expect("bare");
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git(dir.path(), &["push", "-q", "-u", "origin", "main"]);
    if branch != "main" {
        git(dir.path(), &["checkout", "-q", "-b", branch]);
        std::fs::write(dir.path().join("f.txt"), "draft\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "draft"]);
        git(dir.path(), &["push", "-q", "-u", "origin", branch]);
    }
    std::fs::write(dir.path().join("f.txt"), "final\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "--amend", "-m", "final"]);
    (dir, bare)
}

#[test]
fn a_diverged_push_offers_force_with_lease_naming_what_it_overwrites() {
    let (dir, _bare) = diverged_feature("feature/rewrite");
    let mut state = AppState::new();
    state.modal = Modal::Push;
    state.branch = Some("feature/rewrite".into());
    state.ahead_behind = Some((1, 1));

    lg::git::with_repo(dir.path(), || {
        panel::push::handle_key(&mut state, key(KeyCode::Char('f'))).unwrap();
    });

    assert_eq!(state.modal, Modal::ConfirmDestructive);
    assert!(
        state.pending_action.is_none(),
        "nothing is pushed before yes"
    );
    let prompt = state.confirm.clone().expect("confirm prompt");
    assert!(
        prompt.question.contains("origin/feature/rewrite"),
        "{}",
        prompt.question
    );
    assert!(prompt.detail.contains("1 commit "), "{}", prompt.detail);
    assert!(prompt.detail.contains("draft"), "{}", prompt.detail);
    assert!(matches!(
        prompt.action,
        PendingAction::ForcePushWithLease(_)
    ));
}

#[test]
fn a_protected_branch_is_never_offered_a_force_push() {
    let (dir, _bare) = diverged_feature("main");
    let mut state = AppState::new();
    state.modal = Modal::Push;
    state.branch = Some("main".into());
    state.ahead_behind = Some((1, 1));

    lg::git::with_repo(dir.path(), || {
        panel::push::handle_key(&mut state, key(KeyCode::Char('f'))).unwrap();
    });

    assert_ne!(state.modal, Modal::ConfirmDestructive);
    assert!(state.pending_action.is_none());
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|status| status.is_error && status.text.contains("protected")),
        "{:?}",
        state.status
    );
}
