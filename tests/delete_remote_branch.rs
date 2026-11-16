//! Deleting a remote branch deletes it on the remote it is named for, and
//! never a protected branch. The only test in its binary, because it points
//! the process's configuration at a temporary HOME.
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

fn git_ok(dir: &Path, args: &[&str]) {
    let output = git(dir, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn has_branch(bare: &Path, branch: &str) -> bool {
    git(bare, &["rev-parse", "--verify", "--quiet", branch])
        .status
        .success()
}

#[test]
fn deletes_the_branch_on_its_own_remote_and_never_a_protected_one() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("project");
    let origin = temp.path().join("origin.git");
    let fork = temp.path().join("fork.git");
    for dir in [&home, &repo, &origin, &fork] {
        std::fs::create_dir_all(dir).unwrap();
    }
    git_ok(&origin, &["init", "--bare", "-b", "main"]);
    git_ok(&fork, &["init", "--bare", "-b", "main"]);
    git_ok(&repo, &["init", "-b", "main"]);
    git_ok(&repo, &["config", "user.name", "Test"]);
    git_ok(&repo, &["config", "user.email", "test@example.com"]);
    git_ok(&repo, &["commit", "--allow-empty", "-m", "Initial"]);
    git_ok(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git_ok(&repo, &["remote", "add", "fork", fork.to_str().unwrap()]);
    git_ok(&repo, &["push", "origin", "main", "main:feature"]);
    git_ok(&repo, &["push", "fork", "main", "main:feature"]);
    // SAFETY: this is the only test in the binary, and nothing else runs yet.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("LG_PREFERENCES_DIR", home.join("config"));
        std::env::set_var("LG_SETTINGS_DIR", home.join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", home.join("model"));
    }
    lg::git::set_active_repo(&repo);

    lg::git::delete_remote_branch("fork", "feature").expect("delete fork/feature");
    assert!(!has_branch(&fork, "feature"), "the fork's branch is gone");
    assert!(
        has_branch(&origin, "feature"),
        "the namesake on origin is left alone"
    );

    assert!(lg::git::delete_remote_branch("fork", "main").is_err());
    assert!(has_branch(&fork, "main"), "a protected branch is kept");
}
