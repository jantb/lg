//! What the git integration tests share: throwaway repositories, git run
//! inside them, and the lock that guards the process working directory.

pub use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Mutex, MutexGuard},
};
pub use tempfile::TempDir;

pub static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Run a git command inside `dir`, with author/committer env vars set.
pub fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
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
pub fn git_ok(dir: &std::path::Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git_ok(dir.path(), &["init", "-b", "main"]);
    git_ok(dir.path(), &["config", "user.email", "test@example.com"]);
    git_ok(dir.path(), &["config", "user.name", "Test User"]);
    dir
}

/// Call `lg::git::status_porcelain` inside a specific working directory.
pub fn status_in(dir: &std::path::Path) -> (Vec<String>, Vec<String>) {
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

pub fn stage_in(dir: &std::path::Path, path: &str) {
    git_ok(dir, &["add", "--", path]);
}

pub fn commit_in(dir: &std::path::Path, msg: &str) {
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

pub fn commit_in_at(dir: &std::path::Path, msg: &str, date: &str) {
    let out = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub struct CwdGuard {
    old: std::path::PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl CwdGuard {
    pub fn new(dir: &Path) -> Self {
        let lock = CWD_LOCK.lock().expect("cwd lock poisoned");
        let old = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir).expect("set current dir");
        Self { old, _lock: lock }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.old).expect("restore current dir");
    }
}

pub fn head_branch(dir: &Path) -> String {
    let out = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert!(
        out.status.success(),
        "git rev-parse failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

pub fn stash_list(dir: &Path) -> String {
    let out = git(dir, &["stash", "list"]);
    assert!(
        out.status.success(),
        "git stash list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn branch_list(dir: &Path) -> String {
    let out = git(dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        out.status.success(),
        "git branch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A committed file of thirty numbered lines, so two edits far apart make two
/// hunks.
pub fn thirty_lines_repo() -> TempDir {
    let dir = init_repo();
    let lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
    fs::write(dir.path().join("notes.txt"), lines.join("\n") + "\n").unwrap();
    stage_in(dir.path(), "notes.txt");
    commit_in(dir.path(), "base");
    dir
}

/// Change line 2 and line 28 of `notes.txt`: two hunks.
pub fn edit_two_places(dir: &Path) {
    let mut lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
    lines[1] = "line two".into();
    lines[27] = "line twenty-eight".into();
    fs::write(dir.join("notes.txt"), lines.join("\n") + "\n").unwrap();
}

pub fn stdout_of(dir: &Path, args: &[&str]) -> String {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn commit_all(dir: &Path, msg: &str) {
    git_ok(dir, &["add", "-A"]);
    commit_in(dir, msg);
}

/// A repository with `origin`, its main pushed and tracked.
pub fn repo_with_origin() -> (TempDir, TempDir) {
    let dir = thirty_lines_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);
    (dir, bare)
}
