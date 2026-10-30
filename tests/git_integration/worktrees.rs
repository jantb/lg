//! Linked worktrees: listing, adding and removing them, and landing their branches.

use super::common::*;

#[test]
fn worktrees_are_listed_from_any_checkout_with_their_dirty_state() {
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
            "feat/x",
            linked.to_str().expect("worktree path"),
        ],
    );
    fs::write(linked.join("work.txt"), "in progress").unwrap();

    let from_main = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(from_main.len(), 2, "{from_main:#?}");
    assert!(from_main[0].is_main);
    assert_eq!(from_main[0].branch.as_deref(), Some("main"));
    assert!(!from_main[0].has_changes, "main worktree should be clean");
    assert_eq!(from_main[1].branch.as_deref(), Some("feat/x"));
    assert!(
        from_main[1].has_changes,
        "the linked worktree has an untracked file"
    );

    // Asking a linked worktree returns the same set, main worktree first.
    let from_linked = lg::git::with_repo(&linked, lg::git::worktrees).expect("worktrees");
    let branches: Vec<_> = from_linked
        .iter()
        .map(|worktree| worktree.branch.as_deref())
        .collect();
    assert_eq!(branches, [Some("main"), Some("feat/x")]);
    assert!(from_linked[0].is_main);
}

#[test]
fn a_worktree_can_be_added_for_a_new_branch_and_removed_again() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("nested").join("feat-x");

    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "feat/x", "main")
    })
    .expect("add worktree");

    assert!(path.join("init.txt").is_file(), "worktree was checked out");
    assert_eq!(head_branch(&path), "feat/x", "on the new branch");

    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(listed.len(), 2, "{listed:#?}");

    lg::git::with_repo(repo.path(), || lg::git::worktree_remove(&path, false))
        .expect("remove worktree");
    assert!(!path.exists(), "the directory is gone");
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(listed.len(), 1, "{listed:#?}");
}

#[test]
fn adding_a_worktree_checks_out_a_branch_that_already_exists() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");
    git_ok(repo.path(), &["branch", "existing"]);

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("existing");

    // The base is ignored for a branch that already exists, so a nonsense one
    // must not stop it being checked out.
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "existing", "no/such/ref")
    })
    .expect("add worktree");

    assert_eq!(head_branch(&path), "existing");
}

#[test]
fn adding_a_worktree_reports_bad_input_instead_of_running_git() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("x");

    let empty = lg::git::with_repo(repo.path(), || lg::git::worktree_add(&path, "  ", "main"));
    assert!(
        empty.unwrap_err().to_string().contains("cannot be empty"),
        "an empty branch name is caught before git runs"
    );

    let invalid = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "bad..name", "main")
    });
    assert!(
        invalid.unwrap_err().to_string().contains("invalid branch"),
        "an invalid branch name is caught before git runs"
    );

    let unknown_base = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "fresh", "no/such/ref")
    });
    assert!(
        unknown_base
            .unwrap_err()
            .to_string()
            .contains("unknown base ref"),
        "an unknown base is named as such"
    );
    assert!(!path.exists(), "nothing was created");
}

#[test]
fn removing_a_dirty_worktree_needs_force() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("dirty");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "dirty", "main")
    })
    .expect("add worktree");
    fs::write(path.join("scratch.txt"), "work in progress").unwrap();

    let refused = lg::git::with_repo(repo.path(), || lg::git::worktree_remove(&path, false));
    assert!(refused.is_err(), "git should refuse to discard the work");
    assert!(path.exists(), "the worktree survives a refusal");

    lg::git::with_repo(repo.path(), || lg::git::worktree_remove(&path, true))
        .expect("force removal");
    assert!(!path.exists());
}

#[test]
fn pruning_forgets_a_worktree_whose_directory_is_gone() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("gone");
    lg::git::with_repo(repo.path(), || lg::git::worktree_add(&path, "gone", "main"))
        .expect("add worktree");
    fs::remove_dir_all(&path).expect("remove worktree directory");

    lg::git::with_repo(repo.path(), lg::git::worktree_prune).expect("prune");
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(
        listed.len(),
        1,
        "only the main worktree is left: {listed:#?}"
    );
}

#[test]
fn a_removed_worktree_directory_is_reported_as_prunable() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let linked = elsewhere.path().join("gone");
    git_ok(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "gone",
            linked.to_str().expect("worktree path"),
        ],
    );
    fs::remove_dir_all(&linked).expect("remove worktree directory");

    let worktrees = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    let missing = worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some("gone"))
        .expect("the removed worktree is still registered");
    assert!(
        missing.prunable.is_some(),
        "git should call it prunable: {missing:#?}"
    );
    assert!(missing.is_missing());
}

/// Repository with one commit, a bare `origin` it tracks, and a linked
/// worktree on `feat/x` carrying one commit of its own — the shape both
/// handover flows start from.
fn repo_with_landable_worktree() -> (TempDir, TempDir, TempDir, std::path::PathBuf) {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init\n").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        repo.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(repo.path(), &["push", "-u", "origin", "main"]);

    let elsewhere = tempfile::tempdir().expect("worktree tempdir");
    let path = elsewhere.path().join("feat-x");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "feat/x", "main")
    })
    .expect("add worktree");
    fs::write(path.join("feature.txt"), "work\n").unwrap();
    stage_in(&path, "feature.txt");
    commit_in(&path, "add feature");

    (repo, bare, elsewhere, path)
}

#[test]
fn landing_a_worktree_merges_it_into_main_and_clears_the_branch_away() {
    let (repo, bare, _elsewhere, path) = repo_with_landable_worktree();
    git_ok(&path, &["push", "-u", "origin", "feat/x"]);

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect("land worktree");

    assert!(
        repo.path().join("feature.txt").is_file(),
        "main carries the work: {report}"
    );
    assert_eq!(head_branch(repo.path()), "main", "main stayed checked out");
    assert!(!path.exists(), "the worktree directory is gone: {report}");
    assert!(
        !branch_list(repo.path()).contains("feat/x"),
        "the local branch is gone: {report}"
    );

    let pushed = git(bare.path(), &["rev-parse", "refs/heads/main"]);
    let local = git(repo.path(), &["rev-parse", "refs/heads/main"]);
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        String::from_utf8_lossy(&local.stdout).trim(),
        "main was pushed: {report}"
    );
    assert!(
        !git(bare.path(), &["rev-parse", "--verify", "refs/heads/feat/x"])
            .status
            .success(),
        "the remote branch is gone: {report}"
    );
}

#[test]
fn landing_a_worktree_without_a_remote_still_merges_and_cleans_up() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init\n").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("worktree tempdir");
    let path = elsewhere.path().join("feat-x");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "feat/x", "main")
    })
    .expect("add worktree");
    fs::write(path.join("feature.txt"), "work\n").unwrap();
    stage_in(&path, "feature.txt");
    commit_in(&path, "add feature");

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect("land worktree");

    assert!(repo.path().join("feature.txt").is_file(), "{report}");
    assert!(!path.exists(), "{report}");
    assert!(!branch_list(repo.path()).contains("feat/x"), "{report}");
}

#[test]
fn a_conflicting_land_is_aborted_and_leaves_main_where_it_was() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("shared.txt"), "from the branch\n").unwrap();
    stage_in(&path, "shared.txt");
    commit_in(&path, "branch writes shared");
    fs::write(repo.path().join("shared.txt"), "from main\n").unwrap();
    stage_in(repo.path(), "shared.txt");
    commit_in(repo.path(), "main writes shared");
    let before = git(repo.path(), &["rev-parse", "HEAD"]);

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect_err("the merge conflicts");

    assert!(
        err.to_string().contains("by hand"),
        "the error says what to do next: {err}"
    );
    let after = git(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&before.stdout).trim(),
        String::from_utf8_lossy(&after.stdout).trim(),
        "main is back where it started"
    );
    assert!(
        !repo.path().join(".git/MERGE_HEAD").exists(),
        "the merge was aborted, not left open"
    );
    assert!(path.exists(), "the worktree is untouched");
    assert!(branch_list(repo.path()).contains("feat/x"));
}

/// The case landing is stuck on: main moved on, so the merge into main is no
/// longer a fast-forward. Syncing first moves the merge into the worktree,
/// after which landing goes through.
#[test]
fn syncing_a_worktree_behind_main_lets_it_land() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(repo.path().join("moved-on.txt"), "main moved on\n").unwrap();
    stage_in(repo.path(), "moved-on.txt");
    commit_in(repo.path(), "main moves on");
    git_ok(repo.path(), &["push", "origin", "main"]);

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect("sync worktree");

    assert!(
        path.join("moved-on.txt").is_file(),
        "the branch carries main's work now: {report}"
    );
    assert_eq!(
        head_branch(repo.path()),
        "main",
        "main stayed checked out where it was"
    );

    let landed = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect("land after syncing");
    assert!(repo.path().join("feature.txt").is_file(), "{landed}");
    assert!(!path.exists(), "{landed}");
}

/// A sync conflict is the branch's to resolve, so it is left open rather than
/// aborted — that is what lets the conflict modal drive it to a finish.
#[test]
fn a_conflicting_sync_stays_open_in_the_worktree_and_leaves_main_alone() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("shared.txt"), "from the branch\n").unwrap();
    stage_in(&path, "shared.txt");
    commit_in(&path, "branch writes shared");
    fs::write(repo.path().join("shared.txt"), "from main\n").unwrap();
    stage_in(repo.path(), "shared.txt");
    commit_in(repo.path(), "main writes shared");
    let main_before = git(repo.path(), &["rev-parse", "HEAD"]);

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect_err("the merge conflicts");
    assert!(
        err.to_string().contains("shared.txt"),
        "the error names the conflicted file: {err}"
    );

    let conflicts = lg::git::with_repo(&path, lg::git::conflicted_files).expect("conflicted files");
    assert_eq!(
        conflicts,
        vec!["shared.txt".to_string()],
        "the conflict is left open in the worktree for the modal to pick up"
    );

    let main_after = git(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&main_before.stdout).trim(),
        String::from_utf8_lossy(&main_after.stdout).trim(),
        "main never moved"
    );
    assert!(
        !repo.path().join(".git/MERGE_HEAD").exists(),
        "the main checkout has no merge in progress"
    );
}

#[test]
fn syncing_a_worktree_that_is_already_current_changes_nothing() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    let before = git(&path, &["rev-parse", "HEAD"]);

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect("sync worktree");

    assert!(
        report.contains("already up to date"),
        "it says there was nothing to do: {report}"
    );
    let after = git(&path, &["rev-parse", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&before.stdout).trim(),
        String::from_utf8_lossy(&after.stdout).trim(),
        "the branch did not move"
    );
}

#[test]
fn syncing_refuses_while_the_worktree_has_uncommitted_work() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(repo.path().join("moved-on.txt"), "main moved on\n").unwrap();
    stage_in(repo.path(), "moved-on.txt");
    commit_in(repo.path(), "main moves on");
    fs::write(path.join("scratch.txt"), "unsaved\n").unwrap();
    stage_in(&path, "scratch.txt");

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect_err("the worktree is dirty");

    assert!(
        err.to_string().contains("commit or discard"),
        "the error says what to do first: {err}"
    );
    assert!(
        !path.join("moved-on.txt").exists(),
        "nothing was merged into a dirty worktree"
    );
}

#[test]
fn a_handover_refuses_while_the_worktree_has_uncommitted_work() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("scratch.txt"), "unsaved\n").unwrap();

    for outcome in [
        lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x")),
        lg::git::with_repo(repo.path(), || {
            lg::git::worktree_bring_home(&path, "feat/x")
        }),
    ] {
        let err = outcome.expect_err("uncommitted work stops the handover");
        assert!(
            err.to_string().contains("commit or discard"),
            "the error names the way out: {err}"
        );
    }
    assert!(path.exists(), "the worktree is untouched");
    assert!(!repo.path().join("feature.txt").is_file(), "nothing merged");
}

#[test]
fn a_handover_refuses_a_branch_the_worktree_has_since_left() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    git_ok(&path, &["checkout", "-b", "feat/y"]);

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect_err("the confirmed branch is no longer there");

    assert!(
        err.to_string().contains("on feat/y now, not feat/x"),
        "the error names both branches: {err}"
    );
    assert!(path.exists(), "the worktree is untouched");
}

#[test]
fn bringing_a_branch_home_checks_it_out_in_the_main_worktree() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();

    let report = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_bring_home(&path, "feat/x")
    })
    .expect("bring the branch home");

    assert!(!path.exists(), "the worktree directory is gone: {report}");
    assert_eq!(
        head_branch(repo.path()),
        "feat/x",
        "the branch moved to the main checkout: {report}"
    );
    assert!(
        repo.path().join("feature.txt").is_file(),
        "its work came along: {report}"
    );
    assert!(
        branch_list(repo.path()).contains("feat/x"),
        "the branch is kept, not merged away: {report}"
    );
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(
        listed.len(),
        1,
        "only the main checkout is left: {listed:#?}"
    );
}

#[test]
fn bringing_a_branch_home_refuses_while_the_main_checkout_is_dirty() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(repo.path().join("init.txt"), "edited\n").unwrap();

    let err = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_bring_home(&path, "feat/x")
    })
    .expect_err("a dirty main checkout has nowhere to put the branch");

    assert!(
        err.to_string().contains("commit or stash"),
        "the error names the way out: {err}"
    );
    assert!(path.exists(), "the worktree is untouched");
}

#[test]
fn a_worktree_reports_how_far_its_branch_has_run_ahead_of_main() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("second.txt"), "more\n").unwrap();
    stage_in(&path, "second.txt");
    commit_in(&path, "second commit");

    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    let main = listed.iter().find(|w| w.is_main).expect("main worktree");
    let feature = listed
        .iter()
        .find(|w| w.branch.as_deref() == Some("feat/x"))
        .expect("feature worktree");

    assert_eq!(feature.unmerged, Some(2), "two commits main does not have");
    assert_eq!(main.unmerged, None, "main is not ahead of itself");

    // Landing empties it, and the row should say so before the cleanup runs.
    git_ok(repo.path(), &["merge", "--no-edit", "feat/x"]);
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    let feature = listed
        .iter()
        .find(|w| w.branch.as_deref() == Some("feat/x"))
        .expect("feature worktree");
    assert_eq!(feature.unmerged, Some(0), "merged, so only cleanup is left");
}
