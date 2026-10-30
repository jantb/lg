//! Merging main into branches, one or all of them, and its conflicts.

use super::common::*;

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
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/merge-main-feature-merge-main-"),
        "successful merge-main should clean safety backup: {branches}"
    );
}

#[test]
fn merge_main_flow_pulls_current_branch_before_merging_main() {
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

    let feature = "feature/pull-current-before-merge-main";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
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
    fs::write(updater.path().join("remote-feature.txt"), "remote feature").unwrap();
    stage_in(updater.path(), "remote-feature.txt");
    commit_in(updater.path(), "remote feature update");
    git_ok(updater.path(), &["push", "origin", feature]);

    git_ok(updater.path(), &["checkout", "main"]);
    fs::write(updater.path().join("main.txt"), "main update").unwrap();
    stage_in(updater.path(), "main.txt");
    commit_in(updater.path(), "main update");
    git_ok(updater.path(), &["push", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature).expect("merge main into feature");

    assert_eq!(head_branch(dir.path()), feature);
    let log = git(dir.path(), &["log", "--oneline", feature]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(
        log.contains("remote feature update"),
        "feature branch did not pull the remote feature update: {log}"
    );
    assert!(
        log.contains("main update"),
        "feature branch did not receive origin/main: {log}"
    );

    let remote_log = git(bare.path(), &["log", "--oneline", feature]);
    let remote_log = String::from_utf8_lossy(&remote_log.stdout);
    assert!(
        remote_log.contains("main update"),
        "remote feature branch was not pushed after merge-main: {remote_log}"
    );
}

#[test]
fn merge_main_all_branches_merges_and_pushes_tracked_branches() {
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

    let tracked = "feature/tracked";
    git_ok(dir.path(), &["checkout", "-b", tracked]);
    fs::write(dir.path().join("tracked.txt"), "tracked").unwrap();
    stage_in(dir.path(), "tracked.txt");
    commit_in(dir.path(), "tracked commit");
    git_ok(dir.path(), &["push", "-u", "origin", tracked]);

    let local_only = "feature/local-only";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", local_only]);
    fs::write(dir.path().join("local.txt"), "local").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in(dir.path(), "local commit");
    git_ok(dir.path(), &["checkout", tracked]);
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
    let out = lg::git::flow_merge_main_into_all_local_branches().expect("sync branches");

    assert_eq!(head_branch(dir.path()), tracked);
    assert!(
        dir.path().join("dirty.txt").exists(),
        "dirty work should be restored on original branch"
    );
    assert!(
        out.contains("merged origin/main into 2 branches, pushed 1, skipped push 1"),
        "unexpected summary: {out}"
    );

    let tracked_log = git(bare.path(), &["log", "--oneline", tracked]);
    assert!(
        String::from_utf8_lossy(&tracked_log.stdout).contains("main update"),
        "tracked branch was not pushed with main update"
    );
    let local_log = git(dir.path(), &["log", "--oneline", local_only]);
    assert!(
        String::from_utf8_lossy(&local_log.stdout).contains("main update"),
        "local-only branch did not receive main update"
    );
    let remote_local = git(bare.path(), &["rev-parse", "--verify", local_only]);
    assert!(
        !remote_local.status.success(),
        "local-only branch should not be pushed"
    );
    let stash_list = git(dir.path(), &["stash", "list"]);
    assert!(
        String::from_utf8_lossy(&stash_list.stdout).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

#[test]
fn merge_main_all_branches_merges_remote_updates_before_pushing() {
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

    let rejected = "feature/rejected-push";
    git_ok(dir.path(), &["checkout", "-b", rejected]);
    fs::write(dir.path().join("rejected.txt"), "local").unwrap();
    stage_in(dir.path(), "rejected.txt");
    commit_in_at(dir.path(), "rejected local commit", "2026-01-03T00:00:00Z");
    git_ok(dir.path(), &["push", "-u", "origin", rejected]);

    let local_only = "feature/local-after-reject";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", local_only]);
    fs::write(dir.path().join("local.txt"), "local").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in_at(dir.path(), "local-only commit", "2026-01-02T00:00:00Z");
    git_ok(dir.path(), &["checkout", rejected]);

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
    git_ok(updater.path(), &["checkout", rejected]);
    fs::write(updater.path().join("remote.txt"), "remote-only").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote-only rejected update");
    git_ok(updater.path(), &["push", "origin", rejected]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::flow_merge_main_into_all_local_branches().expect("sync branches");

    assert_eq!(head_branch(dir.path()), rejected);
    assert!(
        out.contains("merged origin/main into 2 branches, pushed 1, skipped push 1"),
        "unexpected summary: {out}"
    );

    let rejected_log = git(dir.path(), &["log", "--oneline", rejected]);
    assert!(
        String::from_utf8_lossy(&rejected_log.stdout).contains("main update"),
        "rejected-push branch did not receive main update locally"
    );
    let local_log = git(dir.path(), &["log", "--oneline", local_only]);
    assert!(
        String::from_utf8_lossy(&local_log.stdout).contains("main update"),
        "local-only branch after rejected push did not receive main update"
    );
    let remote_rejected_log = git(bare.path(), &["log", "--oneline", rejected]);
    assert!(
        String::from_utf8_lossy(&remote_rejected_log.stdout).contains("main update"),
        "remote branch should receive merged main update after upstream sync"
    );
}

#[test]
fn merge_main_all_branches_reports_git_conflict_output() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let feature = "feature/sync-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["checkout", feature]);

    let _cwd = CwdGuard::new(dir.path());
    let err = lg::git::flow_merge_main_into_all_local_branches()
        .expect_err("sync branches should stop on conflict")
        .to_string();

    assert!(
        err.contains("merge main into feature/sync-conflict failed"),
        "missing branch context: {err}"
    );
    assert!(
        err.contains("CONFLICT") && err.contains("conflict.txt"),
        "missing git conflict output: {err}"
    );
    assert!(
        err.contains("Automatic merge failed"),
        "missing git merge failure message: {err}"
    );
}

/// A sync that stopped on a conflict has the checkout on the branch it could
/// not merge, and the work it stashed belongs to the branch it started from.
/// Validating has to put both back: without the return leg the stash comes
/// back on top of the conflicted branch and the branch the work was written on
/// is left behind.
#[test]
fn sync_conflict_validation_returns_to_the_branch_the_sync_started_on() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let feature = "feature/sync-stash";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");

    fs::write(dir.path().join("dirty.txt"), "dirty work").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_all_local_branches()
        .expect_err("the sync should stop on the branch that will not merge");
    assert_eq!(head_branch(dir.path()), feature);
    assert!(
        !dir.path().join("dirty.txt").exists(),
        "the work stays stashed while the merge is conflicted"
    );

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::validate_conflict_resolution(lg::git::Followup {
        return_branch: Some("main"),
        ..Default::default()
    })
    .expect("continue the sync conflict");

    assert_eq!(
        head_branch(dir.path()),
        "main",
        "validation should come back to the branch the sync started on: {out}"
    );
    assert!(
        dir.path().join("dirty.txt").exists(),
        "the stashed work should be back on the branch it was written on: {out}"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "nothing should be left stashed: {}",
        stash_list(dir.path())
    );
}

#[test]
fn merge_main_flow_allows_release_branches_when_main_is_ahead() {
    for target in ["develop", "test"] {
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

        git_ok(dir.path(), &["checkout", "-b", target]);
        fs::write(dir.path().join("target.txt"), target).unwrap();
        stage_in(dir.path(), "target.txt");
        commit_in(dir.path(), "target commit");
        git_ok(dir.path(), &["push", "-u", "origin", target]);

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
        lg::git::flow_merge_main_into_current(target).expect("merge main into release branch");

        assert_eq!(head_branch(dir.path()), target);
        let log = git(dir.path(), &["log", "--oneline", target]);
        assert!(
            String::from_utf8_lossy(&log.stdout).contains("main update"),
            "{target} did not receive origin/main"
        );
    }
}

#[test]
fn merge_main_conflict_validation_cleans_safety_backup() {
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
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let feature = "feature/merge-main-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["push", "origin", "main"]);
    git_ok(dir.path(), &["checkout", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature)
        .expect_err("merge-main should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), feature);
    assert!(
        branch_list(dir.path()).contains("lg/backup/merge-main-feature-merge-main-conflict-"),
        "merge-main conflict should leave a safety backup before validation"
    );

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::validate_conflict_resolution(lg::git::Followup {
        push_branch: Some(feature),
        return_branch: Some(feature),
        safety_cleanup: Some(("merge-main", feature)),
        ..Default::default()
    })
    .expect("continue merge-main conflict");

    assert!(
        out.contains("removed lg/backup/merge-main-feature-merge-main-conflict-"),
        "validation should report backup cleanup: {out}"
    );
    assert_eq!(head_branch(dir.path()), feature);
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/merge-main-feature-merge-main-conflict-"),
        "validation should clean merge-main safety backup: {branches}"
    );
}

#[test]
fn merge_main_conflict_abort_cleans_safety_backup() {
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
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let feature = "feature/merge-main-abort";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["push", "origin", "main"]);
    git_ok(dir.path(), &["checkout", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature)
        .expect_err("merge-main should stop for manual conflict resolution");
    assert!(
        branch_list(dir.path()).contains("lg/backup/merge-main-feature-merge-main-abort-"),
        "merge-main conflict should leave a safety backup before abort"
    );

    let out = lg::git::abort_in_progress_operation_with_cleanup(
        Some(feature),
        Some(("merge-main", feature)),
    )
    .expect("abort merge-main conflict");
    assert!(
        out.contains("removed lg/backup/merge-main-feature-merge-main-abort-"),
        "abort should report backup cleanup: {out}"
    );
    assert_eq!(head_branch(dir.path()), feature);
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/merge-main-feature-merge-main-abort-"),
        "abort should clean merge-main safety backup: {branches}"
    );
}

/// Merging main from a linked worktree used to run `git checkout main` there,
/// which git refuses because the main checkout still holds it. The flow died
/// after its auto-stash, so the worktree went empty with the work in a stash
/// nobody mentioned.
#[test]
fn merge_main_flow_merges_from_a_worktree_without_checking_main_out() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let linked = elsewhere.path().join("feat-x");
    git_ok(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/x",
            linked.to_str().expect("worktree path"),
        ],
    );

    fs::write(repo.path().join("main.txt"), "main update").unwrap();
    stage_in(repo.path(), "main.txt");
    commit_in(repo.path(), "main update");

    fs::write(linked.join("dirty.txt"), "dirty work").unwrap();

    let summary = lg::git::with_repo(&linked, || {
        lg::git::flow_merge_main_into_current("feature/x")
    })
    .expect("merge main into the worktree's branch");

    let log = git(&linked, &["log", "--oneline", "feature/x"]);
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("main update"),
        "the worktree's branch did not receive main: {summary}"
    );
    assert!(
        linked.join("dirty.txt").exists(),
        "the stashed work should be back: {summary}"
    );
    assert!(
        stash_list(&linked).is_empty(),
        "auto-stash should be restored and dropped: {}",
        stash_list(&linked)
    );
    assert_eq!(head_branch(&linked), "feature/x");
    assert_eq!(
        head_branch(repo.path()),
        "main",
        "the main checkout should be left where it was"
    );
    assert!(
        summary.contains("kept feature/x local"),
        "a branch without an upstream is not published by a merge: {summary}"
    );
    assert!(
        !branch_list(repo.path()).contains("lg/backup/merge-main-"),
        "a successful merge-main cleans its safety backup: {}",
        branch_list(repo.path())
    );
}

/// The branch list counts how far behind the local `main` a branch is, so that
/// is the `main` a merge has to deliver — merging only `origin/main` reported
/// success while leaving the branch exactly as behind as it was.
#[test]
fn merge_main_flow_merges_commits_only_the_local_main_has() {
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

    let feature = "feature/local-main";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("local.txt"), "not pushed").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in(dir.path(), "local main commit");
    git_ok(dir.path(), &["checkout", feature]);

    let summary = lg::git::with_repo(dir.path(), || {
        lg::git::flow_merge_main_into_current(feature)
    })
    .expect("merge main into feature");

    let log = git(dir.path(), &["log", "--oneline", feature]);
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("local main commit"),
        "the unpushed main commit should have been merged: {summary}"
    );
}

/// A flow that fails after auto-stashing has to put the work back. Leaving it
/// stashed empties the checkout with nothing on screen saying why.
#[test]
fn merge_main_flow_restores_the_auto_stash_when_it_fails() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    git_ok(dir.path(), &["checkout", "-b", "feature/no-main"]);
    git_ok(dir.path(), &["branch", "-D", "main"]);

    fs::write(dir.path().join("dirty.txt"), "dirty work").unwrap();

    let err = lg::git::with_repo(dir.path(), || {
        lg::git::flow_merge_main_into_current("feature/no-main")
    })
    .expect_err("a repository without main cannot merge it");

    let message = err.to_string();
    assert!(
        message.contains("could not find main"),
        "the failure should say what was missing: {message}"
    );
    assert!(
        message.contains("restored your uncommitted changes"),
        "the failure should say the work came back: {message}"
    );
    assert!(
        dir.path().join("dirty.txt").exists(),
        "the stashed work should be back"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "nothing should be left stashed: {}",
        stash_list(dir.path())
    );
}

/// A conflicted merge cannot take the stash back, so the error says where the
/// work is and validating the resolution restores it.
#[test]
fn merge_main_conflict_keeps_the_stash_until_the_conflict_is_validated() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let feature = "feature/stashed-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["checkout", feature]);

    fs::write(dir.path().join("dirty.txt"), "dirty work").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let err = lg::git::flow_merge_main_into_current(feature)
        .expect_err("merge-main should stop for manual conflict resolution");
    let message = err.to_string();
    assert!(
        message.contains("git stash pop"),
        "the error should say how to get the work back: {message}"
    );
    assert!(
        !dir.path().join("dirty.txt").exists(),
        "the work stays stashed while the merge is conflicted"
    );

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::validate_conflict_resolution(lg::git::Followup {
        return_branch: Some(feature),
        safety_cleanup: Some(("merge-main", feature)),
        ..Default::default()
    })
    .expect("continue merge-main conflict");

    assert!(
        out.contains("restored"),
        "validation should report the stash coming back: {out}"
    );
    assert!(
        dir.path().join("dirty.txt").exists(),
        "the stashed work should be back after validation: {out}"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "nothing should be left stashed: {}",
        stash_list(dir.path())
    );
}

/// Syncing every branch stops at the first one that will not merge, and that
/// branch is left merged-but-unpushed with the branches after it untouched.
/// Running the sync again is what finishes it, so it has to be safe to repeat:
/// the branch that conflicted gets pushed, and the ones already done are
/// no-ops rather than errors.
#[test]
fn syncing_again_after_a_conflict_pushes_the_branch_it_stopped_on() {
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

    // A branch that will conflict with main.
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "-u", "origin", "test"]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());

    // The sync stops on test, which is left mid-merge.
    lg::git::flow_merge_main_into_all_local_branches()
        .expect_err("the sync should stop on the branch that conflicts");
    assert!(
        !lg::git::conflicted_files()
            .expect("conflicted files")
            .is_empty(),
        "the conflict should be sitting in the checkout"
    );

    // Resolved and continued, exactly as pressing v does.
    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution(lg::git::Followup::default())
        .expect("continue the conflict");

    // At this point the merge is committed but nothing has been pushed — the
    // state the sync used to be abandoned in.
    let pushed = git(bare.path(), &["show", "test:shared.txt"]);
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout),
        "release\n",
        "the resolution should not have reached the remote yet"
    );

    // Running the sync again is what the followup does, and it finishes.
    lg::git::flow_merge_main_into_all_local_branches().expect("the sync should complete");

    let pushed = git(bare.path(), &["show", "test:shared.txt"]);
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout),
        "resolved\n",
        "the branch it stopped on should now be pushed"
    );
    assert!(
        lg::git::conflicted_files()
            .expect("conflicted files")
            .is_empty(),
        "nothing should be left conflicted"
    );
}
