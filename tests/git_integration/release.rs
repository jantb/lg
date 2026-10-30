//! Releasing a feature into a deploy branch, and its conflicts.

use super::common::*;

/// The followup lg hands `v` for a release of `feature` into `target`. Tests go
/// through this so they exercise the same continuation the UI does, including
/// the feature merge a conflict may have stopped short of.
fn release_followup<'a>(feature: &'a str, target: &'a str) -> lg::git::Followup<'a> {
    lg::git::Followup {
        merge_branch: Some(feature),
        push_branch: Some(target),
        return_branch: Some(feature),
        safety_cleanup: None,
    }
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-return";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

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
    lg::git::flow_release_current(feature, "develop").expect("release to develop");
    assert_eq!(head_branch(dir.path()), feature);

    lg::git::flow_release_current(feature, "test").expect("release to test");
    assert_eq!(head_branch(dir.path()), feature);

    let develop_log = git(bare.path(), &["log", "--oneline", "develop"]);
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("feature commit"),
        "develop did not receive feature commit"
    );
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("main update"),
        "develop did not receive origin/main before release"
    );
    let release_log = git(bare.path(), &["log", "--oneline", "test"]);
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("feature commit"),
        "test did not receive feature commit"
    );
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("main update"),
        "test did not receive origin/main before release"
    );
    let local_release = git(dir.path(), &["rev-parse", "test"]);
    let remote_release = git(bare.path(), &["rev-parse", "test"]);
    assert_eq!(
        String::from_utf8_lossy(&local_release.stdout),
        String::from_utf8_lossy(&remote_release.stdout),
        "origin/test was not pushed to the merged test HEAD"
    );
    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "test@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "test upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/test"
    );
    let branches = git(dir.path(), &["branch", "--format=%(refname:short)"]);
    let branch_text = String::from_utf8_lossy(&branches.stdout);
    assert!(
        !branch_text
            .lines()
            .any(|branch| branch.starts_with("lg/backup/")),
        "successful release should clean up safety branches: {branch_text}"
    );
}

#[test]
fn release_flow_stashes_dirty_work_for_target_checkouts() {
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
    fs::write(dir.path().join("target_only.txt"), "develop\n").unwrap();
    stage_in(dir.path(), "target_only.txt");
    commit_in(dir.path(), "develop target file");
    git_ok(dir.path(), &["push", "origin", "develop"]);

    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("target_only.txt"), "release\n").unwrap();
    stage_in(dir.path(), "target_only.txt");
    commit_in(dir.path(), "release target file");
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-dirty";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

    fs::write(dir.path().join("init.txt"), "dirty init").unwrap();
    fs::write(dir.path().join("target_only.txt"), "untracked local").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "develop").expect("release to develop");
    assert_eq!(head_branch(dir.path()), feature);
    assert_eq!(
        fs::read_to_string(dir.path().join("init.txt")).unwrap(),
        "dirty init"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("target_only.txt")).unwrap(),
        "untracked local"
    );

    lg::git::flow_release_current(feature, "test").expect("release to test");
    assert_eq!(head_branch(dir.path()), feature);
    assert_eq!(
        fs::read_to_string(dir.path().join("init.txt")).unwrap(),
        "dirty init"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("target_only.txt")).unwrap(),
        "untracked local"
    );

    let develop_log = git(bare.path(), &["log", "--oneline", "develop"]);
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("feature commit"),
        "develop did not receive feature commit"
    );
    let release_log = git(bare.path(), &["log", "--oneline", "test"]);
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("feature commit"),
        "test did not receive feature commit"
    );
    let stash_list = git(dir.path(), &["stash", "list"]);
    assert!(
        String::from_utf8_lossy(&stash_list.stdout).is_empty(),
        "auto-stash should be restored and dropped"
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-conflict";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "test");

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue release conflict");

    assert_eq!(head_branch(dir.path()), feature);
    let released_file = git(bare.path(), &["show", "test:conflict.txt"]);
    assert!(
        released_file.status.success(),
        "test file missing: {}",
        String::from_utf8_lossy(&released_file.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&released_file.stdout), "resolved\n");
}

#[test]
fn release_conflict_validate_pushes_target_after_user_returns_to_feature() {
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-conflict-manual";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "test");

    fs::write(dir.path().join("conflict.txt"), "manually resolved\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    git_ok(dir.path(), &["commit", "--no-edit"]);
    git_ok(dir.path(), &["checkout", feature]);

    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("validate manually completed release conflict");

    assert_eq!(head_branch(dir.path()), feature);
    let released_file = git(bare.path(), &["show", "test:conflict.txt"]);
    assert!(
        released_file.status.success(),
        "test file missing: {}",
        String::from_utf8_lossy(&released_file.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&released_file.stdout),
        "manually resolved\n"
    );
}

#[test]
fn release_conflict_validate_merges_advanced_remote_target_before_push() {
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-conflict-remote-advanced";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "test");

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
    git_ok(updater.path(), &["checkout", "test"]);
    fs::write(updater.path().join("remote.txt"), "remote update\n").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote target update");
    git_ok(updater.path(), &["push", "origin", "test"]);

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue release conflict after target advanced");

    assert!(
        out.contains("origin/test advanced"),
        "validation should explain the remote-target retry: {out}"
    );
    assert_eq!(head_branch(dir.path()), feature);
    let released_conflict = git(bare.path(), &["show", "test:conflict.txt"]);
    assert!(
        released_conflict.status.success(),
        "test conflict file missing: {}",
        String::from_utf8_lossy(&released_conflict.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&released_conflict.stdout),
        "resolved\n"
    );
    let remote_file = git(bare.path(), &["show", "test:remote.txt"]);
    assert!(
        remote_file.status.success(),
        "test remote update missing: {}",
        String::from_utf8_lossy(&remote_file.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&remote_file.stdout),
        "remote update\n"
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
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    git_ok(dir.path(), &["push", "origin", "test"]);

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
    let main = status.main.expect("main release status");
    assert!(
        main.released_at.is_empty(),
        "main should not have a release timestamp"
    );
    assert_eq!(main.missing_commits, 2);
    let develop = status.develop.expect("develop release status");
    assert!(!develop.released_at.is_empty(), "missing release timestamp");
    assert_eq!(develop.missing_commits, 1);
    let test = status.test.expect("test release status");
    assert!(
        test.released_at.is_empty(),
        "test should not have a release timestamp"
    );
    assert_eq!(test.missing_commits, 2);
}

#[test]
fn release_branches_detects_each_deploy_branch_on_its_own() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let _cwd = CwdGuard::new(dir.path());
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), None);
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Test), None);
    assert!(!targets.any(), "a repo with only main deploys nothing");

    // alv.no shape: a test branch and no develop branch at all.
    git_ok(dir.path(), &["branch", "test"]);
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), None);
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Test), Some("test"));
    assert!(targets.any(), "a test branch alone is a deploy target");

    // Either spelling of the dev branch counts.
    git_ok(dir.path(), &["branch", "dev"]);
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), Some("dev"));

    // develop wins when a checkout has both spellings.
    git_ok(dir.path(), &["branch", "develop"]);
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), Some("develop"));
}

#[test]
fn release_status_reports_test_only_repository() {
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/test-only";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "released").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "released feature commit");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test").expect("release to test");

    let status = lg::git::branch_release_status(feature).expect("branch release status");
    assert!(
        status.develop.is_none(),
        "a repo without a develop branch has no develop status"
    );
    let test = status.test.expect("test release status");
    assert_eq!(test.missing_commits, 0);
    assert!(!test.released_at.is_empty(), "missing release timestamp");
    let main = status.main.expect("main release status");
    assert_eq!(main.missing_commits, 1);
}

/// A release can conflict at two different merges, and the one that goes wrong
/// first is `origin/main` into the deploy branch — before the feature has been
/// merged at all. Continuing must finish the release, not just push whatever
/// the deploy branch happens to hold.
#[test]
fn release_conflict_on_the_main_merge_still_releases_the_feature() {
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
    git_ok(dir.path(), &["push", "origin", "main"]);

    // test diverges from main on shared.txt...
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    // ...and then main moves on the same file, so merging it into test breaks.
    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    // The feature touches a different file, so it is not part of the conflict.
    let feature = "feature/release-main-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("the main merge should stop for manual resolution");
    assert_eq!(head_branch(dir.path()), "test");

    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release");

    assert_eq!(head_branch(dir.path()), feature);
    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(
        released.status.success(),
        "the feature never reached test: {}",
        String::from_utf8_lossy(&released.stderr)
    );
}

/// git reports conflicted paths relative to the repository, and lg never
/// changes the process working directory — so a file still full of markers must
/// not read as resolved just because it cannot be found next to the process.
#[test]
fn a_conflicted_file_is_read_from_the_repository_not_the_process_directory() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "theirs"]);
    fs::write(dir.path().join("conflict.txt"), "theirs\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "their side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "ours\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "our side");

    let merge = git(dir.path(), &["merge", "theirs"]);
    assert!(!merge.status.success(), "the merge should have conflicted");

    // The process sits somewhere else entirely, as lg's does.
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    let _cwd = CwdGuard::new(elsewhere.path());

    let staged = lg::git::with_repo(dir.path(), lg::git::stage_resolved_conflicts)
        .expect("stage resolved conflicts");
    assert!(
        staged.is_empty(),
        "a file that still holds markers must not be staged: {staged:?}"
    );
    let still_conflicted =
        lg::git::with_repo(dir.path(), lg::git::conflicted_files).expect("conflicted files");
    assert_eq!(still_conflicted, ["conflict.txt"]);

    // Resolved for real, it stages from the same distance.
    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let staged = lg::git::with_repo(dir.path(), lg::git::stage_resolved_conflicts)
        .expect("stage resolved conflicts");
    assert_eq!(staged, ["conflict.txt"]);
}

/// The whole round trip the loop was stuck in: the release conflicts, the
/// conflict is resolved, `v` continues it — and running the release again
/// finds nothing left to do. A second conflict here means the first one was
/// never really finished.
#[test]
fn a_resolved_release_conflict_does_not_come_back_on_the_next_release() {
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
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    let feature = "feature/loop";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());

    // First run stops on the origin/main merge.
    lg::git::flow_release_current(feature, "test").expect_err("the main merge conflicts");
    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release");
    assert_eq!(head_branch(dir.path()), feature);

    // Second run has nothing left to conflict over.
    let summary = lg::git::flow_release_current(feature, "test")
        .expect("the release should be clean the second time");
    assert!(
        summary.contains("released"),
        "unexpected summary: {summary}"
    );
    assert!(
        lg::git::conflicted_files()
            .expect("conflicted files")
            .is_empty(),
        "the resolved conflict came back"
    );

    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(
        released.status.success(),
        "the feature never reached test: {}",
        String::from_utf8_lossy(&released.stderr)
    );
    let shared = git(bare.path(), &["show", "test:shared.txt"]);
    assert_eq!(
        String::from_utf8_lossy(&shared.stdout),
        "resolved\n",
        "the resolution should be what test carries"
    );
}

/// The loop itself: a release run on top of an unfinished conflict used to
/// reset the deploy branch to its remote, throwing the resolution away and
/// walking into the same conflict again. It has to refuse instead, and say how
/// to get out.
#[test]
fn a_release_refuses_to_run_on_top_of_an_unfinished_conflict() {
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
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    let feature = "feature/refuse";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test").expect_err("the main merge conflicts");

    // Resolved, but not yet continued — the state the loop happened in.
    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    let before = git(dir.path(), &["rev-parse", "test"]);

    let err = lg::git::flow_release_current(feature, "test")
        .expect_err("a second release must not run over the resolution");
    let message = format!("{err:#}");
    assert!(
        message.contains("part-way through a merge"),
        "the refusal should say what is in the way: {message}"
    );
    assert!(
        message.contains("F") && message.contains("v") && message.contains("a"),
        "and which keys get out of it: {message}"
    );

    assert_eq!(
        String::from_utf8_lossy(&before.stdout),
        String::from_utf8_lossy(&git(dir.path(), &["rev-parse", "test"]).stdout),
        "the deploy branch must not have been reset under the resolution"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("shared.txt")).unwrap(),
        "resolved\n",
        "the resolution must survive the refusal"
    );

    // And continuing still works from there.
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release after the refusal");
    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(released.status.success(), "the feature should have landed");
}

/// The other half of the loop: the conflict was resolved *and committed* — by a
/// claude session, or by hand — so nothing is in progress any more, but the
/// merge commit is only local. Releasing again would reset the deploy branch to
/// its remote, drop that commit, and hit the same conflict.
#[test]
fn a_release_refuses_to_reset_a_deploy_branch_that_is_ahead_of_its_remote() {
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
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    let feature = "feature/committed";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test").expect_err("the main merge conflicts");

    // Resolved and committed on test, the way a session would leave it.
    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    git_ok(dir.path(), &["add", "shared.txt"]);
    commit_in(dir.path(), "resolve the main merge");
    assert!(
        lg::git::conflicted_files()
            .expect("conflicted files")
            .is_empty(),
        "nothing should be in progress any more"
    );
    let resolved = git(dir.path(), &["rev-parse", "test"]);

    git_ok(dir.path(), &["checkout", feature]);
    let err = lg::git::flow_release_current(feature, "test")
        .expect_err("a release must not reset test over the resolution");
    let message = format!("{err:#}");
    assert!(
        message.contains("would lose them") && message.contains("release again"),
        "the refusal should say what is at stake and what to do: {message}"
    );

    assert_eq!(
        String::from_utf8_lossy(&resolved.stdout),
        String::from_utf8_lossy(&git(dir.path(), &["rev-parse", "test"]).stdout),
        "the committed resolution must still be there"
    );

    // Continuing from there finishes the release it belongs to.
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release");
    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(released.status.success(), "the feature should have landed");
    let shared = git(bare.path(), &["show", "test:shared.txt"]);
    assert_eq!(String::from_utf8_lossy(&shared.stdout), "resolved\n");
}

/// The bug that made the release loop: git reports its in-progress markers
/// relative to git's own directory, and lg points git at a repository with `-C`
/// instead of moving the process. Tested against the process directory they all
/// read as absent, so a conflicted merge looked finished — validation skipped
/// the commit that completes it and the push moved nothing.
#[test]
fn a_merge_in_progress_is_detected_from_outside_the_repository() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "theirs"]);
    fs::write(dir.path().join("conflict.txt"), "theirs\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "their side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "ours\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "our side");

    let merge = git(dir.path(), &["merge", "theirs"]);
    assert!(!merge.status.success(), "the merge should have conflicted");

    // The process sits somewhere else entirely, as lg's does.
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    let _cwd = CwdGuard::new(elsewhere.path());

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::with_repo(dir.path(), || {
        lg::git::validate_conflict_resolution(lg::git::Followup::default())
    })
    .expect("validate the resolution");

    assert!(
        !out.contains("no merge, rebase, or cherry-pick operation is in progress"),
        "the merge in progress should have been found: {out}"
    );
    let parents = git(dir.path(), &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&parents.stdout)
            .split_whitespace()
            .count(),
        3,
        "validation must commit the merge, leaving a commit with two parents"
    );
    assert!(
        lg::git::with_repo(dir.path(), lg::git::conflicted_files)
            .expect("conflicted files")
            .is_empty(),
        "nothing should still be conflicted"
    );
}
