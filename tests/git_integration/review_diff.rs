//! The diffs a review reads: the branch against main, and what is not pushed.

use super::common::*;

#[test]
fn the_branch_diff_reads_the_working_tree_untracked_files_included() {
    let dir = init_repo();
    fs::create_dir_all(dir.path().join("src/main/kotlin")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hello\"\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/worktree-review"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hello worktree\"\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/main/kotlin/NewFlow.kt"),
        "class NewFlow {\n    fun newFlow() = \"worktree\"\n}\n",
    )
    .unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let diff = lg::git::branch_diff_against_main().unwrap().diff;

    assert!(diff.contains("\"hello worktree\""), "{diff}");
    assert!(diff.contains("b/src/main/kotlin/NewFlow.kt"), "{diff}");
    assert!(diff.contains("fun newFlow"), "{diff}");
}

#[test]
fn the_branch_diff_leaves_out_whitespace_only_changes() {
    let dir = init_repo();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/format-only"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() {\n        println!(\"hello\");\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "format greeting");

    let _cwd = CwdGuard::new(dir.path());
    let branch = lg::git::branch_diff_against_main().unwrap();

    assert!(
        branch.diff.trim().is_empty(),
        "whitespace-only branch diff should be empty: {}",
        branch.diff
    );
    assert!(
        branch.commits.iter().any(|c| c.contains("format greeting")),
        "{:?}",
        branch.commits
    );
}

#[test]
fn the_branch_diff_on_main_holds_its_uncommitted_local_changes() {
    let dir = init_repo();
    fs::write(dir.path().join("tracked.txt"), "main\n").unwrap();
    stage_in(dir.path(), "tracked.txt");
    commit_in(dir.path(), "initial tracked");

    fs::write(dir.path().join("tracked.txt"), "local only\n").unwrap();
    fs::write(dir.path().join("scratch.txt"), "untracked local\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let diff = lg::git::branch_diff_against_main().unwrap().diff;

    assert!(
        diff.contains("local only") && diff.contains("scratch.txt"),
        "local changes should be included: {diff}"
    );
}

/// What is not pushed is read against where the branch was pushed: the
/// commits and edits since, and nothing the remote already has.
#[test]
fn the_unpushed_diff_holds_only_what_the_upstream_lacks() {
    let dir = init_repo();
    let remote = tempfile::tempdir().expect("tempdir");
    git_ok(remote.path(), &["init", "--bare", "-b", "main"]);
    fs::write(dir.path().join("a.txt"), "base\n").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "base");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["checkout", "-b", "feature/push"]);
    fs::write(dir.path().join("a.txt"), "pushed\n").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "pushed change");
    git_ok(dir.path(), &["push", "-u", "origin", "feature/push"]);
    fs::write(dir.path().join("b.txt"), "committed, not pushed\n").unwrap();
    stage_in(dir.path(), "b.txt");
    commit_in(dir.path(), "local commit");
    fs::write(dir.path().join("c.txt"), "not committed\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let unpushed = lg::git::unpushed_diff().unwrap();

    assert_eq!(unpushed.base_ref, "origin/feature/push");
    assert!(unpushed.diff.contains("b/b.txt"), "{}", unpushed.diff);
    assert!(unpushed.diff.contains("b/c.txt"), "{}", unpushed.diff);
    assert!(!unpushed.diff.contains("b/a.txt"), "{}", unpushed.diff);
    assert!(
        unpushed.commits.iter().any(|c| c.contains("local commit"))
            && !unpushed.commits.iter().any(|c| c.contains("pushed change")),
        "{:?}",
        unpushed.commits
    );
}
