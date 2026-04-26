use std::{fs, process::Command};
use tempfile::TempDir;

/// Run a git command inside `dir`, with author/committer env vars set.
fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("failed to run git")
}

/// Same but panics if exit code != 0.
fn git_ok(dir: &std::path::Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git_ok(dir.path(), &["init", "-b", "main"]);
    git_ok(dir.path(), &["config", "user.email", "test@example.com"]);
    git_ok(dir.path(), &["config", "user.name", "Test User"]);
    dir
}

/// Call `lg::git::status_porcelain` inside a specific working directory.
fn status_in(dir: &std::path::Path) -> (Vec<String>, Vec<String>) {
    // We need to run in that directory, so we temporarily change the process
    // working directory — but that's not thread-safe.  Instead we shell out
    // to git directly and feed the bytes to the parser.
    let out = Command::new("git")
        .args(["status", "-z", "--porcelain=v1"])
        .current_dir(dir)
        .output()
        .expect("git status");
    lg::git::parse_porcelain(&out.stdout)
}

fn stage_in(dir: &std::path::Path, path: &str) {
    git_ok(dir, &["add", "--", path]);
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

fn commit_in(dir: &std::path::Path, msg: &str) {
    let out = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── Tests ──────────────────────────────────────────────────────────────────

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
fn head_branch_returns_current_branch() {
    let dir = init_repo();
    // Need at least one commit for HEAD to resolve.
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    assert_eq!(branch, "main");
}

#[test]
fn commit_on_empty_message_fails() {
    // lg::git::commit guards against empty messages.
    let result = lg::git::commit("");
    assert!(result.is_err(), "expected Err for empty message");
}

// ── parse_porcelain unit tests (comprehensive) ─────────────────────────────

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
