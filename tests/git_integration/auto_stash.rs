//! The stash lg takes on its own before switching branches or pulling, and
//! that it puts back afterwards: only ever its own.

use super::common::*;

/// A repository whose `sub` is a submodule, with a branch `other` to switch
/// to, a stash of the user's own holding an edit to `notes.txt`, and dirty
/// content inside the submodule: a change `git status` reports but that
/// `git stash` has nothing to save for.
fn repo_with_own_stash_and_dirty_submodule(dir: &Path) {
    let library = tempfile::tempdir().expect("library tempdir");
    git_ok(library.path(), &["init", "-b", "main"]);
    fs::write(library.path().join("lib.txt"), "library\n").unwrap();
    git_ok(library.path(), &["add", "lib.txt"]);
    commit_in(library.path(), "library");

    git_ok(
        dir,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            library.path().to_str().unwrap(),
            "sub",
        ],
    );
    commit_in(dir, "add submodule");
    git_ok(dir, &["branch", "other"]);

    fs::write(dir.join("notes.txt"), "the user's own work\n").unwrap();
    git_ok(dir, &["stash", "push", "-m", "my own stash"]);
    fs::write(dir.join("sub/lib.txt"), "edited inside the submodule\n").unwrap();
}

fn assert_own_stash_untouched(dir: &Path) {
    assert!(
        stash_list(dir).contains("my own stash"),
        "the user's stash is still there: {}",
        stash_list(dir)
    );
    assert!(
        !fs::read_to_string(dir.join("notes.txt"))
            .unwrap()
            .contains("the user's own work"),
        "the user's stash was not applied to the checkout"
    );
}

#[test]
fn checkout_leaves_the_users_own_stash_alone_when_nothing_was_stashed() {
    let dir = thirty_lines_repo();
    repo_with_own_stash_and_dirty_submodule(dir.path());

    let _cwd = CwdGuard::new(dir.path());
    let _ = lg::git::checkout_branch("other");

    assert_own_stash_untouched(dir.path());
}

#[test]
fn pull_leaves_the_users_own_stash_alone_when_nothing_was_stashed() {
    let (dir, _bare) = repo_with_origin();
    repo_with_own_stash_and_dirty_submodule(dir.path());
    git_ok(dir.path(), &["push", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    let _ = lg::git::pull();

    assert_own_stash_untouched(dir.path());
}

#[test]
fn nested_checkout_leaves_the_users_own_stash_alone_when_nothing_was_stashed() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let app = workspace.path().join("app");
    fs::create_dir(&app).unwrap();
    git_ok(&app, &["init", "-b", "main"]);
    fs::write(app.join("notes.txt"), "base\n").unwrap();
    git_ok(&app, &["add", "notes.txt"]);
    commit_in(&app, "base");
    repo_with_own_stash_and_dirty_submodule(&app);

    let _ = lg::git::checkout_nested_branch_at(workspace.path(), "app", "other");

    assert_own_stash_untouched(&app);
}

#[test]
fn checkout_still_restores_the_changes_it_stashed() {
    let dir = thirty_lines_repo();
    git_ok(dir.path(), &["branch", "other"]);
    fs::write(dir.path().join("notes.txt"), "uncommitted edit\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    lg::git::checkout_branch("other").expect("checkout other");

    assert_eq!(head_branch(dir.path()), "other");
    assert_eq!(
        fs::read_to_string(dir.path().join("notes.txt")).unwrap(),
        "uncommitted edit\n"
    );
    assert!(stash_list(dir.path()).is_empty());
}
