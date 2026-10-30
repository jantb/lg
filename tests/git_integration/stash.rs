//! Stashes, lg's own among them.

use super::common::*;

#[test]
fn stashing_lists_applies_pops_and_drops_entries() {
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());
    fs::write(dir.path().join("untracked.txt"), "keep me\n").unwrap();

    lg::git::with_repo(dir.path(), || lg::git::stash_push("half done")).unwrap();
    assert!(
        stdout_of(dir.path(), &["status", "--porcelain"]).is_empty(),
        "untracked files go into the stash too"
    );
    let entries = lg::git::with_repo(dir.path(), lg::git::stash_list).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].subject.contains("half done"), "{entries:?}");
    assert!(!entries[0].auto);

    lg::git::with_repo(dir.path(), || lg::git::stash_apply(&entries[0].sha)).unwrap();
    assert!(dir.path().join("untracked.txt").exists());
    assert_eq!(
        lg::git::with_repo(dir.path(), lg::git::stash_list)
            .unwrap()
            .len(),
        1,
        "apply keeps the entry"
    );

    git_ok(dir.path(), &["checkout", "--", "notes.txt"]);
    fs::remove_file(dir.path().join("untracked.txt")).unwrap();
    lg::git::with_repo(dir.path(), || lg::git::stash_pop(&entries[0].sha)).unwrap();
    assert!(
        fs::read_to_string(dir.path().join("notes.txt"))
            .unwrap()
            .contains("line two")
    );
    assert!(
        lg::git::with_repo(dir.path(), lg::git::stash_list)
            .unwrap()
            .is_empty(),
        "pop drops the entry"
    );

    lg::git::with_repo(dir.path(), || lg::git::stash_push("")).unwrap();
    let entries = lg::git::with_repo(dir.path(), lg::git::stash_list).unwrap();
    lg::git::with_repo(dir.path(), || lg::git::stash_drop(&entries[0].sha)).unwrap();
    assert!(stash_list(dir.path()).trim().is_empty());
}

#[test]
fn a_stash_lg_left_behind_is_marked_as_lg_s_own() {
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());
    git_ok(
        dir.path(),
        &["stash", "push", "-m", "lg: auto-stash before pull"],
    );
    edit_two_places(dir.path());
    git_ok(dir.path(), &["stash", "push", "-m", "mine"]);

    let entries = lg::git::with_repo(dir.path(), lg::git::stash_list).unwrap();
    let auto: Vec<bool> = entries.iter().map(|entry| entry.auto).collect();
    assert_eq!(auto, vec![false, true], "{entries:?}");
}

#[test]
fn a_stash_that_is_gone_is_not_mistaken_for_another() {
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());
    git_ok(dir.path(), &["stash", "push", "-m", "first"]);
    let gone = lg::git::with_repo(dir.path(), lg::git::stash_list).unwrap()[0]
        .sha
        .clone();
    git_ok(dir.path(), &["stash", "drop"]);
    edit_two_places(dir.path());
    git_ok(dir.path(), &["stash", "push", "-m", "second"]);

    let err = lg::git::with_repo(dir.path(), || lg::git::stash_drop(&gone));
    assert!(err.is_err());
    assert!(stash_list(dir.path()).contains("second"), "kept");
}
