//! Local and remote branches: listing, checking out, creating and cleaning up.

use super::common::*;

#[test]
fn transfer_diff_to_feature_branch_recreates_branch_changes_on_main() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    fs::write("app.txt", "one\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/messy"]);
    fs::write("app.txt", "one\ntwo\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "first messy commit");
    fs::write("new.txt", "new file\n").unwrap();
    stage_in(dir.path(), "new.txt");
    commit_in(dir.path(), "second messy commit");

    let status =
        lg::git::flow_transfer_diff_to_feature_branch("feature/messy", "feature/clean").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/clean");
    assert!(
        status.contains("transferred feature/messy diff"),
        "unexpected status: {status}"
    );
    let log = git(
        dir.path(),
        &["log", "--oneline", "--decorate", "--max-count=3"],
    );
    let log_text = String::from_utf8_lossy(&log.stdout);
    assert!(
        !log_text.contains("messy commit"),
        "new branch should not copy source commits: {log_text}"
    );

    let cached = git(dir.path(), &["diff", "--cached", "--name-only"]);
    let cached_text = String::from_utf8_lossy(&cached.stdout);
    assert!(cached_text.contains("app.txt"), "{cached_text}");
    assert!(cached_text.contains("new.txt"), "{cached_text}");
    assert_eq!(fs::read_to_string("app.txt").unwrap(), "one\ntwo\n");
    assert_eq!(fs::read_to_string("new.txt").unwrap(), "new file\n");
}

#[test]
fn create_feature_branch_from_main_pushes_and_sets_upstream() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::flow_create_feature_branch("main", "feature/tracked-create").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/tracked-create");
    assert!(
        out.contains("tracking origin/feature/tracked-create"),
        "status should mention upstream: {out}"
    );
    let upstream = git(
        dir.path(),
        &[
            "rev-parse",
            "--abbrev-ref",
            "feature/tracked-create@{upstream}",
        ],
    );
    assert!(
        upstream.status.success(),
        "upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/feature/tracked-create"
    );
    let remote = git(
        bare.path(),
        &["rev-parse", "--verify", "refs/heads/feature/tracked-create"],
    );
    assert!(
        remote.status.success(),
        "remote feature branch was not created: {}",
        String::from_utf8_lossy(&remote.stderr)
    );
}

#[test]
fn list_branches_orders_newest_commit_first() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");

    git_ok(dir.path(), &["checkout", "-b", "older"]);
    fs::write(dir.path().join("older.txt"), "older\n").unwrap();
    stage_in(dir.path(), "older.txt");
    commit_in_at(dir.path(), "older branch", "2026-01-01T00:00:00Z");

    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "newer"]);
    fs::write(dir.path().join("newer.txt"), "newer\n").unwrap();
    stage_in(dir.path(), "newer.txt");
    commit_in_at(dir.path(), "newer branch", "2026-01-02T00:00:00Z");

    let _cwd = CwdGuard::new(dir.path());
    let branches = lg::git::list_branches().unwrap();
    let names = branches
        .iter()
        .map(|branch| branch.name.as_str())
        .collect::<Vec<_>>();
    let newer = names.iter().position(|name| *name == "newer").unwrap();
    let older = names.iter().position(|name| *name == "older").unwrap();
    assert!(
        newer < older,
        "newer branch should sort before older branch: {names:?}"
    );
}

#[test]
fn list_branches_reports_commits_behind_main() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");

    git_ok(dir.path(), &["checkout", "-b", "feature/stale"]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main update");

    let _cwd = CwdGuard::new(dir.path());
    let branches = lg::git::list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/stale")
        .expect("feature/stale branch should be listed");
    let main = branches
        .iter()
        .find(|branch| branch.name == "main")
        .expect("main branch should be listed");

    assert_eq!(feature.behind_main, 1);
    assert_eq!(main.behind_main, 0);
}

/// Every branch gets its own count, however many there are: the counts come
/// from one call for all of them now, and must not bleed into each other.
#[test]
fn list_branches_counts_each_branch_behind_main_separately() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(dir.path(), &["branch", "three-behind"]);
    for n in 1..=2 {
        fs::write(dir.path().join(format!("m{n}.txt")), "main\n").unwrap();
        stage_in(dir.path(), &format!("m{n}.txt"));
        commit_in(dir.path(), &format!("main {n}"));
    }
    git_ok(dir.path(), &["branch", "one-behind"]);
    fs::write(dir.path().join("m3.txt"), "main\n").unwrap();
    stage_in(dir.path(), "m3.txt");
    commit_in(dir.path(), "main 3");
    git_ok(dir.path(), &["branch", "level"]);

    let _cwd = CwdGuard::new(dir.path());
    let branches = lg::git::list_branches().unwrap();
    let behind = |name: &str| {
        branches
            .iter()
            .find(|branch| branch.name == name)
            .unwrap_or_else(|| panic!("{name} should be listed: {branches:?}"))
            .behind_main
    };
    assert_eq!(behind("three-behind"), 3);
    assert_eq!(behind("one-behind"), 1);
    assert_eq!(behind("level"), 0);
    assert_eq!(behind("main"), 0);
}

#[test]
fn checkout_branch_stashes_unstaged_changes_before_switching() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(dir.path(), &["checkout", "-b", "feature/target"]);
    fs::write(dir.path().join("target.txt"), "target\n").unwrap();
    stage_in(dir.path(), "target.txt");
    commit_in(dir.path(), "target branch");
    git_ok(dir.path(), &["checkout", "main"]);

    fs::write(dir.path().join("README.md"), "dirty\n").unwrap();
    let _cwd = CwdGuard::new(dir.path());

    let out = lg::git::checkout_branch("feature/target").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/target");
    assert!(
        out.contains("applied stashed local changes after checkout"),
        "checkout output should mention stash: {out}"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.contains(&"README.md".to_string()) && staged.is_empty(),
        "unstaged change should be applied on target branch"
    );
    assert!(
        !stash_list(dir.path()).contains("lg: auto-stash before checkout"),
        "stash should be popped after checkout"
    );
}

#[test]
fn checkout_branch_stashes_staged_changes_before_switching() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(dir.path(), &["checkout", "-b", "feature/target"]);
    fs::write(dir.path().join("target.txt"), "target\n").unwrap();
    stage_in(dir.path(), "target.txt");
    commit_in(dir.path(), "target branch");
    git_ok(dir.path(), &["checkout", "main"]);

    fs::write(dir.path().join("staged.txt"), "staged\n").unwrap();
    stage_in(dir.path(), "staged.txt");
    let _cwd = CwdGuard::new(dir.path());

    let out = lg::git::checkout_branch("feature/target").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/target");
    assert!(
        out.contains("applied stashed local changes after checkout"),
        "checkout output should mention stash: {out}"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.is_empty() && staged.contains(&"staged.txt".to_string()),
        "staged change should be applied on target branch"
    );
    assert!(
        !stash_list(dir.path()).contains("lg: auto-stash before checkout"),
        "stash should be popped after checkout"
    );
}

#[test]
fn remote_branches_can_be_listed_and_checked_out_locally() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "feature/remote"]);
    fs::write(dir.path().join("remote.txt"), "remote\n").unwrap();
    stage_in(dir.path(), "remote.txt");
    commit_in(dir.path(), "remote branch");
    git_ok(dir.path(), &["push", "-u", "origin", "feature/remote"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["branch", "-D", "feature/remote"]);

    let _cwd = CwdGuard::new(dir.path());
    let remotes = lg::git::list_remote_branches().unwrap();
    let remote = remotes
        .iter()
        .find(|branch| branch.name == "origin/feature/remote")
        .expect("origin/feature/remote should be listed");
    assert_eq!(remote.remote, "origin");
    assert_eq!(remote.local_name, "feature/remote");
    assert!(remote.last_commit_unix.is_some());

    lg::git::checkout_remote_branch("origin/feature/remote").unwrap();
    assert_eq!(head_branch(dir.path()), "feature/remote");

    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "feature/remote@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/feature/remote"
    );
}

#[test]
fn set_branch_upstream_tracks_existing_remote_branch() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "solution"]);
    fs::write(dir.path().join("solution.txt"), "solution\n").unwrap();
    stage_in(dir.path(), "solution.txt");
    commit_in(dir.path(), "solution branch");
    git_ok(dir.path(), &["push", "origin", "solution"]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::set_branch_upstream("solution", "origin/solution").unwrap();

    assert_eq!(out, "solution tracks origin/solution");
    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "solution@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/solution"
    );
}

#[test]
fn checkout_remote_branch_applies_dirty_changes_after_switching() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "feature/remote"]);
    fs::write(dir.path().join("remote.txt"), "remote\n").unwrap();
    stage_in(dir.path(), "remote.txt");
    commit_in(dir.path(), "remote branch");
    git_ok(dir.path(), &["push", "-u", "origin", "feature/remote"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["branch", "-D", "feature/remote"]);

    fs::write(dir.path().join("README.md"), "dirty\n").unwrap();
    let _cwd = CwdGuard::new(dir.path());

    let out = lg::git::checkout_remote_branch("origin/feature/remote").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/remote");
    assert!(
        out.contains("applied stashed local changes after checkout"),
        "checkout output should mention stash: {out}"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.contains(&"README.md".to_string()) && staged.is_empty(),
        "dirty change should be applied on remote tracking branch"
    );
    assert!(
        !stash_list(dir.path()).contains("lg: auto-stash before remote checkout"),
        "stash should be popped after remote checkout"
    );
}

#[test]
fn list_remote_branches_orders_newest_commit_first() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "older"]);
    fs::write(dir.path().join("older.txt"), "older\n").unwrap();
    stage_in(dir.path(), "older.txt");
    commit_in_at(dir.path(), "older remote branch", "2026-01-01T00:00:00Z");
    git_ok(dir.path(), &["push", "-u", "origin", "older"]);

    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "newer"]);
    fs::write(dir.path().join("newer.txt"), "newer\n").unwrap();
    stage_in(dir.path(), "newer.txt");
    commit_in_at(dir.path(), "newer remote branch", "2026-01-02T00:00:00Z");
    git_ok(dir.path(), &["push", "-u", "origin", "newer"]);

    let _cwd = CwdGuard::new(dir.path());
    let branches = lg::git::list_remote_branches().unwrap();
    let names = branches
        .iter()
        .map(|branch| branch.name.as_str())
        .collect::<Vec<_>>();
    let newer = names
        .iter()
        .position(|name| *name == "origin/newer")
        .unwrap();
    let older = names
        .iter()
        .position(|name| *name == "origin/older")
        .unwrap();
    assert!(
        newer < older,
        "newer remote branch should sort before older remote branch: {names:?}"
    );
}

#[test]
fn delete_current_feature_branch_checks_out_main_first() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    let branch = "feature/delete-current";
    git_ok(dir.path(), &["checkout", "-b", branch]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::delete_local_branch(branch, false).expect("delete current branch");

    assert_eq!(head_branch(dir.path()), "main");
    assert!(
        out.contains("checked out main"),
        "delete output should mention checkout: {out}"
    );
    let deleted = git(dir.path(), &["rev-parse", "--verify", branch]);
    assert!(!deleted.status.success(), "branch should be deleted");
}

/// Having no upstream is also what a branch nobody pushed yet looks like, so
/// cleaning orphans deletes only what the checkout already has, and never
/// touches the backups lg takes before a flow.
#[test]
fn cleaning_orphans_keeps_unmerged_branches_and_lg_backups() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);
    git_ok(dir.path(), &["branch", "merged-orphan"]);
    git_ok(dir.path(), &["checkout", "-b", "unmerged-orphan"]);
    fs::write(dir.path().join("work.txt"), "only here\n").unwrap();
    stage_in(dir.path(), "work.txt");
    commit_in(dir.path(), "unpushed work");
    git_ok(
        dir.path(),
        &["branch", "lg/backup/merge-main-main-1", "unmerged-orphan"],
    );
    git_ok(dir.path(), &["checkout", "main"]);

    let listed = lg::git::with_repo(dir.path(), || lg::git::orphan_branches("main")).unwrap();
    let unmerged = listed
        .iter()
        .find(|branch| branch.name == "unmerged-orphan")
        .expect("the unmerged branch is listed");
    assert_eq!((unmerged.unmerged, unmerged.unpushed), (1, 1));
    assert!(
        listed
            .iter()
            .all(|branch| !branch.name.starts_with("lg/backup/")),
        "{listed:?}"
    );

    let report =
        lg::git::with_repo(dir.path(), || lg::git::flow_clean_orphan_branches("main")).unwrap();

    let exists = |name: &str| {
        git(
            dir.path(),
            &["rev-parse", "--verify", &format!("refs/heads/{name}")],
        )
        .status
        .success()
    };
    assert!(!exists("merged-orphan"), "{report}");
    assert!(exists("unmerged-orphan"), "{report}");
    assert!(exists("lg/backup/merge-main-main-1"), "{report}");
    assert!(exists("main"));
    assert!(report.contains("deleted merged-orphan"), "{report}");
    assert!(report.contains("kept unmerged-orphan"), "{report}");
}
