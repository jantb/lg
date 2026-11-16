//! How untracked files show in the diff pane, and staging them from there.

use super::common::*;
use lg::git::hunk::{HunkOp, HunkSide, apply_hunk, shown_hunks};

fn worktree_hunks(text: &str) -> Vec<lg::git::hunk::ShownHunk> {
    shown_hunks(text, true)
        .into_iter()
        .filter(|hunk| hunk.side == HunkSide::Worktree)
        .collect()
}

/// A directory whose name ends in `b`, and a line that mentions the file the
/// way a patch header would: neither may change how the file is shown.
#[test]
fn an_untracked_file_under_lib_shows_and_stages_under_its_own_path() {
    let dir = thirty_lines_repo();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("lib/bar.txt"), "see b/bar.txt\n").unwrap();

    let text = lg::git::with_repo(dir.path(), || lg::git::file_diff("lib/bar.txt")).unwrap();
    assert!(text.contains("+see b/bar.txt\n"), "{text}");
    let hunks = worktree_hunks(&text);
    assert!(
        hunks.iter().all(|hunk| hunk.path == "lib/bar.txt"),
        "{hunks:?}"
    );

    lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Stage, &hunks[0])).unwrap();
    let staged = stdout_of(dir.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), "lib/bar.txt");
}

#[test]
fn the_review_names_an_untracked_file_under_lib_by_its_own_path() {
    let dir = thirty_lines_repo();
    git_ok(dir.path(), &["checkout", "-b", "feature/lib"]);
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("lib/bar.txt"), "see b/bar.txt\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let diff = lg::git::branch_diff_against_main().unwrap().diff;

    assert!(
        diff.contains("diff --git a/lib/bar.txt b/lib/bar.txt\n"),
        "{diff}"
    );
    assert!(diff.contains("+see b/bar.txt\n"), "{diff}");
}

/// A build or an agent dropping a directory of new files must not make every
/// refresh read all of them; the tracked change stays in view, and each new
/// file still shows in full when it is the one selected.
#[test]
fn a_directory_of_many_untracked_files_is_cut_off_but_each_still_shows_alone() {
    let dir = thirty_lines_repo();
    edit_two_places(dir.path());
    fs::create_dir(dir.path().join("generated")).unwrap();
    let body = "x".repeat(4 * 1024) + "\n";
    for n in 0..400 {
        fs::write(dir.path().join(format!("generated/file-{n:03}.txt")), &body).unwrap();
    }
    let untracked_bytes = 400 * body.len();

    let all = lg::git::with_repo(dir.path(), lg::git::all_diffs).unwrap();
    let folder = lg::git::with_repo(dir.path(), || lg::git::folder_diff("generated")).unwrap();

    assert!(all.contains("+line two\n"), "the tracked change is shown");
    assert!(all.len() < untracked_bytes / 2, "{} bytes", all.len());
    assert!(folder.len() < untracked_bytes / 2, "{} bytes", folder.len());

    let last = "generated/file-399.txt";
    let text = lg::git::with_repo(dir.path(), || lg::git::file_diff(last)).unwrap();
    let hunks = worktree_hunks(&text);
    assert!(hunks.iter().any(|hunk| hunk.path == last), "{text}");
    lg::git::with_repo(dir.path(), || apply_hunk(HunkOp::Stage, &hunks[0])).unwrap();
    let staged = stdout_of(dir.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), last);
}
