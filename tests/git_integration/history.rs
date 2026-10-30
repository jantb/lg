//! Rewriting history: amend, revert and cherry-pick.

use super::common::*;

#[test]
fn amending_replaces_the_last_commit_with_the_staged_changes() {
    let dir = thirty_lines_repo();
    fs::write(dir.path().join("extra.txt"), "more\n").unwrap();
    commit_all(dir.path(), "second");
    let parent = stdout_of(dir.path(), &["rev-parse", "HEAD~1"]);
    fs::write(dir.path().join("late.txt"), "forgotten\n").unwrap();
    stage_in(dir.path(), "late.txt");

    let message = lg::git::with_repo(dir.path(), lg::git::head_commit_message).unwrap();
    assert_eq!(message, "second");
    lg::git::with_repo(dir.path(), || lg::git::amend_commit("second, complete")).unwrap();

    assert_eq!(
        stdout_of(dir.path(), &["log", "-1", "--format=%s"]).trim(),
        "second, complete"
    );
    assert_eq!(stdout_of(dir.path(), &["rev-parse", "HEAD~1"]), parent);
    let files = stdout_of(dir.path(), &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        files.contains("late.txt") && files.contains("extra.txt"),
        "{files}"
    );
}

#[test]
fn a_pushed_head_is_reported_as_published_and_an_unpushed_one_is_not() {
    let (dir, _bare) = repo_with_origin();
    assert_eq!(
        lg::git::with_repo(dir.path(), lg::git::head_published_on).as_deref(),
        Some("origin/main")
    );
    fs::write(dir.path().join("local.txt"), "local\n").unwrap();
    commit_all(dir.path(), "local only");
    assert_eq!(
        lg::git::with_repo(dir.path(), lg::git::head_published_on),
        None
    );
}

#[test]
fn reverting_a_commit_adds_one_that_undoes_it() {
    let dir = thirty_lines_repo();
    fs::write(dir.path().join("oops.txt"), "mistake\n").unwrap();
    commit_all(dir.path(), "add oops");
    let sha = stdout_of(dir.path(), &["rev-parse", "--short", "HEAD"]);

    lg::git::with_repo(dir.path(), || lg::git::revert_commit(sha.trim())).unwrap();

    assert!(!dir.path().join("oops.txt").exists());
    let subject = stdout_of(dir.path(), &["log", "-1", "--format=%s"]);
    assert!(subject.starts_with("Revert \"add oops\""), "{subject}");
    assert_eq!(
        stdout_of(dir.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "3"
    );
}

#[test]
fn a_conflicting_revert_waits_for_the_conflict_and_can_be_continued() {
    let dir = thirty_lines_repo();
    fs::write(dir.path().join("notes.txt"), "first rewrite\n").unwrap();
    commit_all(dir.path(), "rewrite");
    let rewrite = stdout_of(dir.path(), &["rev-parse", "HEAD"]);
    fs::write(dir.path().join("notes.txt"), "second rewrite\n").unwrap();
    commit_all(dir.path(), "rewrite again");

    let err = lg::git::with_repo(dir.path(), || lg::git::revert_commit(rewrite.trim()))
        .expect_err("undoing the first rewrite conflicts with the second");
    assert!(err.to_string().contains("CONFLICT"), "{err}");
    let conflicts = lg::git::with_repo(dir.path(), lg::git::conflicted_files).unwrap();
    assert_eq!(conflicts, vec!["notes.txt".to_string()]);

    fs::write(dir.path().join("notes.txt"), "settled\n").unwrap();
    lg::git::with_repo(dir.path(), || {
        lg::git::validate_conflict_resolution(lg::git::Followup::default())
    })
    .unwrap();
    let subject = stdout_of(dir.path(), &["log", "-1", "--format=%s"]);
    assert!(subject.starts_with("Revert \"rewrite\""), "{subject}");
    assert!(
        !git(dir.path(), &["rev-parse", "--verify", "REVERT_HEAD"])
            .status
            .success(),
        "the revert is finished"
    );
}

#[test]
fn a_conflicting_revert_can_be_aborted() {
    let dir = thirty_lines_repo();
    fs::write(dir.path().join("notes.txt"), "first rewrite\n").unwrap();
    commit_all(dir.path(), "rewrite");
    let rewrite = stdout_of(dir.path(), &["rev-parse", "HEAD"]);
    fs::write(dir.path().join("notes.txt"), "second rewrite\n").unwrap();
    commit_all(dir.path(), "rewrite again");
    let head = stdout_of(dir.path(), &["rev-parse", "HEAD"]);
    let _ = lg::git::with_repo(dir.path(), || lg::git::revert_commit(rewrite.trim()));

    lg::git::with_repo(dir.path(), lg::git::abort_in_progress_operation).unwrap();

    assert_eq!(stdout_of(dir.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read_to_string(dir.path().join("notes.txt")).unwrap(),
        "second rewrite\n"
    );
}

#[test]
fn cherry_picking_brings_another_branchs_commit_onto_this_one() {
    let dir = thirty_lines_repo();
    git_ok(dir.path(), &["checkout", "-q", "-b", "feature/other"]);
    fs::write(dir.path().join("feature.txt"), "from the feature\n").unwrap();
    commit_all(dir.path(), "feature work");
    let sha = stdout_of(dir.path(), &["rev-parse", "--short", "HEAD"]);
    git_ok(dir.path(), &["checkout", "-q", "main"]);

    lg::git::with_repo(dir.path(), || lg::git::cherry_pick_commit(sha.trim())).unwrap();

    assert_eq!(head_branch(dir.path()), "main");
    assert_eq!(
        fs::read_to_string(dir.path().join("feature.txt")).unwrap(),
        "from the feature\n"
    );
    assert_eq!(
        stdout_of(dir.path(), &["log", "-1", "--format=%s"]).trim(),
        "feature work"
    );
    let again = lg::git::with_repo(dir.path(), || lg::git::cherry_pick_commit("HEAD"));
    assert!(again.is_err(), "a commit already here is refused");
    assert!(
        !git(dir.path(), &["rev-parse", "--verify", "CHERRY_PICK_HEAD"])
            .status
            .success(),
        "nothing is left in progress"
    );
}

#[test]
fn a_conflicting_cherry_pick_waits_for_the_conflict_and_can_be_continued() {
    let dir = thirty_lines_repo();
    git_ok(dir.path(), &["checkout", "-q", "-b", "feature/other"]);
    fs::write(dir.path().join("notes.txt"), "feature version\n").unwrap();
    commit_all(dir.path(), "feature rewrite");
    let sha = stdout_of(dir.path(), &["rev-parse", "HEAD"]);
    git_ok(dir.path(), &["checkout", "-q", "main"]);
    fs::write(dir.path().join("notes.txt"), "main version\n").unwrap();
    commit_all(dir.path(), "main rewrite");

    let err = lg::git::with_repo(dir.path(), || lg::git::cherry_pick_commit(sha.trim()))
        .expect_err("both rewrote the file");
    assert!(err.to_string().contains("CONFLICT"), "{err}");
    assert_eq!(
        lg::git::with_repo(dir.path(), lg::git::conflicted_files).unwrap(),
        vec!["notes.txt".to_string()]
    );

    fs::write(dir.path().join("notes.txt"), "both versions\n").unwrap();
    lg::git::with_repo(dir.path(), || {
        lg::git::validate_conflict_resolution(lg::git::Followup::default())
    })
    .unwrap();
    assert_eq!(
        stdout_of(dir.path(), &["log", "-1", "--format=%s"]).trim(),
        "feature rewrite"
    );
}
