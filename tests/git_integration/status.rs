//! The working tree: status, staging, ignoring, discarding and the file diffs.

use super::common::*;

#[test]
fn status_entries_expands_untracked_directories_to_files() {
    let dir = init_repo();
    git_ok(
        dir.path(),
        &["config", "status.showUntrackedFiles", "normal"],
    );
    fs::create_dir_all(dir.path().join("src/test/service")).unwrap();
    fs::write(
        dir.path()
            .join("src/test/service/ExchangeRateServiceTest.kt"),
        "test\n",
    )
    .unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let entries = lg::git::status_entries().unwrap();
    let paths = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();

    assert!(
        paths.contains(&"src/test/service/ExchangeRateServiceTest.kt"),
        "untracked directory contents should be listed: {paths:?}"
    );
    assert!(
        !paths.contains(&"src/test/service/"),
        "collapsed untracked directory row should be replaced: {paths:?}"
    );
}

/// `status.showUntrackedFiles=no` is a choice not to see untracked files;
/// asking git for all of them overruled it.
#[test]
fn status_entries_leave_untracked_files_out_when_git_is_told_to() {
    let dir = init_repo();
    git_ok(dir.path(), &["config", "status.showUntrackedFiles", "no"]);
    fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
    stage_in(dir.path(), "tracked.txt");
    fs::write(dir.path().join("scratch.txt"), "untracked\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let entries = lg::git::status_entries().unwrap();
    let paths = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();

    assert!(paths.contains(&"tracked.txt"), "{paths:?}");
    assert!(!paths.contains(&"scratch.txt"), "{paths:?}");
}

/// File events for ignored paths are dropped by asking git, in one call for
/// the lot. A tracked file is never ignored, even under an ignore pattern:
/// editing it changes the status.
#[test]
fn ignored_paths_are_recognised_and_tracked_files_never_are() {
    let dir = init_repo();
    fs::write(dir.path().join(".gitignore"), "build/\n*.log\n").unwrap();
    fs::create_dir_all(dir.path().join("build")).unwrap();
    fs::write(dir.path().join("build/kept.txt"), "tracked anyway\n").unwrap();
    git_ok(dir.path(), &["add", "-f", ".gitignore", "build/kept.txt"]);
    commit_in(dir.path(), "initial");

    let _cwd = CwdGuard::new(dir.path());
    let paths = |list: &[&str]| {
        list.iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>()
    };
    assert!(lg::git::all_ignored(&paths(&["build/out.o", "debug.log"])));
    assert!(
        !lg::git::all_ignored(&paths(&["build/out.o", "src/main.rs"])),
        "one file that matters is enough to refresh for"
    );
    assert!(
        !lg::git::all_ignored(&paths(&["build/kept.txt"])),
        "a tracked file is not ignored"
    );
    assert!(
        !lg::git::all_ignored(&[]),
        "nothing to judge is not all ignored"
    );
}

fn unstage_in(dir: &std::path::Path, path: &str) {
    // pre-initial-commit: use rm --cached
    let out = git(dir, &["reset", "-q", "HEAD", "--", path]);
    if !out.status.success() {
        let out2 = git(dir, &["rm", "--cached", "--", path]);
        assert!(
            out2.status.success(),
            "unstage failed: {}",
            String::from_utf8_lossy(&out2.stderr)
        );
    }
}

#[test]
fn add_to_gitignore_appends_file_and_folder_entries_once() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    assert_eq!(
        lg::git::add_to_gitignore("./logs/debug.log", false).unwrap(),
        "ignored logs/debug.log"
    );
    assert_eq!(
        lg::git::add_to_gitignore("tmp/cache", true).unwrap(),
        "ignored tmp/cache/"
    );
    assert_eq!(
        lg::git::add_to_gitignore("tmp/cache/", true).unwrap(),
        "tmp/cache/ already ignored"
    );

    let ignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(ignore, "logs/debug.log\ntmp/cache/\n");
}

#[test]
fn delete_worktree_path_removes_file_or_folder() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::create_dir_all("src/nested").unwrap();
    fs::write("src/main.rs", "fn main() {}\n").unwrap();
    fs::write("src/nested/old.rs", "old\n").unwrap();

    lg::git::delete_worktree_path("src/main.rs", false).unwrap();
    assert!(!dir.path().join("src/main.rs").exists());

    lg::git::delete_worktree_path("src/nested", true).unwrap();
    assert!(!dir.path().join("src/nested").exists());
}

#[test]
fn delete_worktree_path_rejects_unsafe_paths() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    assert!(lg::git::delete_worktree_path("../outside.txt", false).is_err());
    assert!(lg::git::delete_worktree_path("", false).is_err());
}

#[test]
fn rollback_worktree_path_discards_folder_changes() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::create_dir_all("src").unwrap();
    fs::write("src/main.rs", "original\n").unwrap();
    git_ok(dir.path(), &["add", "src/main.rs"]);
    commit_in(dir.path(), "initial");

    fs::write("src/main.rs", "changed\n").unwrap();
    fs::write("src/added.rs", "added\n").unwrap();
    fs::write("src/untracked.rs", "untracked\n").unwrap();
    git_ok(dir.path(), &["add", "src/main.rs", "src/added.rs"]);

    lg::git::rollback_worktree_path("src").unwrap();

    assert_eq!(fs::read_to_string("src/main.rs").unwrap(), "original\n");
    assert!(!dir.path().join("src/added.rs").exists());
    assert!(!dir.path().join("src/untracked.rs").exists());
    let (unstaged, staged) = status_in(dir.path());
    assert!(unstaged.is_empty(), "unstaged: {unstaged:?}");
    assert!(staged.is_empty(), "staged: {staged:?}");
}

#[test]
fn file_diff_includes_untracked_file_contents() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::write("KNOWLEDGE.md", "# Notes\n\nnew detail\n").unwrap();

    let diff = lg::git::file_diff("KNOWLEDGE.md").unwrap();

    assert!(diff.contains("== worktree =="), "{diff}");
    assert!(diff.contains("diff --git"), "{diff}");
    assert!(diff.contains("b/KNOWLEDGE.md"), "{diff}");
    assert!(diff.contains("+# Notes"), "{diff}");
    assert!(diff.contains("+new detail"), "{diff}");
}

#[test]
fn all_diffs_includes_untracked_file_contents() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::create_dir_all("docs").unwrap();
    fs::write("docs/README.md", "# Docs\n").unwrap();

    let diff = lg::git::all_diffs().unwrap();

    assert!(diff.contains("== worktree =="), "{diff}");
    assert!(diff.contains("b/docs/README.md"), "{diff}");
    assert!(diff.contains("+# Docs"), "{diff}");
}

#[test]
fn status_porcelain_on_fresh_repo_is_empty() {
    let dir = init_repo();
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.is_empty(),
        "expected no unstaged, got {unstaged:?}"
    );
    assert!(staged.is_empty(), "expected no staged, got {staged:?}");
}

#[test]
fn stage_then_unstage_round_trips() {
    let dir = init_repo();
    let file = dir.path().join("hello.txt");
    fs::write(&file, "hello").unwrap();

    // After writing: file is untracked (unstaged only).
    let (u, s) = status_in(dir.path());
    assert!(u.contains(&"hello.txt".to_string()), "should be untracked");
    assert!(!s.contains(&"hello.txt".to_string()));

    // Stage it.
    stage_in(dir.path(), "hello.txt");
    let (u, s) = status_in(dir.path());
    assert!(s.contains(&"hello.txt".to_string()), "should be staged");
    assert!(!u.contains(&"hello.txt".to_string()));

    // Unstage it.
    unstage_in(dir.path(), "hello.txt");
    let (u, s) = status_in(dir.path());
    assert!(
        u.contains(&"hello.txt".to_string()),
        "should be back in unstaged"
    );
    assert!(!s.contains(&"hello.txt".to_string()));
}

#[test]
fn parse_porcelain_modified_untracked_renamed_and_both() {
    // Build a synthetic -z byte string:
    //  " M modified.rs"  — worktree-only modified (unstaged)
    //  "?? untracked.txt" — untracked (unstaged)
    //  "R  renamed_new.rs" + "renamed_old.rs"  — staged rename
    //  "MM both.rs"       — staged AND unstaged modified
    let input: Vec<u8> = {
        let mut v = Vec::new();
        v.extend_from_slice(b" M modified.rs");
        v.push(0);
        v.extend_from_slice(b"?? untracked.txt");
        v.push(0);
        v.extend_from_slice(b"R  renamed_new.rs");
        v.push(0);
        v.extend_from_slice(b"renamed_old.rs");
        v.push(0);
        v.extend_from_slice(b"MM both.rs");
        v.push(0);
        v
    };

    let (unstaged, staged) = lg::git::parse_porcelain(&input);

    // Staged: renamed_new.rs (R) + both.rs (MM index side)
    assert!(
        staged.contains(&"renamed_new.rs".to_string()),
        "staged: {staged:?}"
    );
    assert!(
        staged.contains(&"both.rs".to_string()),
        "staged: {staged:?}"
    );
    assert!(
        !staged.contains(&"modified.rs".to_string()),
        "modified.rs should not be staged"
    );
    assert!(!staged.contains(&"untracked.txt".to_string()));

    // Unstaged: modified.rs + untracked.txt + both.rs (MM worktree side)
    assert!(
        unstaged.contains(&"modified.rs".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(
        unstaged.contains(&"untracked.txt".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(
        unstaged.contains(&"both.rs".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(!unstaged.contains(&"renamed_new.rs".to_string()));
}

/// A git process that died mid-write leaves `index.lock` behind, and from then
/// on everything that writes the index fails while `git status` keeps working
/// — so lg shows the files and refuses to stage them. Git's four-line refusal
/// puts the part worth acting on last, which a one-line status bar cuts off.
#[test]
fn staging_against_a_leftover_index_lock_says_what_to_do_about_it() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    fs::write(dir.path().join("work.txt"), "work").unwrap();
    fs::write(dir.path().join(".git/index.lock"), "").unwrap();

    let err = lg::git::with_repo(dir.path(), || lg::git::stage("work.txt"))
        .expect_err("git cannot take the index lock twice");
    let first_line = err
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();

    assert!(
        first_line.contains("index.lock"),
        "the first line should name the lock: {first_line}"
    );
    assert!(
        first_line.contains("delete it if none is"),
        "the first line should say what to do: {first_line}"
    );
}

/// The `apps/api/static/dist/* linguist-generated=true` shape a real
/// `.gitattributes` uses: the bundle still shows as changed, but nothing of
/// what it contains reaches the diff a prompt is built from.
#[test]
fn a_generated_file_shows_as_changed_without_its_contents() {
    let repo = init_repo();
    let dist = repo.path().join("apps/api/static/dist");
    fs::create_dir_all(dist.join("sub")).expect("mkdir");
    fs::write(
        repo.path().join(".gitattributes"),
        "apps/api/static/dist/** linguist-generated=true\n\
         apps/api/static/dist/keep.js linguist-generated=false\n",
    )
    .expect("write .gitattributes");
    fs::write(dist.join("app.js"), "old bundle\n").expect("write");
    fs::write(dist.join("keep.js"), "old keep\n").expect("write");
    fs::write(repo.path().join("real.rs"), "fn main() {}\n").expect("write");
    git_ok(repo.path(), &["add", "-A"]);
    git_ok(repo.path(), &["commit", "-m", "base"]);

    fs::write(dist.join("app.js"), "SECRET_BUNDLE_CONTENTS\n").expect("write");
    fs::write(dist.join("keep.js"), "hand written change\n").expect("write");
    fs::write(repo.path().join("real.rs"), "fn main() { real(); }\n").expect("write");
    git_ok(repo.path(), &["add", "-A"]);

    let diff = lg::git::with_repo(repo.path(), lg::git::all_diffs).expect("all_diffs");

    assert!(
        diff.contains("apps/api/static/dist/app.js"),
        "the generated file should still show as changed: {diff}"
    );
    assert!(
        !diff.contains("SECRET_BUNDLE_CONTENTS"),
        "generated contents must not reach the diff: {diff}"
    );
    assert!(
        diff.contains("hand written change"),
        "an explicit linguist-generated=false carves the file back out: {diff}"
    );
    assert!(
        diff.contains("real()"),
        "ordinary files are untouched: {diff}"
    );
}

/// Git enforces `-diff` itself, so the check is that lg reports the file rather
/// than dropping it once its patch is a binary stand-in.
#[test]
fn a_minus_diff_file_shows_as_changed_without_its_contents() {
    let repo = init_repo();
    fs::write(repo.path().join(".gitattributes"), "secrets.env -diff\n").expect("write");
    fs::write(repo.path().join("secrets.env"), "TOKEN=old\n").expect("write");
    git_ok(repo.path(), &["add", "-A"]);
    git_ok(repo.path(), &["commit", "-m", "base"]);

    fs::write(repo.path().join("secrets.env"), "TOKEN=hunter2\n").expect("write");
    git_ok(repo.path(), &["add", "-A"]);

    let diff = lg::git::with_repo(repo.path(), lg::git::all_diffs).expect("all_diffs");

    assert!(diff.contains("secrets.env"), "file should show: {diff}");
    assert!(!diff.contains("hunter2"), "contents must not reach: {diff}");
}
