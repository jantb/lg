//! A running lg keeps what it read of its preferences between frames. This
//! checks that a change saved elsewhere still reaches it. The only test in its
//! binary, because it points the process's configuration at a temporary HOME.
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Save a preference the way someone at another terminal would.
fn lg_config_set(home: &Path, root: &Path, key: &str, value: &str, scope: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_lg"))
        .args(["config", "set", key, value, "--scope", scope])
        .current_dir(root)
        .env("HOME", home)
        .env("LG_PREFERENCES_DIR", home.join("config"))
        .env("LG_SETTINGS_DIR", home.join("legacy"))
        .env("LG_CONFIG_FILE", home.join("model"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Whether reading the preferences over and over, as drawing does, comes to
/// show `animations` within a few seconds.
fn comes_to_show(animations: bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if lg::preferences::load().config.tools.decorative_animations == animations {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn preferences_saved_by_another_lg_reach_a_running_one() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["commit", "--allow-empty", "-m", "Initial"]);
    // SAFETY: this is the only test in the binary, and nothing else runs yet.
    unsafe {
        std::env::set_var("LG_PREFERENCES_DIR", home.join("config"));
        std::env::set_var("LG_SETTINGS_DIR", home.join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", home.join("model"));
    }
    lg::git::set_active_repo(&repo);
    assert!(lg::preferences::load().config.tools.decorative_animations);

    lg_config_set(&home, &repo, "tools.decorative_animations", "false", "user");
    assert!(
        comes_to_show(false),
        "a user preference saved elsewhere never reached the running lg"
    );

    lg_config_set(
        &home,
        &repo,
        "tools.decorative_animations",
        "true",
        "repository",
    );
    assert!(
        comes_to_show(true),
        "a repository preference saved elsewhere never reached the running lg"
    );
}
