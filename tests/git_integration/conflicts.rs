//! Files left conflicted by a merge, whatever they are called.

use super::common::*;

#[test]
fn conflicted_files_with_spaces_and_non_ascii_names_are_named_as_on_disk() {
    let dir = init_repo();
    let names = ["my file.txt", "Ørsta.txt"];
    for name in names {
        fs::write(dir.path().join(name), "base\n").unwrap();
    }
    commit_all(dir.path(), "base");
    git_ok(dir.path(), &["checkout", "-b", "other"]);
    for name in names {
        fs::write(dir.path().join(name), "theirs\n").unwrap();
    }
    commit_all(dir.path(), "theirs");
    git_ok(dir.path(), &["checkout", "main"]);
    for name in names {
        fs::write(dir.path().join(name), "ours\n").unwrap();
    }
    commit_all(dir.path(), "ours");
    assert!(!git(dir.path(), &["merge", "other"]).status.success());

    let _cwd = CwdGuard::new(dir.path());
    let mut conflicted = lg::git::conflicted_files().expect("conflicted files");
    conflicted.sort();

    let mut expected: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    expected.sort();
    assert_eq!(conflicted, expected);
}
