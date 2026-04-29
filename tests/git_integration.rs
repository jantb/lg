use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Mutex, MutexGuard},
};
use tempfile::TempDir;

static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Run a git command inside `dir`, with author/committer env vars set.
fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("failed to run git")
}

/// Same but panics if exit code != 0.
fn git_ok(dir: &std::path::Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git_ok(dir.path(), &["init", "-b", "main"]);
    git_ok(dir.path(), &["config", "user.email", "test@example.com"]);
    git_ok(dir.path(), &["config", "user.name", "Test User"]);
    dir
}

/// Call `lg::git::status_porcelain` inside a specific working directory.
fn status_in(dir: &std::path::Path) -> (Vec<String>, Vec<String>) {
    // We need to run in that directory, so we temporarily change the process
    // working directory — but that's not thread-safe.  Instead we shell out
    // to git directly and feed the bytes to the parser.
    let out = Command::new("git")
        .args(["status", "-z", "--porcelain=v1"])
        .current_dir(dir)
        .output()
        .expect("git status");
    lg::git::parse_porcelain(&out.stdout)
}

fn stage_in(dir: &std::path::Path, path: &str) {
    git_ok(dir, &["add", "--", path]);
}

fn unstage_in(dir: &std::path::Path, path: &str) {
    // pre-initial-commit: use rm --cached
    let out = git(dir, &["reset", "-q", "HEAD", "--", path]);
    if !out.status.success() {
        let out2 = git(dir, &["rm", "--cached", "--", path]);
        assert!(
            out2.status.success(),
            "unstage failed: {}",
            String::from_utf8_lossy(&out2.stderr)
        );
    }
}

fn commit_in(dir: &std::path::Path, msg: &str) {
    let out = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct CwdGuard {
    old: std::path::PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl CwdGuard {
    fn new(dir: &Path) -> Self {
        let lock = CWD_LOCK.lock().expect("cwd lock poisoned");
        let old = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir).expect("set current dir");
        Self { old, _lock: lock }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.old).expect("restore current dir");
    }
}

fn head_branch(dir: &Path) -> String {
    let out = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert!(
        out.status.success(),
        "git rev-parse failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn status_porcelain_on_fresh_repo_is_empty() {
    let dir = init_repo();
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.is_empty(),
        "expected no unstaged, got {unstaged:?}"
    );
    assert!(staged.is_empty(), "expected no staged, got {staged:?}");
}

#[test]
fn stage_then_unstage_round_trips() {
    let dir = init_repo();
    let file = dir.path().join("hello.txt");
    fs::write(&file, "hello").unwrap();

    // After writing: file is untracked (unstaged only).
    let (u, s) = status_in(dir.path());
    assert!(u.contains(&"hello.txt".to_string()), "should be untracked");
    assert!(!s.contains(&"hello.txt".to_string()));

    // Stage it.
    stage_in(dir.path(), "hello.txt");
    let (u, s) = status_in(dir.path());
    assert!(s.contains(&"hello.txt".to_string()), "should be staged");
    assert!(!u.contains(&"hello.txt".to_string()));

    // Unstage it.
    unstage_in(dir.path(), "hello.txt");
    let (u, s) = status_in(dir.path());
    assert!(
        u.contains(&"hello.txt".to_string()),
        "should be back in unstaged"
    );
    assert!(!s.contains(&"hello.txt".to_string()));
}

#[test]
fn head_branch_returns_current_branch() {
    let dir = init_repo();
    // Need at least one commit for HEAD to resolve.
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    assert_eq!(branch, "main");
}

#[test]
fn commit_on_empty_message_fails() {
    // lg::git::commit guards against empty messages.
    let result = lg::git::commit("");
    assert!(result.is_err(), "expected Err for empty message");
}

#[test]
fn release_flow_returns_to_original_branch_after_target_push() {
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
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "origin", "develop"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "release/next"]);
    git_ok(dir.path(), &["push", "origin", "release/next"]);

    let feature = "feature/release-return";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "develop").expect("release to develop");
    assert_eq!(head_branch(dir.path()), feature);

    lg::git::flow_release_current(feature, "release/next").expect("release to release/next");
    assert_eq!(head_branch(dir.path()), feature);

    let develop_log = git(bare.path(), &["log", "--oneline", "develop"]);
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("feature commit"),
        "develop did not receive feature commit"
    );
    let release_log = git(bare.path(), &["log", "--oneline", "release/next"]);
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("feature commit"),
        "release/next did not receive feature commit"
    );
    let local_release = git(dir.path(), &["rev-parse", "release/next"]);
    let remote_release = git(bare.path(), &["rev-parse", "release/next"]);
    assert_eq!(
        String::from_utf8_lossy(&local_release.stdout),
        String::from_utf8_lossy(&remote_release.stdout),
        "origin/release/next was not pushed to the merged release/next HEAD"
    );
    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "release/next@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "release/next upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/release/next"
    );
}

#[test]
fn release_conflict_continue_auto_stages_pushes_target_and_returns_to_feature() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "origin", "develop"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "release/next"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "release/next"]);

    let feature = "feature/release-conflict";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "release/next")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "release/next");

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    lg::git::continue_in_progress_operation_with_followup(Some("release/next"), Some(feature))
        .expect("continue release conflict");

    assert_eq!(head_branch(dir.path()), feature);
    let released_file = git(bare.path(), &["show", "release/next:conflict.txt"]);
    assert!(
        released_file.status.success(),
        "release/next file missing: {}",
        String::from_utf8_lossy(&released_file.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&released_file.stdout), "resolved\n");
}

#[test]
fn merge_main_flow_stashes_dirty_work_updates_main_and_returns_to_feature() {
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

    let feature = "feature/merge-main";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

    fs::write(dir.path().join("dirty.txt"), "dirty work").unwrap();

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

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature).expect("merge main into feature");

    assert_eq!(head_branch(dir.path()), feature);
    assert!(
        dir.path().join("dirty.txt").exists(),
        "dirty work should be restored"
    );

    let main_rev = git(dir.path(), &["rev-parse", "main"]);
    let origin_main_rev = git(dir.path(), &["rev-parse", "origin/main"]);
    assert_eq!(
        String::from_utf8_lossy(&main_rev.stdout),
        String::from_utf8_lossy(&origin_main_rev.stdout),
        "local main should be updated to origin/main"
    );

    let log = git(dir.path(), &["log", "--oneline", feature]);
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("main update"),
        "feature branch did not receive origin/main"
    );

    let stash_list = git(dir.path(), &["stash", "list"]);
    assert!(
        String::from_utf8_lossy(&stash_list.stdout).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

#[test]
fn branch_release_status_reports_missing_commits_after_release() {
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
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "origin", "develop"]);

    let feature = "feature/release-status";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "released").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "released feature commit");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "develop").expect("release to develop");

    fs::write(dir.path().join("followup.txt"), "not released").unwrap();
    stage_in(dir.path(), "followup.txt");
    commit_in(dir.path(), "unreleased followup");

    let status = lg::git::branch_release_status(feature).expect("branch release status");
    let develop = status.develop.expect("develop release status");
    assert!(!develop.released_at.is_empty(), "missing release timestamp");
    assert_eq!(develop.missing_commits, 1);
    assert!(status.test.is_none(), "release/next should not be marked");
}

// ── parse_porcelain unit tests (comprehensive) ─────────────────────────────

#[test]
fn parse_porcelain_modified_untracked_renamed_and_both() {
    // Build a synthetic -z byte string:
    //  " M modified.rs"  — worktree-only modified (unstaged)
    //  "?? untracked.txt" — untracked (unstaged)
    //  "R  renamed_new.rs" + "renamed_old.rs"  — staged rename
    //  "MM both.rs"       — staged AND unstaged modified
    let input: Vec<u8> = {
        let mut v = Vec::new();
        v.extend_from_slice(b" M modified.rs");
        v.push(0);
        v.extend_from_slice(b"?? untracked.txt");
        v.push(0);
        v.extend_from_slice(b"R  renamed_new.rs");
        v.push(0);
        v.extend_from_slice(b"renamed_old.rs");
        v.push(0);
        v.extend_from_slice(b"MM both.rs");
        v.push(0);
        v
    };

    let (unstaged, staged) = lg::git::parse_porcelain(&input);

    // Staged: renamed_new.rs (R) + both.rs (MM index side)
    assert!(
        staged.contains(&"renamed_new.rs".to_string()),
        "staged: {staged:?}"
    );
    assert!(
        staged.contains(&"both.rs".to_string()),
        "staged: {staged:?}"
    );
    assert!(
        !staged.contains(&"modified.rs".to_string()),
        "modified.rs should not be staged"
    );
    assert!(!staged.contains(&"untracked.txt".to_string()));

    // Unstaged: modified.rs + untracked.txt + both.rs (MM worktree side)
    assert!(
        unstaged.contains(&"modified.rs".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(
        unstaged.contains(&"untracked.txt".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(
        unstaged.contains(&"both.rs".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(!unstaged.contains(&"renamed_new.rs".to_string()));
}
