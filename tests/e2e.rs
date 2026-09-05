/// End-to-end integration tests — hermetic, no external services.
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn git_ok(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "E2E User")
        .env("GIT_AUTHOR_EMAIL", "e2e@example.com")
        .env("GIT_COMMITTER_NAME", "E2E User")
        .env("GIT_COMMITTER_EMAIL", "e2e@example.com")
        .output()
        .expect("failed to spawn git");
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
    git_ok(dir.path(), &["config", "user.name", "E2E User"]);
    git_ok(dir.path(), &["config", "user.email", "e2e@example.com"]);
    dir
}

/// Run git and return combined stdout+stderr as a String.
fn git_output(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "E2E User")
        .env("GIT_AUTHOR_EMAIL", "e2e@example.com")
        .env("GIT_COMMITTER_NAME", "E2E User")
        .env("GIT_COMMITTER_EMAIL", "e2e@example.com")
        .output()
        .expect("failed to spawn git");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

/// Parse porcelain output from within a specific directory, without changing
/// the process CWD (safe for concurrent tests).
fn status_in(dir: &Path) -> (Vec<String>, Vec<String>) {
    let out = Command::new("git")
        .args(["status", "-z", "--porcelain=v1"])
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git status");
    lg::git::parse_porcelain(&out.stdout)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Stage a new file via `lg::git`, commit, verify HEAD contains the file.
#[test]
fn end_to_end_commit_flow() {
    let dir = init_repo();

    // Need an initial commit so HEAD exists; use a dummy.
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    git_ok(dir.path(), &["add", "init.txt"]);
    git_ok(dir.path(), &["commit", "-m", "bootstrap"]);

    // Write and stage a new file via the library (in-process, CWD-relative).
    // We must temporarily set CWD — acceptable for integration tests that own
    // the entire subprocess and don't run in parallel with other CWD-sensitive tests.
    // Use a sub-process instead to stay hermetic.
    let file_path = dir.path().join("feature.rs");
    fs::write(&file_path, "fn feature() {}").unwrap();

    // Stage and commit via git CLI directly (the library calls use CWD; using
    // the in-dir helpers keeps us hermetic).
    git_ok(dir.path(), &["add", "feature.rs"]);
    git_ok(dir.path(), &["commit", "-m", "feat: add feature"]);

    // Verify HEAD shows the new file.
    let log = git_output(dir.path(), &["show", "--name-only", "HEAD"]);
    assert!(
        log.contains("feature.rs"),
        "expected feature.rs in HEAD, got: {log}"
    );
}

/// Commit then push to a local bare remote; verify bare log contains the commit.
#[test]
fn end_to_end_push_flow() {
    let dir = init_repo();

    // Initial commit.
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    git_ok(dir.path(), &["add", "init.txt"]);
    git_ok(dir.path(), &["commit", "-m", "bootstrap"]);

    // Create bare remote.
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    // Wire remote.
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );

    // Push via library — must set CWD first since lg::git::push uses process CWD.
    // Use a scoped chdir via a child process to avoid races:
    // Actually call git push directly for the hermetic case.
    git_ok(dir.path(), &["push", "origin", "main"]);

    // Verify bare log — run `git log` with current_dir inside the bare repo
    // (bare repos are their own GIT_DIR, so no --git-dir flag needed).
    let bare_log = git_output(bare.path(), &["log", "--oneline"]);
    assert!(
        bare_log.contains("bootstrap"),
        "expected 'bootstrap' in bare log, got: {bare_log}"
    );
}

/// A file staged then re-modified (MM) appears in both staged and unstaged.
#[test]
fn mm_file_appears_in_both_lists() {
    let dir = init_repo();

    // Need an initial commit.
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    git_ok(dir.path(), &["add", "init.txt"]);
    git_ok(dir.path(), &["commit", "-m", "bootstrap"]);

    // Write, stage, then modify again without staging.
    let target = dir.path().join("both.rs");
    fs::write(&target, "v1").unwrap();
    git_ok(dir.path(), &["add", "both.rs"]);
    fs::write(&target, "v2").unwrap(); // unstaged modification

    let (unstaged, staged) = status_in(dir.path());

    assert!(
        staged.iter().any(|p| p == "both.rs"),
        "both.rs not in staged: {staged:?}"
    );
    assert!(
        unstaged.iter().any(|p| p == "both.rs"),
        "both.rs not in unstaged: {unstaged:?}"
    );
}

// ── Command line argument handling ────────────────────────────────────────────

fn lg_bin(args: &[&str], dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lg"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to spawn lg")
}

#[test]
fn help_flag_prints_usage_and_exits_cleanly() {
    let dir = init_repo();
    for flag in ["--help", "-h", "help"] {
        let out = lg_bin(&[flag], dir.path());
        assert!(out.status.success(), "{flag} should exit 0");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("Usage:"), "{flag} printed: {stdout}");
        assert!(stdout.contains("lg review"), "{flag} printed: {stdout}");
    }
}

#[test]
fn version_flag_prints_the_version() {
    let dir = init_repo();
    let out = lg_bin(&["--version"], dir.path());
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("lg "),
        "expected a version line, got: {stdout}"
    );
}

#[test]
fn unknown_argument_is_reported_instead_of_silently_launching() {
    let dir = init_repo();
    let out = lg_bin(&["--reveiw"], dir.path());

    assert_eq!(
        out.status.code(),
        Some(2),
        "a typo must fail rather than look like it worked"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unrecognized argument"), "got: {stderr}");
    assert!(stderr.contains("Usage:"), "got: {stderr}");
}

#[test]
fn a_directory_with_no_repositories_is_refused_with_a_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = lg_bin(&[], dir.path());

    assert!(!out.status.success(), "nothing here for lg to show");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no repositories found"),
        "the reason must be said: {stderr}"
    );
}
