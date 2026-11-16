//! Finding the repositories inside a workspace.

use super::common::*;
use std::sync::mpsc;
use std::time::Duration;

fn committed_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git_ok(dir, &["init", "-b", "main"]);
    fs::write(dir.join("readme.txt"), "repo\n").unwrap();
    git_ok(dir, &["add", "readme.txt"]);
    commit_in(dir, "base");
}

/// A symlink back up the tree would otherwise be walked again and again,
/// finding the same repository under ever longer paths.
#[test]
fn a_symlinked_repository_is_found_and_a_symlink_loop_is_not_walked() {
    let workspace = tempfile::tempdir().expect("workspace");
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    committed_repo(&workspace.path().join("app"));
    committed_repo(&elsewhere.path().join("lib"));
    std::os::unix::fs::symlink(
        elsewhere.path().join("lib"),
        workspace.path().join("linked"),
    )
    .unwrap();
    fs::create_dir(workspace.path().join("tools")).unwrap();
    std::os::unix::fs::symlink(workspace.path(), workspace.path().join("tools/up")).unwrap();

    let root = workspace.path().to_path_buf();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(lg::git::nested_repositories_at(&root));
    });
    let repos = receiver
        .recv_timeout(Duration::from_secs(60))
        .expect("the scan finished")
        .expect("the scan succeeded");

    let mut paths: Vec<String> = repos.into_iter().map(|repo| repo.path).collect();
    paths.sort();
    assert_eq!(paths, ["app", "linked"]);
}
