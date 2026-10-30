//! Staging, unstaging and discarding single hunks.

use super::common::*;

/// The hunks the diff pane shows for `path`, read the way lg reads them.
fn pane_hunks(dir: &Path, path: &str) -> Vec<lg::git::hunk::ShownHunk> {
    let text = lg::git::with_repo(dir, || lg::git::file_diff(path)).expect("file diff");
    lg::git::hunk::shown_hunks(&text, true)
}

fn side_hunks(
    hunks: &[lg::git::hunk::ShownHunk],
    side: lg::git::hunk::HunkSide,
) -> Vec<lg::git::hunk::ShownHunk> {
    hunks.iter().filter(|h| h.side == side).cloned().collect()
}

#[test]
fn staging_one_of_two_hunks_leaves_the_other_unstaged() {
    use lg::git::hunk::{HunkOp, HunkSide, apply_hunk};
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());

    let worktree = side_hunks(&pane_hunks(dir.path(), "notes.txt"), HunkSide::Worktree);
    assert_eq!(worktree.len(), 2, "{worktree:?}");
    lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Stage, &worktree[1])).unwrap();

    let staged = stdout_of(dir.path(), &["diff", "--cached"]);
    let unstaged = stdout_of(dir.path(), &["diff"]);
    assert!(staged.contains("+line twenty-eight"), "{staged}");
    assert!(!staged.contains("+line two\n"), "{staged}");
    assert!(unstaged.contains("+line two\n"), "{unstaged}");
    assert!(!unstaged.contains("+line twenty-eight"), "{unstaged}");
    // The pane read again shows one hunk on each side.
    let hunks = pane_hunks(dir.path(), "notes.txt");
    assert_eq!(side_hunks(&hunks, HunkSide::Staged).len(), 1);
    assert_eq!(side_hunks(&hunks, HunkSide::Worktree).len(), 1);
}

#[test]
fn unstaging_a_staged_hunk_puts_it_back_in_the_worktree_only() {
    use lg::git::hunk::{HunkOp, HunkSide, apply_hunk};
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());
    stage_in(dir.path(), "notes.txt");

    let staged = side_hunks(&pane_hunks(dir.path(), "notes.txt"), HunkSide::Staged);
    assert_eq!(staged.len(), 2, "{staged:?}");
    lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Unstage, &staged[0])).unwrap();

    let cached = stdout_of(dir.path(), &["diff", "--cached"]);
    let unstaged = stdout_of(dir.path(), &["diff"]);
    assert!(!cached.contains("+line two\n"), "{cached}");
    assert!(cached.contains("+line twenty-eight"), "{cached}");
    assert!(unstaged.contains("+line two\n"), "{unstaged}");
    let file = fs::read_to_string(dir.path().join("notes.txt")).unwrap();
    assert!(
        file.contains("line two\n"),
        "unstaging leaves the file alone"
    );
}

#[test]
fn discarding_a_hunk_undoes_only_that_hunk_in_the_file() {
    use lg::git::hunk::{HunkOp, HunkSide, apply_hunk};
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());

    let worktree = side_hunks(&pane_hunks(dir.path(), "notes.txt"), HunkSide::Worktree);
    lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Discard, &worktree[0])).unwrap();

    let file = fs::read_to_string(dir.path().join("notes.txt")).unwrap();
    assert!(file.contains("line 2\n"), "{file}");
    assert!(!file.contains("line two"), "{file}");
    assert!(file.contains("line twenty-eight"), "the other hunk stays");
}

#[test]
fn a_hunk_is_refused_once_its_file_changed_under_it() {
    use lg::git::hunk::{HunkOp, HunkSide, apply_hunk};
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());
    let worktree = side_hunks(&pane_hunks(dir.path(), "notes.txt"), HunkSide::Worktree);

    // Edited again after the pane read it: the shown hunk is no longer there.
    let file = fs::read_to_string(dir.path().join("notes.txt")).unwrap();
    fs::write(
        dir.path().join("notes.txt"),
        file.replace("line two", "line TWO"),
    )
    .unwrap();

    let err = lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Stage, &worktree[0]))
        .expect_err("a stale hunk must not be applied");
    assert!(err.to_string().contains("changed since"), "{err}");
    assert!(
        stdout_of(dir.path(), &["diff", "--cached"]).is_empty(),
        "nothing was staged"
    );
}

#[test]
fn staging_the_hunk_of_an_untracked_file_adds_the_file() {
    use lg::git::hunk::{HunkOp, HunkSide, apply_hunk};
    let dir = thirty_lines_repo();
    fs::write(dir.path().join("fresh.txt"), "new\nfile\n").unwrap();

    let worktree = side_hunks(&pane_hunks(dir.path(), "fresh.txt"), HunkSide::Worktree);
    assert_eq!(worktree.len(), 1, "{worktree:?}");
    let discard = lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Discard, &worktree[0]));
    assert!(
        discard.is_err(),
        "an untracked file is deleted, not discarded"
    );
    assert!(dir.path().join("fresh.txt").exists());

    lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Stage, &worktree[0])).unwrap();
    let staged = stdout_of(dir.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), "fresh.txt");
}
