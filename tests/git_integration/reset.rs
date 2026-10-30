//! Resetting a branch from its remote, and discarding the checkout.

use super::common::*;

#[test]
fn reset_flow_cleans_safety_backup_after_success() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init\n").unwrap();
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
    fs::write(dir.path().join("develop.txt"), "develop\n").unwrap();
    stage_in(dir.path(), "develop.txt");
    commit_in(dir.path(), "develop commit");
    git_ok(dir.path(), &["push", "-u", "origin", "develop"]);

    let feature = "feature/reset-return";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_reset_branch_from_main(feature, "develop").expect("reset develop");

    assert_eq!(head_branch(dir.path()), feature);
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/reset-develop-develop-"),
        "successful reset should clean safety backup: {branches}"
    );
}

/// Resetting an environment branch rewrites the remote one, and only as it
/// was last seen: a commit pushed there that this checkout has not fetched is
/// refused rather than thrown away, which `--force` used to do.
#[test]
fn reset_flow_refuses_to_overwrite_remote_work_it_has_not_seen() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init\n").unwrap();
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
    fs::write(dir.path().join("develop.txt"), "develop\n").unwrap();
    stage_in(dir.path(), "develop.txt");
    commit_in(dir.path(), "develop commit");
    git_ok(dir.path(), &["push", "-u", "origin", "develop"]);
    git_ok(dir.path(), &["checkout", "main"]);

    // Someone else pushes to develop, and this checkout's fetch only ever
    // updates main, so it never sees that.
    let other = tempfile::tempdir().expect("other clone");
    git_ok(other.path(), &["clone", bare.path().to_str().unwrap(), "."]);
    git_ok(other.path(), &["config", "user.email", "test@example.com"]);
    git_ok(other.path(), &["config", "user.name", "Test User"]);
    git_ok(other.path(), &["checkout", "develop"]);
    fs::write(other.path().join("theirs.txt"), "theirs\n").unwrap();
    stage_in(other.path(), "theirs.txt");
    commit_in(other.path(), "their develop work");
    git_ok(other.path(), &["push", "origin", "develop"]);
    git_ok(
        dir.path(),
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );

    let _cwd = CwdGuard::new(dir.path());
    let result = lg::git::flow_reset_branch_from_main("main", "develop");

    assert!(result.is_err(), "the lease refuses the push: {result:?}");
    let remote_log = git(bare.path(), &["log", "--oneline", "develop"]);
    assert!(
        String::from_utf8_lossy(&remote_log.stdout).contains("their develop work"),
        "their commit is still on the remote branch"
    );
}

#[test]
fn discard_checkout_flow_resets_current_branch_from_remote_and_cleans_worktree() {
    let dir = init_repo();
    fs::write(dir.path().join("app.txt"), "main\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let feature = "feature/reload-from-remote";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("app.txt"), "feature base\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "feature base");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

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
    git_ok(updater.path(), &["checkout", feature]);
    fs::write(updater.path().join("app.txt"), "server head\n").unwrap();
    stage_in(updater.path(), "app.txt");
    commit_in(updater.path(), "server update");
    git_ok(updater.path(), &["push", "origin", feature]);

    fs::write(dir.path().join("app.txt"), "local only\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "local only");
    fs::write(dir.path().join("app.txt"), "dirty staged\n").unwrap();
    stage_in(dir.path(), "app.txt");
    fs::write(dir.path().join("scratch.txt"), "untracked\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::flow_discard_checkout_from_remote(feature).expect("discard checkout");

    assert_eq!(head_branch(dir.path()), feature);
    assert!(
        out.contains("reset to origin/feature/reload-from-remote"),
        "status should mention remote ref: {out}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "server head\n"
    );
    assert!(
        !dir.path().join("scratch.txt").exists(),
        "untracked file should be removed"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(unstaged.is_empty(), "unstaged: {unstaged:?}");
    assert!(staged.is_empty(), "staged: {staged:?}");

    let log = git(dir.path(), &["log", "--oneline", "--max-count=5"]);
    let log_text = String::from_utf8_lossy(&log.stdout);
    assert!(log_text.contains("server update"), "{log_text}");
    assert!(
        !log_text.contains("local only"),
        "local-only commit should not remain on branch: {log_text}"
    );
}
