//! The commit log and committing.

use super::common::*;

fn commit_in_as(dir: &std::path::Path, msg: &str, author: &str, email: &str) {
    let out = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", author)
        .env("GIT_COMMITTER_EMAIL", email)
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn head_branch_returns_current_branch() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let _cwd = CwdGuard::new(dir.path());
    assert_eq!(lg::git::head_branch().unwrap(), "main");
}

#[test]
fn head_branch_returns_unborn_branch_before_first_commit() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    assert_eq!(lg::git::head_branch().unwrap(), "main");
}

#[test]
fn list_commits_on_empty_repo_is_empty() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    let commits = lg::git::list_commits(10).unwrap();
    let current_branch_commits = lg::git::list_commits_for_ref("main", 10).unwrap();

    assert!(commits.is_empty());
    assert!(current_branch_commits.is_empty());
}

#[test]
fn list_commits_includes_short_author_name() {
    let dir = init_repo();
    fs::write(dir.path().join("a.txt"), "one").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in_as(
        dir.path(),
        "add authored commit",
        "Alice Example",
        "alice@example.com",
    );

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits(10).unwrap();

    assert_eq!(commits[0].author, "Alice Example");
    assert_eq!(commits[0].author_short, "AE");
    assert!(commits[0].is_first_parent);
    assert_eq!(commits[0].subject, "add authored commit");
}

#[test]
fn list_commits_for_ref_reads_selected_branch_history() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/log"]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature branch commit");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main branch commit");

    let _cwd = CwdGuard::new(dir.path());
    let feature_commits = lg::git::list_commits_for_ref("feature/log", 10).unwrap();
    let main_commits = lg::git::list_commits_for_ref("main", 10).unwrap();

    assert_eq!(feature_commits[0].subject, "feature branch commit");
    assert_eq!(main_commits[0].subject, "main branch commit");
}

#[test]
fn list_commits_marks_merge_commits_with_multiple_parents() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/merge"]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main side");
    git_ok(
        dir.path(),
        &["merge", "--no-ff", "feature/merge", "-m", "merge feature"],
    );

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits_for_ref("main", 10).unwrap();

    assert_eq!(commits[0].subject, "merge feature");
    assert_eq!(commits[0].parent_count(), 2);
    assert!(commits[0].is_first_parent);
    assert!(
        commits
            .iter()
            .any(|commit| commit.subject == "feature side" && !commit.is_first_parent),
        "merged-in feature commit should not be on the first-parent branch: {commits:?}"
    );
}

#[test]
fn list_commits_renders_complex_merges_with_lazygit_glyphs() {
    let dir = init_repo();
    fs::write(dir.path().join("base.txt"), "base").unwrap();
    stage_in(dir.path(), "base.txt");
    commit_in(dir.path(), "base");

    git_ok(dir.path(), &["checkout", "-b", "side-a"]);
    fs::write(dir.path().join("a.txt"), "a1").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "a1");
    fs::write(dir.path().join("a.txt"), "a2").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "a2");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main1").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main1");
    git_ok(dir.path(), &["merge", "--no-ff", "side-a", "-m", "merge-a"]);

    git_ok(dir.path(), &["checkout", "-b", "side-b", "HEAD~1"]);
    fs::write(dir.path().join("b.txt"), "b1").unwrap();
    stage_in(dir.path(), "b.txt");
    commit_in(dir.path(), "b1");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main2").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main2");
    git_ok(dir.path(), &["merge", "--no-ff", "side-b", "-m", "merge-b"]);

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits_for_ref("main", 20).unwrap();

    // 8 real commits: base, a1, a2, main1, merge-a, b1, main2, merge-b.
    assert_eq!(commits.len(), 8);
    assert!(
        commits.iter().all(|commit| !commit.subject.is_empty()),
        "every rendered row should be a real commit with a subject: {commits:?}"
    );
    let merge_a = commits
        .iter()
        .find(|commit| commit.subject == "merge-a")
        .expect("merge-a commit");
    assert_eq!(merge_a.parent_count(), 2);
    let merge_b = commits
        .iter()
        .find(|commit| commit.subject == "merge-b")
        .expect("merge-b commit");
    assert_eq!(merge_b.parent_count(), 2);

    let mut state = lg::state::AppState::new();
    state.commits = commits;
    let backend = ratatui::backend::TestBackend::new(120, 20);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            lg::panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    // Lazygit-style merge marker followed directly by ─╮ (no padding).
    assert!(
        rendered.contains("\u{23e3}\u{2500}\u{256e}"),
        "rendered graph should include merge connector: {rendered}"
    );
    // Round corners only — no slash diagonals or backslashes.
    assert!(
        !rendered.contains('\\')
            && !rendered.contains('\u{2572}')
            && !rendered.contains('\u{2571}'),
        "rendered graph should use curved connector glyphs instead of slash diagonals: {rendered}"
    );
    assert!(
        rendered.contains("merge-a") && rendered.contains("a2"),
        "rendered graph should include merge and side branch commits: {rendered}"
    );
}

#[test]
fn list_commits_renders_repeated_main_merges_into_feature_branch() {
    let dir = init_repo();
    fs::write(dir.path().join("base.txt"), "base").unwrap();
    stage_in(dir.path(), "base.txt");
    commit_in(dir.path(), "base");

    git_ok(dir.path(), &["checkout", "-b", "feature"]);
    fs::write(dir.path().join("feature.txt"), "feature-1").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature-1");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main-1.txt"), "main-1").unwrap();
    stage_in(dir.path(), "main-1.txt");
    commit_in(dir.path(), "main-1");

    git_ok(dir.path(), &["checkout", "feature"]);
    git_ok(
        dir.path(),
        &["merge", "--no-ff", "main", "-m", "merge-main-1"],
    );
    fs::write(dir.path().join("feature.txt"), "feature-2").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature-2");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main-2.txt"), "main-2").unwrap();
    stage_in(dir.path(), "main-2.txt");
    commit_in(dir.path(), "main-2");

    git_ok(dir.path(), &["checkout", "feature"]);
    git_ok(
        dir.path(),
        &["merge", "--no-ff", "main", "-m", "merge-main-2"],
    );
    fs::write(dir.path().join("feature.txt"), "feature-3").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature-3");

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits_for_ref("feature", 30).unwrap();

    let merge_main = commits
        .iter()
        .find(|commit| commit.subject == "merge-main-1")
        .expect("merge-main-1 commit");
    assert_eq!(merge_main.parent_count(), 2);

    let mut state = lg::state::AppState::new();
    state.commits = commits;
    let backend = ratatui::backend::TestBackend::new(120, 20);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            lg::panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains('\u{23e3}') && rendered.contains('\u{256e}'),
        "rendered main merges should include visible merge connectors: {rendered}"
    );
}

#[test]
fn branch_log_renders_decorated_graph_log() {
    let dir = init_repo();
    fs::write(dir.path().join("a.txt"), "one").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "initial commit");

    let _cwd = CwdGuard::new(dir.path());
    let log = lg::git::branch_log("main", 10).unwrap();

    assert!(
        log.contains("* commit "),
        "missing graph commit line: {log}"
    );
    assert!(log.contains("Author:"), "missing author line: {log}");
    assert!(log.contains("Date:"), "missing date line: {log}");
    assert!(log.contains("initial commit"), "missing message: {log}");
}

#[test]
fn commit_on_empty_message_fails() {
    // lg::git::commit guards against empty messages.
    let result = lg::git::commit("");
    assert!(result.is_err(), "expected Err for empty message");
}

#[test]
fn commit_on_empty_repo_creates_initial_commit() {
    let dir = init_repo();
    fs::write(dir.path().join("first.txt"), "first").unwrap();
    stage_in(dir.path(), "first.txt");
    let _cwd = CwdGuard::new(dir.path());

    lg::git::commit("initial commit").unwrap();

    let commits = lg::git::list_commits(10).unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].subject, "initial commit");
}

#[test]
fn commit_on_release_branch_updates_from_origin_main_first() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "-u", "origin", "develop"]);

    let updater = tempfile::tempdir().expect("updater tempdir");
    git_ok(
        updater.path(),
        &["clone", bare.path().to_str().unwrap(), "."],
    );
    git_ok(
        updater.path(),
        &["config", "user.email", "test@example.com"],
    );
    git_ok(updater.path(), &["config", "user.name", "Test User"]);
    fs::write(updater.path().join("main.txt"), "main update").unwrap();
    stage_in(updater.path(), "main.txt");
    commit_in(updater.path(), "main update");
    git_ok(updater.path(), &["push", "origin", "main"]);

    fs::write(dir.path().join("direct.txt"), "direct").unwrap();
    stage_in(dir.path(), "direct.txt");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::commit("direct release commit").expect("commit on stale release branch");

    let main_is_ancestor = git(
        dir.path(),
        &["merge-base", "--is-ancestor", "origin/main", "HEAD"],
    );
    assert!(
        main_is_ancestor.status.success(),
        "release branch should include origin/main before the direct commit"
    );
    let log = git(dir.path(), &["log", "--oneline", "develop"]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(log.contains("main update"), "missing main update: {log}");
    assert!(
        log.contains("direct release commit"),
        "missing direct commit: {log}"
    );
}

/// A hook that refuses a commit says why on stdout as often as on stderr, and
/// the error used to start with the whole commit message. It now names the
/// command and keeps what the hook printed.
#[test]
fn a_refused_commit_says_what_the_hook_printed_and_not_the_message() {
    let dir = init_repo();
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    stage_in(dir.path(), "a.txt");
    let hook = dir.path().join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        "#!/bin/sh\necho 'lint: a.txt has trailing junk'\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let _cwd = CwdGuard::new(dir.path());
    let err = lg::git::commit("feat: a message long enough to bury anything after it")
        .expect_err("the hook refuses the commit");
    let message = format!("{err:#}");

    assert!(message.starts_with("git commit failed"), "{message}");
    assert!(
        message.contains("lint: a.txt has trailing junk"),
        "{message}"
    );
    assert!(!message.contains("bury anything"), "{message}");
}
