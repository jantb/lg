//! Pulling, pushing and force pushing.

use super::common::*;

#[test]
fn pull_merges_upstream_when_current_branch_diverged() {
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

    fs::write(dir.path().join("local.txt"), "local").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in(dir.path(), "local commit");

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
    fs::write(updater.path().join("remote.txt"), "remote").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote commit");
    git_ok(updater.path(), &["push", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::pull().expect("pull diverged branch");

    assert!(
        out.contains("Merge") || out.contains("merge"),
        "expected merge output for diverged pull: {out}"
    );
    let remote_is_ancestor = git(
        dir.path(),
        &["merge-base", "--is-ancestor", "origin/main", "HEAD"],
    );
    assert!(
        remote_is_ancestor.status.success(),
        "pull should merge origin/main into the local branch"
    );
    let log = git(dir.path(), &["log", "--oneline", "main"]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(log.contains("local commit"), "missing local commit: {log}");
    assert!(
        log.contains("remote commit"),
        "missing remote commit: {log}"
    );
}

#[test]
fn pull_stashes_dirty_work_before_updating_main() {
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
    fs::write(updater.path().join("remote.txt"), "remote\n").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote commit");
    git_ok(updater.path(), &["push", "origin", "main"]);

    fs::write(dir.path().join("dirty.txt"), "dirty work\n").unwrap();
    stage_in(dir.path(), "dirty.txt");

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::pull().expect("pull with dirty work");

    assert!(
        out.contains("applied stashed local changes after pull"),
        "pull output should mention stash: {out}"
    );
    assert!(
        dir.path().join("remote.txt").exists(),
        "remote update should be pulled"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("dirty.txt")).unwrap(),
        "dirty work\n"
    );
    assert!(
        status_in(dir.path()).1.contains(&"dirty.txt".to_string()),
        "dirty file should still be staged"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

/// A clone of `bare` that commits as the test user, for pushing updates the
/// repository under test then pulls.
fn updater_clone(bare: &Path) -> TempDir {
    let updater = tempfile::tempdir().expect("updater tempdir");
    git_ok(updater.path(), &["clone", bare.to_str().unwrap(), "."]);
    git_ok(
        updater.path(),
        &["config", "user.email", "test@example.com"],
    );
    git_ok(updater.path(), &["config", "user.name", "Test User"]);
    updater
}

#[test]
fn pull_updates_branch_whose_upstream_has_another_name_and_remote() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "fork", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "fork", "main:feature/remote-name"]);
    git_ok(
        dir.path(),
        &[
            "checkout",
            "-b",
            "local-name",
            "--track",
            "fork/feature/remote-name",
        ],
    );

    let updater = updater_clone(bare.path());
    git_ok(updater.path(), &["checkout", "feature/remote-name"]);
    fs::write(updater.path().join("remote.txt"), "remote\n").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote commit");
    git_ok(updater.path(), &["push", "origin", "feature/remote-name"]);

    fs::write(dir.path().join("dirty.txt"), "dirty work\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    lg::git::pull().expect("pull branch with differently named upstream");

    assert!(
        dir.path().join("remote.txt").exists(),
        "remote update should be pulled"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("dirty.txt")).unwrap(),
        "dirty work\n"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

#[test]
fn pull_without_upstream_leaves_local_changes_in_place() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    fs::write(dir.path().join("dirty.txt"), "dirty work\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    lg::git::pull().expect_err("a branch without upstream has nothing to pull");

    assert_eq!(
        fs::read_to_string(dir.path().join("dirty.txt")).unwrap(),
        "dirty work\n"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "nothing should be stashed when there is nowhere to pull from"
    );
}

#[test]
fn pull_keeps_update_and_says_so_when_local_changes_cannot_be_restored() {
    let dir = init_repo();
    fs::write(dir.path().join("shared.txt"), "base\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let updater = updater_clone(bare.path());
    fs::write(updater.path().join("shared.txt"), "remote\n").unwrap();
    stage_in(updater.path(), "shared.txt");
    commit_in(updater.path(), "remote commit");
    git_ok(updater.path(), &["push", "origin", "main"]);

    fs::write(dir.path().join("shared.txt"), "local\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let err = lg::git::pull().expect_err("restoring conflicting local changes fails");

    assert!(
        err.to_string().contains("left in stash"),
        "error should say where the local changes are: {err}"
    );
    let log = git(dir.path(), &["log", "--oneline", "main"]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(
        log.contains("remote commit"),
        "remote update should be pulled: {log}"
    );
    assert!(
        stash_list(dir.path()).contains("lg: auto-stash before pull"),
        "local changes should still be in the stash"
    );
}

/// A push still goes ahead when the fetch before it fails, but it compared the
/// branch against the remote as of the last fetch that worked, and says so.
#[test]
fn a_push_after_a_failed_fetch_says_its_remote_check_may_be_stale() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    let missing = dir.path().join("no-such-remote");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", missing.to_str().unwrap()],
    );
    git_ok(
        dir.path(),
        &[
            "config",
            "remote.origin.pushurl",
            bare.path().to_str().unwrap(),
        ],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);
    fs::write(dir.path().join("README.md"), "main\nmore\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "second");

    let out = lg::git::with_repo(dir.path(), || lg::git::push("origin", "main"))
        .expect("the push itself still goes through");

    assert!(out.contains("fetch before push failed"), "{out}");
    let pushed = git(bare.path(), &["log", "--format=%s", "-1", "main"]);
    assert_eq!(String::from_utf8_lossy(&pushed.stdout).trim(), "second");
}

/// A feature branch pushed to `origin`, then amended locally so the two have
/// diverged: one commit each side.
fn diverged_feature() -> (TempDir, TempDir) {
    let (dir, bare) = repo_with_origin();
    git_ok(dir.path(), &["checkout", "-q", "-b", "feature/rewrite"]);
    fs::write(dir.path().join("feature.txt"), "draft\n").unwrap();
    commit_all(dir.path(), "feature draft");
    git_ok(dir.path(), &["push", "-u", "origin", "feature/rewrite"]);
    fs::write(dir.path().join("feature.txt"), "final\n").unwrap();
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(
        dir.path(),
        &["commit", "-q", "--amend", "-m", "feature final"],
    );
    (dir, bare)
}

#[test]
fn force_push_with_lease_replaces_the_diverged_remote_branch() {
    let (dir, bare) = diverged_feature();

    let plan = lg::git::with_repo(dir.path(), lg::git::force_push_plan).unwrap();
    assert_eq!(plan.remote_ref(), "origin/feature/rewrite");
    assert_eq!(plan.overwritten.len(), 1, "{:?}", plan.overwritten);
    assert!(plan.overwritten[0].contains("feature draft"));

    lg::git::with_repo(dir.path(), || lg::git::push_force_with_lease(&plan)).unwrap();
    assert_eq!(
        stdout_of(bare.path(), &["rev-parse", "refs/heads/feature/rewrite"]),
        stdout_of(dir.path(), &["rev-parse", "HEAD"])
    );
}

#[test]
fn force_push_with_lease_refuses_a_remote_that_moved_since_the_prompt() {
    let (dir, bare) = diverged_feature();
    let plan = lg::git::with_repo(dir.path(), lg::git::force_push_plan).unwrap();

    // Someone else pushes to the branch after the prompt was shown.
    let other = tempfile::tempdir().expect("other clone");
    git_ok(
        other.path(),
        &[
            "clone",
            "-q",
            "-b",
            "feature/rewrite",
            bare.path().to_str().unwrap(),
            ".",
        ],
    );
    git_ok(other.path(), &["config", "user.email", "o@example.com"]);
    git_ok(other.path(), &["config", "user.name", "Other"]);
    fs::write(other.path().join("theirs.txt"), "theirs\n").unwrap();
    commit_all(other.path(), "their work");
    git_ok(other.path(), &["push", "-q", "origin", "feature/rewrite"]);
    let theirs = stdout_of(other.path(), &["rev-parse", "HEAD"]);
    // The local tracking ref catches up, as a background fetch would.
    git_ok(dir.path(), &["fetch", "-q", "origin"]);

    let result = lg::git::with_repo(dir.path(), || lg::git::push_force_with_lease(&plan));
    assert!(result.is_err(), "the lease no longer holds");
    assert_eq!(
        stdout_of(bare.path(), &["rev-parse", "refs/heads/feature/rewrite"]),
        theirs,
        "their commit is still there"
    );
}

#[test]
fn force_push_is_never_planned_for_a_protected_branch() {
    let (dir, _bare) = repo_with_origin();
    fs::write(dir.path().join("x.txt"), "x\n").unwrap();
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(
        dir.path(),
        &["commit", "-q", "--amend", "-m", "rewritten base"],
    );

    let err =
        lg::git::with_repo(dir.path(), lg::git::force_push_plan).expect_err("main is protected");
    assert!(err.to_string().contains("protected"), "{err}");
}
